//! Splitting one transport into a control view and a gameplay view: each view
//! receives only its own channel's messages.

use ferrets_network::demux;
use ferrets_network::message::control::{ControlMessage, InGameMessage};
use ferrets_network::message::gameplay::GameplayMessage;
use ferrets_network::message::{Message, encode};
use ferrets_network::transport::loopback::LoopbackTransport;
use ferrets_network::transport::{NetworkTransport, TransportEvent};

#[test]
fn split_routes_each_message_to_its_own_channel() {
    let (endpoint, mut other) = LoopbackTransport::pair();
    let (mut control, mut gameplay) = demux::split(Box::new(endpoint));

    let frame = encode(&Message::Gameplay(GameplayMessage::Sync {
        tick: 7,
        hash: 9,
    }))
    .unwrap();
    let pause = encode(&Message::Control(ControlMessage::InGame(
        InGameMessage::PauseRequest { paused: true },
    )))
    .unwrap();
    other.broadcast(&frame).expect("send frame");
    other.broadcast(&pause).expect("send pause");

    // Each view sees only its channel's message.
    assert_eq!(messages(&mut gameplay), vec![frame]);
    assert_eq!(messages(&mut control), vec![pause]);
}

//
// ─── Helpers ────────────────────────────────────────────────────────────────
//

/// Polls a transport and returns just the message payloads.
fn messages(transport: &mut Box<dyn NetworkTransport>) -> Vec<Vec<u8>> {
    transport
        .poll()
        .into_iter()
        .filter_map(|event| match event {
            TransportEvent::Message { bytes, .. } => Some(bytes),
            _ => None,
        })
        .collect()
}
