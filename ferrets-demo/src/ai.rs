//! The demo's AI: a simple economy-then-army brain, authored in Lua and
//! installed for every AI slot this node computes.
//!
//! The script is deterministic (integer arithmetic, `ipairs` over the ordered
//! view arrays only), so it is valid under either AI hosting mode.

use std::collections::BTreeMap;

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::{AiRuntimes, install_ai_runtimes, sourced_ai_players};
use ferrets_script::ai::view::content::ContentView;
use ferrets_script::engine::ScriptEngine;
use ferrets_script::engine::lua::LuaEngine;
use ferrets_simulation::content::registry::ContentRegistry;
use ferrets_simulation::session::GameSession;
use ferrets_simulation::session::player_slot::PlayerId;

/// The demo AI, one brain per AI slot. Thinks once a second (20 Hz ticks):
/// keeps a small worker line training and harvesting, puts up one barracks,
/// then trains soldiers and attacks with each full wave.
pub const AI_SCRIPT: &str = r#"
    local RACES = {
        human = { worker = "peasant", hall = "town_hall", barracks = "barracks", soldier = "archer" },
        orc = { worker = "peon", hall = "great_hall", barracks = "orc_barracks", soldier = "grunt" },
    }

    local MAX_WORKERS = 5
    local ARMY_ATTACK_AT = 5
    local MAX_QUEUE = 2

    -- Candidate barracks cells relative to the hall's origin, tried in turn.
    local OFFSETS = { { 0, 4 }, { 4, 4 }, { -4, 0 }, { 0, -4 }, { 4, -4 } }

    local function cost_of(type_name, kind)
        for _, entry in ipairs(content.entities[type_name].cost) do
            if entry.kind == kind then return entry.amount end
        end
        return 0
    end

    local function count_queued(entities, type_name)
        local queued = 0
        for _, e in ipairs(entities) do
            for _, name in ipairs(e.train_queue) do
                if name == type_name then queued = queued + 1 end
            end
        end
        return queued
    end

    -- The accepted candidate nearest to `from` by squared cell distance;
    -- earlier (lower-id) candidates win ties.
    local function nearest(from, candidates, accept)
        local best, best_distance = nil, nil
        for _, e in ipairs(candidates) do
            if accept == nil or accept(e) then
                local dx, dy = e.x - from.x, e.y - from.y
                local distance = dx * dx + dy * dy
                if best == nil or distance < best_distance then
                    best, best_distance = e, distance
                end
            end
        end
        return best
    end

    define_ai("default", {
        period = 20,
        think = function(state, view)
            local names = RACES[view.race]
            if names == nil then return end
            local commands = {}
            local gold = view.resources.gold or 0
            local wood = view.resources.wood or 0

            local halls, workers, soldiers, barracks = {}, {}, {}, {}
            for _, e in ipairs(view.my_entities) do
                if e.type_name == names.hall then halls[#halls + 1] = e
                elseif e.type_name == names.worker then workers[#workers + 1] = e
                elseif e.type_name == names.soldier then soldiers[#soldiers + 1] = e
                elseif e.type_name == names.barracks then barracks[#barracks + 1] = e
                end
            end
            local hall = halls[1]

            -- Keep the worker line going.
            if hall ~= nil and not hall.under_construction
                and #workers + count_queued(halls, names.worker) < MAX_WORKERS
                and #hall.train_queue < MAX_QUEUE
                and gold >= cost_of(names.worker, "gold") then
                commands[#commands + 1] =
                    { kind = "train", trainer = hall.id, type_name = names.worker }
                gold = gold - cost_of(names.worker, "gold")
            end

            -- Put up one barracks. An invalid placement is a silent no-op that
            -- leaves the builder idle and no barracks in the next view, so the
            -- offset ring advances until a candidate fits.
            local build_in_flight = false
            if state.builder_id ~= nil then
                for _, w in ipairs(workers) do
                    if w.id == state.builder_id and not w.idle then
                        build_in_flight = true
                    end
                end
            end
            local builder_id = nil
            if #barracks == 0 and not build_in_flight and hall ~= nil
                and gold >= cost_of(names.barracks, "gold")
                and wood >= cost_of(names.barracks, "wood") then
                local builder = nil
                for _, w in ipairs(workers) do
                    if w.idle and not w.hidden then builder = w break end
                end
                builder = builder or workers[1]
                if builder ~= nil then
                    for _ = 1, #OFFSETS do
                        local offset = OFFSETS[state.build_offset or 1]
                        state.build_offset = (state.build_offset or 1) % #OFFSETS + 1
                        local x = hall.x + offset[1]
                        local y = hall.y + offset[2]
                        if x >= 0 and y >= 0
                            and x + 3 <= view.map.width and y + 3 <= view.map.height then
                            commands[#commands + 1] = {
                                kind = "build", builder = builder.id,
                                type_name = names.barracks, x = x, y = y,
                            }
                            state.builder_id = builder.id
                            builder_id = builder.id
                            break
                        end
                    end
                end
            end

            -- Idle workers gather gold; the first one fetches wood while the
            -- barracks still needs it.
            local need_wood = #barracks == 0 and wood < cost_of(names.barracks, "wood")
            for _, w in ipairs(workers) do
                if w.idle and not w.hidden and w.id ~= builder_id then
                    local target = nil
                    if need_wood then
                        target = nearest(w, view.neutral_entities, function(e)
                            return e.type_name == "tree" and (e.resource_amount or 0) > 0
                        end)
                        need_wood = false
                    end
                    if target == nil then
                        target = nearest(w, view.neutral_entities, function(e)
                            return e.type_name == "gold_mine" and (e.resource_amount or 0) > 0
                        end)
                    end
                    if target ~= nil then
                        commands[#commands + 1] = { kind = "select", id = w.id }
                        commands[#commands + 1] = { kind = "send", target = target.id }
                    end
                end
            end

            -- Train the army once the barracks stands.
            for _, b in ipairs(barracks) do
                if not b.under_construction and #b.train_queue < MAX_QUEUE
                    and gold >= cost_of(names.soldier, "gold") then
                    commands[#commands + 1] =
                        { kind = "train", trainer = b.id, type_name = names.soldier }
                    gold = gold - cost_of(names.soldier, "gold")
                    break
                end
            end

            -- Attack-move every idle soldier onto the nearest enemy once the
            -- wave is big enough — the push engages whatever it meets.
            if #soldiers >= ARMY_ATTACK_AT and #view.enemy_entities > 0 then
                for _, s in ipairs(soldiers) do
                    if s.idle then
                        local target = nearest(s, view.enemy_entities)
                        commands[#commands + 1] = { kind = "select", id = s.id }
                        commands[#commands + 1] =
                            { kind = "attack_move", x = target.x, y = target.y }
                    end
                end
            end

            return commands
        end,
    })
"#;

/// The boss brain, for the environment slot holding the lake. Thinks once a
/// second: keeps the fleet manned from the fortress and shells the nearest
/// enemy within aggro range with every idle ship. Ships never wander — an
/// unreachable or out-of-range target simply leaves them guarding the lake.
pub const BOSS_AI_SCRIPT: &str = r#"
    local AGGRO_SQ = 100
    local MAX_SHIPS = 4
    local MAX_QUEUE = 2

    define_ai("default", {
        period = 20,
        think = function(state, view)
            local commands = {}

            local ships, fortresses = {}, {}
            for _, e in ipairs(view.my_entities) do
                if e.type_name == "ship" then ships[#ships + 1] = e
                elseif e.type_name == "sea_fortress" then fortresses[#fortresses + 1] = e
                end
            end

            -- Keep the fleet manned.
            local queued = 0
            for _, f in ipairs(fortresses) do queued = queued + #f.train_queue end
            for _, f in ipairs(fortresses) do
                if not f.under_construction and #ships + queued < MAX_SHIPS
                    and #f.train_queue < MAX_QUEUE then
                    commands[#commands + 1] =
                        { kind = "train", trainer = f.id, type_name = "ship" }
                    queued = queued + 1
                end
            end

            -- Each idle ship shells the nearest enemy within aggro range.
            for _, s in ipairs(ships) do
                if s.idle then
                    local best, best_distance = nil, nil
                    for _, e in ipairs(view.enemy_entities) do
                        local dx, dy = e.x - s.x, e.y - s.y
                        local distance = dx * dx + dy * dy
                        if distance <= AGGRO_SQ
                            and (best == nil or distance < best_distance) then
                            best, best_distance = e, distance
                        end
                    end
                    if best ~= nil then
                        commands[#commands + 1] = { kind = "select", id = s.id }
                        commands[#commands + 1] = { kind = "attack", target = best.id }
                    end
                end
            end

            return commands
        end,
    })
"#;

/// Builds one demo-AI runtime per AI slot this node sources (which nodes those
/// are follows the session's AI hosting mode) and installs them: the boss brain
/// for environment slots, the economy-army brain for the rest. A script
/// failure degrades to idle AI slots — logged, never a stalled game.
pub fn install_demo_ai(world: &mut World) {
    let ai_players = sourced_ai_players(world);
    if ai_players.is_empty() {
        return;
    }

    let environments: Vec<PlayerId> = {
        let session = world.resource::<GameSession>();
        session.environment_slots().map(|slot| slot.id()).collect()
    };

    let content = ContentView::from_registry(world.resource::<ContentRegistry>());
    let mut runtimes = BTreeMap::new();
    for (player, _) in ai_players {
        let script = if environments.contains(&player) {
            BOSS_AI_SCRIPT
        } else {
            AI_SCRIPT
        };
        match LuaEngine.load_ai(script, &content) {
            Ok(runtime) => {
                runtimes.insert(player, runtime);
            }
            Err(error) => {
                eprintln!("demo ai failed to load: {error}");
                return;
            }
        }
    }
    install_ai_runtimes(world, AiRuntimes(runtimes));
}
