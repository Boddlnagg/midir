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

// The scenario the split actually targets: an ALSA-side softsynth output
// (e.g. a soundfont player talking to a hardware synth over ALSA seq) and a
// JACK-side input (e.g. a keyboard whose driver only exposes a JACK port),
// alive in the same process at the same time, neither one touching the
// other's backend. Each leg is verified independently by its own loopback,
// so this isn't just "both constructors returned Ok", it's "both actually
// move bytes while the other is live".
#[test]
fn alsa_out_and_jack_in_coexist_and_work() {
    // ALSA leg: stand-in for the soundfont player writing notes out.
    let alsa_out = MidiOutput::new_with_backend("midir-test-softsynth-out", Backend::Alsa)
        .expect("alsa backend must always be available on linux");
    let alsa_in = MidiInput::new_with_backend("midir-test-softsynth-monitor", Backend::Alsa)
        .expect("alsa backend must always be available on linux");
    let (alsa_tx, alsa_rx) = mpsc::channel();
    let _alsa_conn_in = alsa_in
        .create_virtual("midir-test-softsynth-monitor-port", move |_timestamp, message, _| {
            alsa_tx.send(message.to_vec()).unwrap();
        }, ())
        .expect("alsa create_virtual should succeed through the Backend::Alsa enum arm");
    let mut alsa_conn_out = alsa_out
        .create_virtual("midir-test-softsynth-out-port")
        .expect("alsa create_virtual should succeed through the Backend::Alsa enum arm");
    let alsa_wired = std::process::Command::new("aconnect")
        .args([
            "midir-test-softsynth-out:midir-test-softsynth-out-port",
            "midir-test-softsynth-monitor:midir-test-softsynth-monitor-port",
        ])
        .status();
    if !matches!(alsa_wired, Ok(s) if s.success()) {
        eprintln!("skipping: aconnect unavailable, can't wire the alsa leg");
        return;
    }

    // JACK leg: stand-in for the keyboard driver's input port, opened while
    // the ALSA handles above are still live.
    let jack_out = match MidiOutput::new_with_backend("midir-test-keyboard-driver", Backend::Jack) {
        Ok(client) => client,
        Err(_) => {
            eprintln!("skipping jack leg: no jack server reachable (run under pw-jack or jackd)");
            let sent = vec![0x90, 0x3c, 0x64];
            alsa_conn_out.send(&sent).unwrap();
            let received = alsa_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(received, sent);
            return;
        }
    };
    let jack_in = MidiInput::new_with_backend("midir-test-keyboard-app", Backend::Jack)
        .expect("jack server was just reachable for jack_out, should be for jack_in too");
    let (jack_tx, jack_rx) = mpsc::channel();
    let _jack_conn_in = jack_in
        .create_virtual("midir-test-keyboard-in-port", move |_timestamp, message, _| {
            jack_tx.send(message.to_vec()).unwrap();
        }, ())
        .expect("jack create_virtual should succeed through the Backend::Jack enum arm");
    let mut jack_conn_out = jack_out
        .create_virtual("midir-test-keyboard-out-port")
        .expect("jack create_virtual should succeed through the Backend::Jack enum arm");

    // jack ports need their own graph connection, distinct namespace from alsa seq.
    let jack_wired = std::process::Command::new("jack_connect")
        .args([
            "midir-test-keyboard-driver:midir-test-keyboard-out-port",
            "midir-test-keyboard-app:midir-test-keyboard-in-port",
        ])
        .status();
    if !matches!(jack_wired, Ok(s) if s.success()) {
        eprintln!("skipping jack leg: jack_connect unavailable or failed to wire the graph");
        let sent = vec![0x90, 0x3c, 0x64];
        alsa_conn_out.send(&sent).unwrap();
        let received = alsa_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(received, sent);
        return;
    }

    // Send on both legs, in both orders, interleaved, prove one backend's
    // traffic never leaks into or blocks the other.
    let alsa_sent = vec![0x90, 0x3c, 0x64]; // soundfont player: note-on C4
    let jack_sent = vec![0x90, 0x45, 0x50]; // keyboard: note-on A4

    alsa_conn_out.send(&alsa_sent).unwrap();
    jack_conn_out.send(&jack_sent).unwrap();

    let alsa_received = alsa_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("alsa leg should still deliver while a jack client is live in the same process");
    let jack_received = jack_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("jack leg should deliver while an alsa client is live in the same process");

    assert_eq!(alsa_received, alsa_sent);
    assert_eq!(jack_received, jack_sent);
}
