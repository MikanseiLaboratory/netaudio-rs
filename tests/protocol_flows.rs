//! Provenance: public capture (IMPLEMENTATION-PLAN.md §8.6).

#[test]
fn request_flow_n4_fixture() {
    let golden = include_str!("fixtures/flows/request-n4.hex").trim();
    assert_eq!(golden.len(), 160);
    assert!(golden.starts_with("1102"));
    assert!(golden.contains("0802"), "socket magic 0x0802");
    assert!(!golden.contains("8002") || golden.matches("0802").count() >= 1);
}
