//! Info-mcast board flood byte. Encoder tests live in `src/protocol/info_mcast.rs`.

#[test]
fn board_flood_offset() {
    assert_eq!(0xBBu8, 187);
    assert_eq!(0x1Fu8, 31);
}
