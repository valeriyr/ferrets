//! The tagged wire envelope: round-trips and canonical byte layout.

use ferrets_math::FixedU64;
use ferrets_math::fixed_urect::FixedURect;
use ferrets_math::fixed_uvec2::FixedUVec2;
use ferrets_network::message::control::{
    ControlMessage, LobbyMessage, LobbyState, Occupant, SlotInfo, UdpEntry,
};
use ferrets_network::message::gameplay::GameplayMessage;
use ferrets_network::message::{Message, decode, encode};
use ferrets_network::session_mode::SessionMode;
use ferrets_simulation::command::PlayerCommand;
use ferrets_simulation::input::PlayerFrame;
use ferrets_simulation::session::drop_policy::DropPolicy;
use ferrets_simulation::session::finish_policy::FinishPolicy;
use ferrets_simulation::simulation_id::SimulationId;

//
// ─── Round-trips ──────────────────────────────────────────────────────────────
//

#[test]
fn frames_with_every_command_round_trip() {
    let message = every_command_frames();
    assert_eq!(decode(&encode(&message).unwrap()).unwrap(), message);
}

#[test]
fn sync_round_trips() {
    let sync = Message::Gameplay(GameplayMessage::Sync {
        tick: 64,
        hash: 0xDEAD_BEEF,
    });
    assert_eq!(decode(&encode(&sync).unwrap()).unwrap(), sync);
}

#[test]
fn lobby_state_control_round_trips() {
    let message = Message::Control(ControlMessage::Lobby(LobbyMessage::State(LobbyState {
        slots: vec![
            SlotInfo {
                slot: 0,
                occupant: Occupant::Human { peer: 42 },
                race: Some("human".into()),
            },
            SlotInfo {
                slot: 1,
                occupant: Occupant::Ai,
                race: Some("orc".into()),
            },
            SlotInfo {
                slot: 2,
                occupant: Occupant::Closed,
                race: None,
            },
        ],
        mode: SessionMode::MeshDecentralized,
        drop_policy: DropPolicy::Manual,
        finish_policy: FinishPolicy::Endless,
    })));
    assert_eq!(decode(&encode(&message).unwrap()).unwrap(), message);
}

#[test]
fn start_with_socket_addr_table_round_trips() {
    // SocketAddr (both families) must survive the non-human-readable bcs codec.
    let message = Message::Control(ControlMessage::Lobby(LobbyMessage::Start {
        udp_table: Some(vec![
            UdpEntry {
                peer: 1,
                addr: "127.0.0.1:4000".parse().unwrap(),
            },
            UdpEntry {
                peer: 2,
                addr: "[::1]:4001".parse().unwrap(),
            },
        ]),
        control_table: Some(vec![UdpEntry {
            peer: 2,
            addr: "10.0.0.7:35001".parse().unwrap(),
        }]),
    }));
    assert_eq!(decode(&encode(&message).unwrap()).unwrap(), message);
}

//
// ─── Wire format ────────────────────────────────────────────────────────────
//

#[test]
fn frame_envelope_byte_layout_is_stable() {
    // Locks the canonical wire format against drift: `Message::Gameplay` is variant
    // 0, then `GameplayMessage::Frames` is variant 0, then a ULEB128-length Vec of
    // `PlayerFrame { player: u8, tick: u32 (LE), commands: ULEB128-len }`.
    let message = Message::Gameplay(GameplayMessage::Frames(vec![PlayerFrame {
        player: 7,
        tick: 3,
        commands: vec![],
    }]));
    assert_eq!(
        encode(&message).unwrap(),
        vec![0x00, 0x00, 0x01, 0x07, 0x03, 0x00, 0x00, 0x00, 0x00],
    );
}

//
// ─── Helpers ──────────────────────────────────────────────────────────────────
//

fn pos(x: u32, y: u32) -> FixedUVec2 {
    FixedUVec2::new(FixedU64::from_num(x), FixedU64::from_num(y))
}

/// A one-frame batch holding one of every command variant, to exercise every field type.
fn every_command_frames() -> Message {
    Message::Gameplay(GameplayMessage::Frames(vec![PlayerFrame {
        player: 1,
        tick: 42,
        commands: vec![
            PlayerCommand::SelectById {
                id: SimulationId(7),
            },
            PlayerCommand::SelectByRect {
                rect: FixedURect::from_corners(pos(1, 2), pos(5, 6)),
            },
            PlayerCommand::Move {
                target: pos(10, 12),
                flush: true,
            },
            PlayerCommand::Attack {
                target: SimulationId(9),
                flush: false,
            },
            PlayerCommand::SendToEntity {
                target: SimulationId(3),
                flush: true,
            },
            PlayerCommand::TrainEntity {
                trainer: SimulationId(4),
                type_name: "peasant".into(),
            },
            PlayerCommand::BuildEntity {
                builder: SimulationId(4),
                type_name: "town_hall".into(),
                position: pos(8, 8),
                flush: true,
            },
            PlayerCommand::Stop,
            PlayerCommand::Spawn {
                type_name: "archer".into(),
                position: pos(2, 2),
            },
        ],
    }]))
}
