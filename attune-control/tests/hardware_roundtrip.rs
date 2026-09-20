//! Hardware round-trip test.
//!
//! Ignored by default: it needs a running daemon and a real GoXLR attached, so
//! CI cannot run it. Run it deliberately:
//!
//! ```text
//! cargo test -p attune-control --test hardware_roundtrip -- --ignored --nocapture
//! ```
//!
//! It restores whatever it changes. If an assertion fails partway the original
//! value may be left changed, which the output says explicitly rather than
//! leaving the operator to guess.

use attune_control::client::DaemonClient;

#[tokio::test]
#[ignore = "requires a running daemon and an attached GoXLR"]
async fn gate_threshold_round_trip_is_verified_on_hardware() {
    let client = DaemonClient::default();

    let serial = client
        .first_serial()
        .await
        .expect("no GoXLR found -- is the daemon running?");

    let original = client
        .mic_chain(&serial)
        .await
        .expect("could not read mic chain")
        .gate_threshold_db;

    println!("device {serial}: gate threshold starts at {original} dB");

    // Move by 1 dB, away from the rail so the test works at either extreme.
    let probe = if original >= 0 {
        original - 1
    } else {
        original + 1
    };

    client
        .set_gate_threshold(&serial, probe)
        .await
        .unwrap_or_else(|e| panic!("write of {probe} dB failed verification: {e}"));

    let observed = client
        .mic_chain(&serial)
        .await
        .expect("could not read back")
        .gate_threshold_db;

    assert_eq!(
        observed, probe,
        "device did not hold the written value (original {original} dB may still be changed)"
    );
    println!("wrote {probe} dB and the device confirmed it");

    client
        .set_gate_threshold(&serial, original)
        .await
        .unwrap_or_else(|e| {
            panic!("RESTORE FAILED -- gate left at {probe} dB, not {original}: {e}")
        });

    let restored = client
        .mic_chain(&serial)
        .await
        .expect("could not read back after restore")
        .gate_threshold_db;

    assert_eq!(
        restored, original,
        "restore did not take -- gate is at {restored} dB, should be {original}"
    );
    println!("restored to {original} dB -- device left as found");
}
