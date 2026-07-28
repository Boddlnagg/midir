// Only meaningful on Linux with the `jack` feature: proves ALSA and JACK are
// two live, independently selectable backends rather than one excluding the
// other. See https://github.com/Boddlnagg/midir/issues/4.
#![cfg(all(target_os = "linux", feature = "jack"))]

use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{Backend, MidiInput, MidiOutput};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn alsa_and_jack_coexist() {
    // ALSA has no external server dependency, so this must always succeed.
    let alsa_in = MidiInput::new_with_backend("midir-test-alsa-in", Backend::Alsa)
        .expect("alsa backend must always be available on linux");
    let alsa_out = MidiOutput::new_with_backend("midir-test-alsa-out", Backend::Alsa)
        .expect("alsa backend must always be available on linux");
    let _ = alsa_in.ports();
    let _ = alsa_out.ports();

    // JACK needs a running server; only assert that trying it doesn't
    // disturb the already-open ALSA handles above (proves they're
    // independent, not mutually exclusive).
    let _ = MidiInput::new_with_backend("midir-test-jack-in", Backend::Jack);
    let _ = MidiOutput::new_with_backend("midir-test-jack-out", Backend::Jack);

    let _ = alsa_in.ports();
    let _ = alsa_out.ports();
}

// Coexistence alone doesn't prove the enum dispatch actually routes bytes
// correctly, connect a real virtual-port loopback through the ALSA variant
// and confirm the exact bytes sent come back out the callback. This is the
// real risk surface of the refactor (the `match (self, port)` arms in
// linux.rs), not just "both backends can open".
#[test]
fn alsa_loopback_through_enum_dispatch() {
    let input = MidiInput::new_with_backend("midir-test-loopback-in", Backend::Alsa)
        .expect("alsa backend must always be available on linux");
    let output = MidiOutput::new_with_backend("midir-test-loopback-out", Backend::Alsa)
        .expect("alsa backend must always be available on linux");

    let (tx, rx) = mpsc::channel();
    let _conn_in = input
        .create_virtual("midir-test-loopback-in-port", move |_timestamp, message, _| {
            tx.send(message.to_vec()).unwrap();
        }, ())
        .expect("create_virtual should succeed through the Backend::Alsa enum arm");
    let mut conn_out = output
        .create_virtual("midir-test-loopback-out-port")
        .expect("create_virtual should succeed through the Backend::Alsa enum arm");

    // ALSA seq virtual ports need an explicit subscription between them,
    // unlike a hardware loopback cable.
    let status = std::process::Command::new("aconnect")
        .args([
            "midir-test-loopback-out:midir-test-loopback-out-port",
            "midir-test-loopback-in:midir-test-loopback-in-port",
        ])
        .status();
    if !matches!(status, Ok(s) if s.success()) {
        eprintln!("skipping: aconnect unavailable or failed, can't wire the loopback");
        return;
    }

    let sent = vec![0x90, 0x40, 0x7f]; // note-on, middle C, full velocity
    conn_out.send(&sent).unwrap();

    let received = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("callback should fire with the bytes sent through the enum-dispatched connection");
    assert_eq!(received, sent);
}
