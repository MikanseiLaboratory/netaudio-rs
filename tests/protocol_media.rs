//! Codec fixtures live next to the unit tests in `src/protocol`.
//! Integration coverage of live ARC/CMC is in `protocol_arc.rs`.

#[test]
fn keepalive_hex() {
    assert_eq!(hex::decode("1337").unwrap(), [0x13, 0x37]);
}

#[test]
fn port_ranges_hex_fixture() {
    let req = include_str!("fixtures/arc/3300-req.hex").trim();
    let resp = include_str!("fixtures/arc/3300-resp.hex").trim();
    assert_eq!(req, "2729000a033c33000000");
    assert_eq!(resp, "27290012033c330000013800397f398039ff");
}
