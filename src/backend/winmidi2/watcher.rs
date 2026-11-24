//! The MIDI 1.0 port list, kept up to date by a `MidiLegacyPortDeviceWatcher`.
//!
//! A full enumeration is a Plug and Play query every time. The watcher does that query once and
//! then keeps its own list current as ports are added, removed and renamed, so listing ports
//! afterward is cheap. One watcher serves the whole process. If it can't be started, or it stops
//! later, the port list comes from a full enumeration instead.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use log::{error, warn};
use windows_core::{IInspectable, Ref, HSTRING};

use super::bindings::Windows::Devices::Midi2::Enumeration::Legacy::{
    MidiLegacyPortDeviceInformation, MidiLegacyPortDeviceWatcher,
};
use super::bindings::Windows::Devices::Midi2::Enumeration::Midi1PortFlow;
use super::bindings::Windows::Foundation::TypedEventHandler;

/// How long the first call that lists ports waits for the watcher's first enumeration.
const FIRST_ENUMERATION_TIMEOUT: Duration = Duration::from_secs(10);

struct RunningWatcher {
    watcher: MidiLegacyPortDeviceWatcher,
    stopped: Arc<AtomicBool>,
}

enum State {
    NotStarted,
    Running(RunningWatcher),
    /// The watcher couldn't be started. Don't try again; use full enumerations.
    Failed,
}

static STATE: Mutex<State> = Mutex::new(State::NotStarted);

type WatcherEvent = TypedEventHandler<MidiLegacyPortDeviceWatcher, IInspectable>;

fn start() -> Option<RunningWatcher> {
    let watcher = match MidiLegacyPortDeviceWatcher::Create() {
        Ok(Some(watcher)) => watcher,
        Ok(None) => {
            warn!("Could not create a MIDI port watcher");
            return None;
        }
        Err(e) => {
            warn!("Could not create a MIDI port watcher: {e:?}");
            return None;
        }
    };

    let stopped = Arc::new(AtomicBool::new(false));
    let stopped_for_event = stopped.clone();
    let on_stopped = WatcherEvent::new(
        move |_: Ref<'_, MidiLegacyPortDeviceWatcher>, _: Ref<'_, IInspectable>| {
            stopped_for_event.store(true, Ordering::Release);
            Ok(())
        },
    );
    if let Err(e) = watcher.Stopped(&on_stopped) {
        warn!("Could not listen for the MIDI port watcher stopping: {e:?}");
        return None;
    }

    let (completed_sender, completed) = mpsc::sync_channel::<()>(1);
    let on_completed = WatcherEvent::new(
        move |_: Ref<'_, MidiLegacyPortDeviceWatcher>, _: Ref<'_, IInspectable>| {
            let _ = completed_sender.try_send(());
            Ok(())
        },
    );
    let token = match watcher.EnumerationCompleted(&on_completed) {
        Ok(token) => token,
        Err(e) => {
            warn!("Could not listen for the MIDI port watcher's first enumeration: {e:?}");
            return None;
        }
    };

    watcher.Start();
    let finished = completed.recv_timeout(FIRST_ENUMERATION_TIMEOUT).is_ok();
    let _ = watcher.RemoveEnumerationCompleted(token);
    if !finished {
        warn!("The MIDI port watcher did not finish its first enumeration in time");
        watcher.Stop();
        return None;
    }

    Some(RunningWatcher { watcher, stopped })
}

/// The running watcher, started first if needed. `None` means use a full enumeration.
fn current() -> Option<MidiLegacyPortDeviceWatcher> {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    match &*state {
        State::Running(running) if !running.stopped.load(Ordering::Acquire) => {
            return Some(running.watcher.clone());
        }
        State::Failed => return None,
        // Not started yet, or stopped since (for example when the MIDI service restarted).
        _ => {}
    }
    *state = match start() {
        Some(running) => State::Running(running),
        None => State::Failed,
    };
    match &*state {
        State::Running(running) => Some(running.watcher.clone()),
        _ => None,
    }
}

/// Every MIDI 1.0 port in one direction, in no particular order.
pub(super) fn ports(flow: Midi1PortFlow) -> Vec<MidiLegacyPortDeviceInformation> {
    let list = match current() {
        Some(watcher) => Ok(watcher.GetEnumeratedPortsForFlow(flow)),
        None => MidiLegacyPortDeviceInformation::FindAll2(flow),
    };
    match list {
        Ok(Some(list)) => list.into_iter().collect(),
        Ok(None) => {
            error!("Listing MIDI ports failed");
            Vec::new()
        }
        Err(e) => {
            error!("Listing MIDI ports failed: {e:?}");
            Vec::new()
        }
    }
}

/// The port with this device id, or `None` if it no longer exists.
pub(super) fn find_port(
    port_device_id: &HSTRING,
    flow: Midi1PortFlow,
) -> Option<MidiLegacyPortDeviceInformation> {
    match current() {
        Some(watcher) => {
            let wanted = port_device_id.to_string_lossy();
            watcher
                .GetEnumeratedPortsForFlow(flow)?
                .into_iter()
                .find(|port| {
                    port.PortDeviceId()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&wanted)
                })
        }
        None => MidiLegacyPortDeviceInformation::CreateFromPortDeviceId(port_device_id)
            .ok()
            .flatten(),
    }
}
