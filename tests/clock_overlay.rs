//! Overlay clock unit tests live in `src/clock/overlay.rs` (crate-private type).

use netaudio::ClockStatus;

mod common;

#[tokio::test]
async fn starts_unlocked_without_ptp() {
    let d = common::start_device("clktest").await;
    assert_eq!(d.clock_status(), ClockStatus::Unlocked);
    d.shutdown().await.unwrap();
}
