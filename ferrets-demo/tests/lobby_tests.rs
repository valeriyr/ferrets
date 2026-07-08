//! The lobby's dial-address parsing.

use ferrets_demo::lobby::{dial_addr, parse_tcp_port, parse_udp_port};

#[test]
fn dial_addr_appends_default_port_when_omitted() {
    assert_eq!(dial_addr("192.168.1.7"), "192.168.1.7:4000");
    assert_eq!(dial_addr("gaming-laptop.local"), "gaming-laptop.local:4000");
}

#[test]
fn dial_addr_keeps_explicit_port() {
    assert_eq!(dial_addr("192.168.1.7:5123"), "192.168.1.7:5123");
}

#[test]
fn empty_udp_port_means_ephemeral() {
    assert_eq!(parse_udp_port(""), Ok(None));
}

#[test]
fn explicit_udp_port_parses_exactly() {
    assert_eq!(parse_udp_port("4001"), Ok(Some(4001)));
}

#[test]
fn out_of_range_udp_port_is_rejected_by_value() {
    assert_eq!(
        parse_udp_port("70000"),
        Err("invalid udp port '70000'".to_string())
    );
}

#[test]
fn empty_tcp_port_means_default() {
    assert_eq!(parse_tcp_port(""), Ok(4000));
}

#[test]
fn explicit_tcp_port_parses_exactly() {
    assert_eq!(parse_tcp_port("5123"), Ok(5123));
}

#[test]
fn zero_tcp_port_is_rejected_by_value() {
    assert_eq!(parse_tcp_port("0"), Err("invalid tcp port '0'".to_string()));
}
