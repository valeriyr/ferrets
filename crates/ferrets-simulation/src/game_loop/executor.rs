//! Per-tick command dispatch: translates buffered [`InputFrame`]s into order-queue mutations.

use std::collections::HashMap;

use bevy_ecs::prelude::Entity;
use ferrets_math::fixed_uvec2::FixedUVec2;

use crate::{
    command::PlayerCommand,
    components::order_queue::{CancelPolicy, OrderQueueComponent},
    input::InputFrames,
    order::Order,
    selection::Selection,
    simulation_id::SimulationId,
};

/// Processes the frame for `current_tick` if all players have contributed.
///
/// Returns `true` if the frame was ready and processed, `false` if the tick should block.
pub fn tick(
    pending: &mut InputFrames,
    selection: &mut Selection,
    current_tick: u32,
    entities: &HashMap<SimulationId, Entity>,
    positions: &HashMap<SimulationId, FixedUVec2>,
    queues: &mut HashMap<Entity, &mut OrderQueueComponent>,
) -> bool {
    let Some(input) = pending.get_ready(current_tick) else {
        return false;
    };

    for (player, commands) in input.iter() {
        for command in commands {
            match command {
                PlayerCommand::SelectById { id } => {
                    selection.set(player, vec![*id]);
                }
                PlayerCommand::SelectByRect { rect } => {
                    let mut selected: Vec<SimulationId> = positions
                        .iter()
                        .filter(|(_, pos)| rect.contains(**pos))
                        .map(|(&id, _)| id)
                        .collect();
                    selected.sort_unstable();

                    selection.set(player, selected);
                }
                PlayerCommand::Move { target, flush } => {
                    for id in selection.get(player).to_owned() {
                        if let Some(&entity) = entities.get(&id)
                            && let Some(queue) = queues.get_mut(&entity)
                        {
                            queue.push(
                                Order::Move { target: *target },
                                CancelPolicy::from_bool(*flush),
                            );
                        }
                    }
                }
                PlayerCommand::Stop => {
                    for id in selection.get(player).to_owned() {
                        if let Some(&entity) = entities.get(&id)
                            && let Some(queue) = queues.get_mut(&entity)
                        {
                            queue.cancel_all(CancelPolicy::Soft);
                        }
                    }
                }
            }
        }
    }

    true
}
