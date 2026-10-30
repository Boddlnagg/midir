//! Round trip through the MIDI service, using a temporary loopback endpoint pair: what is sent
//! to endpoint A arrives at endpoint B.
//!
//! These tests need Windows 11 25H2 or later with Windows MIDI Services in full mode, and the
//! API in Windows or next to the test executable. Elsewhere they print why and pass, unless
//! `MIDIR_WMS_REQUIRE_SERVICE` is set, which CI does on runners that have the service.

use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows_core::{GUID, HSTRING};

use super::bindings::Windows::Devices::Midi2::Transports::Loopback::{
    MidiLoopbackCreationConfig, MidiLoopbackEndpointDefinition, MidiLoopbackEndpointEntry,
    MidiLoopbackManager, MidiLoopbackRemovalConfig,
};
use super::*;
use crate::{Ignore, MidiInput as PublicMidiInput, MidiOutput as PublicMidiOutput};

fn service_required() -> bool {
    std::env::var_os("MIDIR_WMS_REQUIRE_SERVICE").is_some()
}

/// Tests that use the service take turns, because the loopback test adds and removes ports.
static SERVICE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn service_test_lock() -> std::sync::MutexGuard<'static, ()> {
    SERVICE_TESTS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Returns false, after saying why, when the test can't run on this PC.
fn can_run(what: &str) -> bool {
    if ensure_available().is_ok() {
        return true;
    }
    if service_required() {
        panic!(
            "{what}: Windows MIDI Services is not available, and MIDIR_WMS_REQUIRE_SERVICE is set"
        );
    }
    eprintln!("{what}: skipped, Windows MIDI Services is not available on this PC");
    false
}

/// A loopback pair that is removed when dropped.
struct Loopback {
    association_id: GUID,
    endpoint_a: HSTRING,
    endpoint_b: HSTRING,
}

impl Loopback {
    fn create() -> Result<Loopback, String> {
        if !MidiLoopbackManager::IsTransportAvailable().unwrap_or(false) {
            return Err("the loopback transport is not available".into());
        }
        // Unique ids must not repeat while the service is running.
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let unique = format!("midirtest{}x{}", std::process::id(), nonce % 1_000_000_000);

        let definition = |side: &str| -> windows_core::Result<MidiLoopbackEndpointDefinition> {
            MidiLoopbackEndpointDefinition::CreateInstance3(
                &HSTRING::from(format!("midir test {side}")),
                &HSTRING::from("Created by midir's tests. Removed when they finish."),
                &HSTRING::from(format!("{unique}{side}")),
            )
        };
        let a = definition("A").map_err(|e| format!("endpoint A definition: {e:?}"))?;
        let b = definition("B").map_err(|e| format!("endpoint B definition: {e:?}"))?;
        let config = MidiLoopbackCreationConfig::CreateInstance(&a, &b)
            .map_err(|e| format!("creation config: {e:?}"))?;

        let response = MidiLoopbackManager::CreateTransientLoopback(&config)
            .map_err(|e| format!("CreateTransientLoopback: {e:?}"))?
            .ok_or("CreateTransientLoopback returned nothing")?;
        if !response.Success() {
            let message = response.ErrorMessage().to_string_lossy();
            return Err(format!("CreateTransientLoopback failed: {message}"));
        }
        let entry = response
            .CreatedLoopbackEntry()
            .ok_or("CreateTransientLoopback returned no loopback entry")?;
        let endpoint_id = |e: Option<MidiLoopbackEndpointEntry>, side: &str| {
            e.map(|e| e.EndpointDeviceId())
                .filter(|id| !id.is_empty())
                .ok_or(format!("no device id for endpoint {side}"))
        };
        Ok(Loopback {
            association_id: entry.AssociationId(),
            endpoint_a: endpoint_id(entry.EndpointA(), "A")?,
            endpoint_b: endpoint_id(entry.EndpointB(), "B")?,
        })
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        let removed = MidiLoopbackRemovalConfig::CreateInstance(self.association_id)
            .and_then(|config| MidiLoopbackManager::RemoveTransientLoopback(&config))
            .map(|response| response.map(|r| r.Success()));
        if !matches!(removed, Ok(Some(true))) {
            eprintln!("Removing the test loopback failed: {removed:?}");
        }
    }
}

fn same_endpoint(a: &HSTRING, b: &HSTRING) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

