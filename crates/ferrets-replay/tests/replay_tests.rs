//! Writing a replay to a stream and reading it back: round-trips, end-of-stream
//! handling, and the format and version guards.

use ferrets_geometry::projection::Projection;
use ferrets_replay::{
    buffer::SharedBuffer,
    error::ReplayError,
    header::{FORMAT_VERSION, RecordedGame, ReplayHeader},
    record::TickRecord,
    recorder::Recorder,
    replay::Replay,
};
use ferrets_simulation::{
    command::PlayerCommand,
    movement_model::MovementModel,
    session::{
        ai_vision::AiVision,
        elimination_scope::EliminationScope,
        finish_policy::FinishPolicy,
        player_slot::{PlayerId, PlayerSlot},
        player_type::PlayerType,
    },
    skirmish::Skirmish,
};

#[test]
fn round_trips_header_and_records() {
    let buffer = SharedBuffer::default();
    // A skirmish header, so the spelled-out definition — slots, map, finish
    // policy — is proven to survive the round-trip.
    let header = header();
    {
        let mut recorder = Recorder::new(buffer.clone(), &header).expect("start recording");
        recorder.record(&record(0, &[], None)).expect("record 0");
        recorder
            .record(&record(1, &[commands(0)], Some(42)))
            .expect("record 1");
    }

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");

    assert_eq!(replay.header(), &header);
    assert_eq!(replay.last_tick(), Some(1));
    assert_eq!(replay.inputs_at(1), &[commands(0)]);
    assert_eq!(replay.checksum_at(1), Some(42));
}

#[test]
fn round_trips_scenario_header() {
    let buffer = SharedBuffer::default();
    // A scenario game is recorded by name alone; the name is what playback
    // rebuilds the whole game from, so it must survive verbatim.
    let header = ReplayHeader::new(
        RecordedGame::Scenario("build_army".to_string()),
        MovementModel::Continuous,
        Projection::Isometric,
    );
    {
        let mut recorder = Recorder::new(buffer.clone(), &header).expect("start recording");
        recorder.record(&record(0, &[], None)).expect("record 0");
    }

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");

    assert_eq!(replay.header(), &header);
}

#[test]
fn drops_truncated_trailing_record() {
    let buffer = SharedBuffer::default();
    {
        let mut recorder = Recorder::new(buffer.clone(), &header()).expect("start recording");
        recorder
            .record(&record(0, &[commands(0)], None))
            .expect("record 0");
        recorder
            .record(&record(1, &[commands(1)], None))
            .expect("record 1");
    }
    // Simulate a crash mid-write by lopping bytes off the final record.
    let mut bytes = buffer.bytes();
    bytes.truncate(bytes.len() - 3);

    let replay = Replay::read(bytes.as_slice()).expect("read truncated replay");

    // The complete record survives; the half-written one is discarded.
    assert_eq!(replay.last_tick(), Some(0));
    assert_eq!(replay.inputs_at(0), &[commands(0)]);
}

#[test]
fn round_trips_dropped_players() {
    let buffer = SharedBuffer::default();
    {
        let mut recorder = Recorder::new(buffer.clone(), &header()).expect("start recording");
        recorder
            .record(&record(0, &[commands(1)], None))
            .expect("record 0");
        let mut drop_tick = record(1, &[], None);
        drop_tick.dropped = vec![1];
        recorder.record(&drop_tick).expect("record 1");
    }

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");

    // The drop is carried on its tick and absent everywhere else.
    assert_eq!(replay.drops_at(1), &[1]);
    assert!(replay.drops_at(0).is_empty());
    assert!(replay.drops_at(99).is_empty());
}

#[test]
fn inputs_at_is_empty_for_unrecorded_or_idle_ticks() {
    let buffer = SharedBuffer::default();
    {
        let mut recorder = Recorder::new(buffer.clone(), &header()).expect("start recording");
        recorder
            .record(&record(0, &[], None))
            .expect("record idle tick");
    }

    let replay = Replay::read(buffer.bytes().as_slice()).expect("read replay");

    assert!(replay.inputs_at(0).is_empty());
    assert!(replay.inputs_at(99).is_empty());
}

#[test]
fn rejects_stream_without_magic_prelude() {
    let error = Replay::read(b"not a replay at all".as_slice()).expect_err("must reject");

    assert!(matches!(error, ReplayError::BadMagic));
}

#[test]
fn rejects_unsupported_format_version() {
    let future = ReplayHeader {
        format_version: FORMAT_VERSION + 1,
        ..header()
    };
    let bytes = craft_prelude(&future);

    let error = Replay::read(bytes.as_slice()).expect_err("must reject");

    assert!(matches!(
        error,
        ReplayError::UnsupportedVersion { found, expected }
            if found == FORMAT_VERSION + 1 && expected == FORMAT_VERSION
    ));
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// A two-slot header to record against.
fn header() -> ReplayHeader {
    let slots = vec![
        PlayerSlot::occupied(0, PlayerType::Human, Some("human"), None),
        PlayerSlot::occupied(
            1,
            PlayerType::Ai {
                vision: AiVision::Filtered,
            },
            Some("orc"),
            None,
        ),
    ];
    ReplayHeader::new(
        RecordedGame::Skirmish(Skirmish {
            slots,
            map: "demo".to_string(),
            finish_policy: FinishPolicy::LastStanding {
                elimination: EliminationScope::Player,
            },
        }),
        MovementModel::Continuous,
        Projection::Isometric,
    )
}

/// A tick record with the given per-player inputs and optional checksum, and no
/// drops.
fn record(
    tick: u32,
    inputs: &[(PlayerId, Vec<PlayerCommand>)],
    checksum: Option<u64>,
) -> TickRecord {
    TickRecord {
        tick,
        inputs: inputs.to_vec(),
        dropped: Vec::new(),
        checksum,
    }
}

/// A player's input carrying a single command, so it is distinguishable from idle.
fn commands(player: PlayerId) -> (PlayerId, Vec<PlayerCommand>) {
    (player, vec![PlayerCommand::Stop])
}

/// Hand-encodes the magic prelude and header, the way the writer would, so a
/// header this build would never produce (e.g. a future version) can be tested.
fn craft_prelude(header: &ReplayHeader) -> Vec<u8> {
    let body = bcs::to_bytes(header).expect("encode header");
    let mut bytes = b"FREP".to_vec();
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&body);
    bytes
}
