//! Replays a recording headless, as fast as the machine manages, and reports
//! whether it still plays back faithfully.
//!
//! Run with: `cargo run -p ferrets-demo --bin replay -- replays/<stamp>.frep`
//!
//! Exits non-zero when the replayed simulation diverged from the recording (a
//! determinism regression) or when it could not be played to its end.

use std::{fs::File, io::BufReader, process::ExitCode};

use ferrets_demo::playback;
use ferrets_replay::replay::Replay;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: replay <path.frep>");
        return ExitCode::FAILURE;
    };

    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("failed to open {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let replay = match Replay::read(BufReader::new(file)) {
        Ok(replay) => replay,
        Err(error) => {
            eprintln!("failed to read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };

    let recorded = replay.header().engine_version.clone();
    let current = ferrets_simulation::VERSION;
    if recorded != current {
        eprintln!(
            "warning: replay recorded by engine {recorded} but this build is {current}; it may not replay faithfully",
        );
    }

    let mut rebuilt = match playback::rebuild(replay) {
        Ok(rebuilt) => rebuilt,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let Some(last_tick) = rebuilt.last_tick else {
        eprintln!("{path} holds no completed ticks, so there is nothing to verify");
        return ExitCode::FAILURE;
    };

    println!("replaying {path}: {} recorded ticks", last_tick + 1);
    let report = ferrets_bevy_plugin::run_playback(rebuilt.app.world_mut());
    match report.mismatch {
        Some(tick) => {
            println!("diverged at tick {tick} (stopped at tick {})", report.tick);
            ExitCode::FAILURE
        }
        None if report.done => {
            println!("replayed {} ticks, checksums verified", report.tick);
            ExitCode::SUCCESS
        }
        None => {
            println!(
                "stopped at tick {} of {} without reaching the end",
                report.tick, last_tick,
            );
            ExitCode::FAILURE
        }
    }
}