/// MIDI 1.0 ports for a new endpoint appear shortly after the endpoint does.
fn wait_for_group_0_port(endpoint: &HSTRING, flow: Midi1PortFlow) -> Option<HSTRING> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Some(port) = enumerate_ports(flow)
            .into_iter()
            .find(|p| p.group == 0 && same_endpoint(&p.endpoint_device_id, endpoint))
        {
            return Some(port.port_device_id);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

#[test]
fn loopback_round_trip() {
    let _turn = service_test_lock();
    if !can_run("loopback_round_trip") {
        return;
    }
    let loopback = match Loopback::create() {
        Ok(loopback) => loopback,
        Err(e) if !service_required() => {
            eprintln!("loopback_round_trip: skipped, {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    };

    let out_id = wait_for_group_0_port(&loopback.endpoint_a, Midi1PortFlow::MidiMessageDestination)
        .expect("output port for loopback endpoint A");
    let in_id = wait_for_group_0_port(&loopback.endpoint_b, Midi1PortFlow::MidiMessageSource)
        .expect("input port for loopback endpoint B");

    let mut midi_in = PublicMidiInput::new("midir wms test").unwrap();
    midi_in.ignore(Ignore::None);
    let midi_out = PublicMidiOutput::new("midir wms test").unwrap();

    let in_port = midi_in
        .find_port_by_id(&in_id.to_string_lossy())
        .expect("input port is listed");
    let out_port = midi_out
        .find_port_by_id(&out_id.to_string_lossy())
        .expect("output port is listed");
    assert!(!midi_in.port_name(&in_port).unwrap().is_empty());

    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    let conn_in = midi_in
        .connect(
            &in_port,
            "midir-test",
            move |_stamp, message, sender| {
                let _ = sender.send(message.to_vec());
            },
            sender,
        )
        .unwrap();
    let mut conn_out = midi_out.connect(&out_port, "midir-test").unwrap();

    let mut sysex = vec![0xF0, 0x7D];
    sysex.extend((0..200u32).map(|i| (i % 128) as u8));
    sysex.push(0xF7);

    let sent: Vec<Vec<u8>> = vec![
        vec![0x90, 60, 100],
        vec![0x80, 60, 0],
        vec![0xC3, 12],
        vec![0xE0, 0x00, 0x40],
        vec![0xF8],
        sysex,
    ];
    for message in &sent {
        conn_out.send(message).unwrap();
    }

    let mut received = Vec::new();
    while received.len() < sent.len() {
        match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(message) => received.push(message),
            Err(_) => break,
        }
    }
    assert_eq!(received, sent);

    conn_out.close();
    let (_midi_in, _sender) = conn_in.close();
    drop(loopback);
}

/// The watcher's list is the same as a full enumeration.
#[test]
fn watcher_matches_full_enumeration() {
    let _turn = service_test_lock();
    if !can_run("watcher_matches_full_enumeration") {
        return;
    }
    let describe = |ports: Vec<MidiLegacyPortDeviceInformation>| {
        let mut list: Vec<(u32, String, String)> = ports
            .iter()
            .map(|p| {
                (
                    p.Number(),
                    p.PortDeviceId().to_string_lossy().to_lowercase(),
                    p.Name().to_string_lossy(),
                )
            })
            .collect();
        list.sort();
        list
    };
    for flow in [
        Midi1PortFlow::MidiMessageSource,
        Midi1PortFlow::MidiMessageDestination,
    ] {
        // Ports from an earlier test may still be going away, so give the lists time to settle.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let from_watcher = describe(watcher::ports(flow));
            let full = describe(
                MidiLegacyPortDeviceInformation::FindAll2(flow)
                    .unwrap()
                    .expect("a port list")
                    .into_iter()
                    .collect(),
            );
            if from_watcher == full {
                break;
            }
            if Instant::now() > deadline {
                assert_eq!(from_watcher, full);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

#[test]
fn ignore_flags_filter_input() {
    assert!(is_ignored(Ignore::Sysex, &[0xF0, 1, 0xF7]));
    assert!(is_ignored(Ignore::Time, &[0xF8]));
    assert!(is_ignored(Ignore::Time, &[0xF1, 0]));
    assert!(is_ignored(Ignore::ActiveSense, &[0xFE]));
    assert!(!is_ignored(Ignore::All, &[0x90, 60, 1]));
    assert!(!is_ignored(Ignore::None, &[0xF8]));
}

#[test]
fn tick_conversion() {
    let ticks = TickConverter {
        ticks_per_second: 10_000_000,
    };
    assert_eq!(ticks.to_microseconds(10_000_000), 1_000_000);
    assert_eq!(ticks.to_microseconds(15), 1);
    let ticks = TickConverter {
        ticks_per_second: 3_000_000,
    };
    assert_eq!(ticks.to_microseconds(3), 1);
}
