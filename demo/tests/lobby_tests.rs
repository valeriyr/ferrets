//! The lobby's dial-address parsing.

use demo::lobby::dial_addr;

#[test]
fn dial_addr_appends_default_port_when_omitted() {
    assert_eq!(dial_addr("192.168.1.7"), "192.168.1.7:4000");
    assert_eq!(dial_addr("gaming-laptop.local"), "gaming-laptop.local:4000");
}

#[test]
fn dial_addr_keeps_explicit_port() {
    assert_eq!(dial_addr("192.168.1.7:5123"), "192.168.1.7:5123");
}
