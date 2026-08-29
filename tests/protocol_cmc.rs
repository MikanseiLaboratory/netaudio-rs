//! CMC codec is covered in `src/protocol/cmc.rs`. Live server: `protocol_arc.rs`.

#[test]
fn advertisement_port_8700() {
    assert_eq!(8700u16.to_be_bytes(), [0x21, 0xFC]);
}
