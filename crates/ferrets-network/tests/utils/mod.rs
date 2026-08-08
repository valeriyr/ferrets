#![allow(dead_code)]

use std::time::{Duration, Instant};

use ferrets_network::{
    control::ControlChannel,
    lobby::{client::LobbyClient, host::LobbyHost},
    session_mode::SessionMode,
    transport::NetworkTransport,
};
use ferrets_simulation::session::{drop_policy::DropPolicy, finish_policy::FinishPolicy};

/// A lobby host over `transport` in the given `mode`, with the suite's
/// baseline choices for everything else: automatic drops, last-standing
/// finish, `capacity` slots defaulting to the "human" race.
pub fn lobby_host(
    transport: impl NetworkTransport + 'static,
    mode: SessionMode,
    capacity: usize,
) -> LobbyHost {
    LobbyHost::new(
        ControlChannel::new(Box::new(transport)),
        mode,
        DropPolicy::Automatic,
        FinishPolicy::LastStanding,
        capacity,
        "human",
    )
}

/// A lobby client over `transport`.
pub fn lobby_client(transport: impl NetworkTransport + 'static) -> LobbyClient {
    LobbyClient::new(ControlChannel::new(Box::new(transport)))
}

/// Polls `ready` until it yields a value, panicking with `what` after a
/// generous deadline — sockets and background threads need real time.
pub fn wait_for<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(value) = ready() {
            return value;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

/// Polls `ready` until it returns `true`, panicking with `what` after the
/// same deadline as [`wait_for`].
pub fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    wait_for(what, || ready().then_some(()));
}
