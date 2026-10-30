//! Windows MIDI Services backend (`Windows.Devices.Midi2`).
//!
//! Each midir port is one MIDI 1.0 port that Windows MIDI Services publishes for a UMP endpoint:
//! the endpoint, one group, and one direction. These are the same ports, with the same names and
//! in the same order, that WinMM applications see. Messages are converted between MIDI 1.0 bytes
//! and UMP in this backend (see `ump.rs`).
//!
//! The process uses one Windows MIDI Services session, named after the first `client_name` it is
//! given, and one connection per endpoint, shared by every port on that endpoint.

#[allow(
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types,
    dead_code,
    rust_2018_idioms,
    clippy::all
)]
mod bindings;
#[cfg(test)]
mod tests;
mod ump;

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, OnceLock};

use log::{error, info, warn};
use windows_core::{Ref, HSTRING};

use self::bindings::Windows::Devices::Midi2::Enumeration::Legacy::MidiLegacyPortDeviceInformation;
use self::bindings::Windows::Devices::Midi2::Enumeration::Midi1PortFlow;
use self::bindings::Windows::Devices::Midi2::{
    IMidiMessageReceivedEventSource, MidiApi, MidiClock, MidiEndpointConnection,
    MidiMessageReceivedEventArgs, MidiSendMessageResults, MidiSession,
};
use self::bindings::Windows::Foundation::TypedEventHandler;
use self::ump::{BytesToUmp, Ump, UmpToBytes};

use crate::errors::*;
use crate::Ignore;

/// Windows 11 25H2. The API is supported on this build and later only. Earlier builds may have
/// an older MIDI service that the API partly works with, which is worse than not working.
const MINIMUM_WINDOWS_BUILD: u32 = 26200;

// ---------------------------------------------------------------------------------------------
// Startup checks
// ---------------------------------------------------------------------------------------------

fn windows_build_number() -> Option<u32> {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // RtlGetVersion reports the real version. GetVersionEx does not, unless the host
    // application's manifest says it supports Windows 10, and a library can't control that.
    let status = unsafe { RtlGetVersion(&mut info) };
    if status.is_ok() && info.dwMajorVersion == 10 {
        Some(info.dwBuildNumber)
    } else {
        None
    }
}

/// Checks, once per process, that Windows MIDI Services can be used: a supported Windows build,
/// the API present (in Windows, or next to the application), and the MIDI service running in
/// full Windows MIDI Services mode rather than Legacy API mode.
fn ensure_available() -> Result<(), InitError> {
    static AVAILABLE: OnceLock<()> = OnceLock::new();
    if AVAILABLE.get().is_some() {
        return Ok(());
    }

    match windows_build_number() {
        Some(build) if build >= MINIMUM_WINDOWS_BUILD => {}
        Some(build) => {
            warn!(
                "Windows MIDI Services needs Windows 11 25H2 (build {MINIMUM_WINDOWS_BUILD}) or \
                 later. This is build {build}."
            );
            return Err(InitError);
        }
        None => {
            warn!("Could not read the Windows version");
            return Err(InitError);
        }
    }

    // Any call fails if the API is not in Windows and not next to the application.
    match MidiApi::EnsureServiceAvailable() {
        Ok(true) => {}
        Ok(false) => {
            warn!(
                "The Windows MIDI Services API is present, but the MIDI service is not available. \
                 The PC may be set to Legacy API mode."
            );
            return Err(InitError);
        }
        Err(e) => {
            warn!("The Windows MIDI Services API could not be loaded: {e:?}");
            return Err(InitError);
        }
    }

    match MidiApi::IsSystemProvided() {
        Ok(true) => info!("Using the Windows MIDI Services API that is part of Windows"),
        Ok(false) => info!("Using a Windows MIDI Services API shipped with the application"),
        Err(_) => {}
    }

    let _ = AVAILABLE.set(());
    Ok(())
}

/// Converts MIDI clock ticks to microseconds, the unit midir passes to input callbacks.
#[derive(Clone, Copy)]
struct TickConverter {
    ticks_per_second: u64,
}

impl TickConverter {
    fn new() -> Self {
        let ticks_per_second = MidiClock::TimestampFrequency().unwrap_or(10_000_000).max(1);
        TickConverter { ticks_per_second }
    }

    fn to_microseconds(self, ticks: u64) -> u64 {
        ((ticks as u128 * 1_000_000) / self.ticks_per_second as u128) as u64
    }
}

// ---------------------------------------------------------------------------------------------
// Shared session and connections
// ---------------------------------------------------------------------------------------------

struct SharedConnection {
    connection: MidiEndpointConnection,
    users: usize,
}

struct SharedState {
    session: MidiSession,
    connections: HashMap<String, SharedConnection>,
}

static SHARED: Mutex<Option<SharedState>> = Mutex::new(None);

fn connection_key(endpoint_device_id: &HSTRING) -> String {
    endpoint_device_id.to_string_lossy().to_lowercase()
}

/// A counted use of the shared connection to one endpoint. The connection closes when the last
/// lease is dropped.
struct ConnectionLease {
    key: String,
    connection: MidiEndpointConnection,
}

impl ConnectionLease {
    /// Gets or opens the connection to an endpoint. `before_open` runs before a new connection
    /// is opened, and right away for one that is already open, so that an input handler is
    /// in place before the first message can arrive.
    fn acquire<F>(
        client_name: &str,
        endpoint_device_id: &HSTRING,
        before_open: F,
    ) -> Result<(ConnectionLease, i64), &'static str>
    where
        F: FnOnce(&MidiEndpointConnection) -> Result<i64, &'static str>,
    {
        let key = connection_key(endpoint_device_id);
        let mut guard = SHARED.lock().unwrap_or_else(|e| e.into_inner());

        if guard.is_none() {
            let session = MidiSession::Create(&HSTRING::from(client_name))
                .ok()
                .flatten()
                .ok_or("could not create a Windows MIDI Services session")?;
            *guard = Some(SharedState {
                session,
                connections: HashMap::new(),
            });
        }
        let state = guard.as_mut().expect("session was just created");

        if let Some(shared) = state.connections.get_mut(&key) {
            let token = before_open(&shared.connection)?;
            shared.users += 1;
            return Ok((
                ConnectionLease {
                    key,
                    connection: shared.connection.clone(),
                },
                token,
            ));
        }

        let connection = state
            .session
            .CreateEndpointConnection(endpoint_device_id)
            .ok_or("could not create a connection to the endpoint")?;
        let token = match before_open(&connection) {
            Ok(token) => token,
            Err(e) => {
                disconnect(&state.session, &connection);
                return Err(e);
            }
        };
        if !connection.Open() {
            disconnect(&state.session, &connection);
            return Err("could not open the connection to the endpoint");
        }
        state.connections.insert(
            key.clone(),
            SharedConnection {
                connection: connection.clone(),
                users: 1,
            },
        );
        Ok((ConnectionLease { key, connection }, token))
    }
}

fn disconnect(session: &MidiSession, connection: &MidiEndpointConnection) {
    session.DisconnectEndpointConnection(connection.ConnectionId());
}

impl Drop for ConnectionLease {
    fn drop(&mut self) {
        // Disconnect outside the lock, so that nothing waiting on the service holds it.
        let to_disconnect = {
            let mut guard = SHARED.lock().unwrap_or_else(|e| e.into_inner());
            let Some(state) = guard.as_mut() else {
                return;
            };
            let last_user = match state.connections.get_mut(&self.key) {
                Some(shared) => {
                    shared.users -= 1;
                    shared.users == 0
                }
                None => false,
            };
            if last_user {
                state.connections.remove(&self.key);
                Some(state.session.clone())
            } else {
                None
            }
        };
        if let Some(session) = to_disconnect {
            disconnect(&session, &self.connection);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------------

struct PortInfo {
    port_device_id: HSTRING,
    endpoint_device_id: HSTRING,
    group: u8,
}

fn port_info(port: &MidiLegacyPortDeviceInformation) -> Option<(u32, PortInfo)> {
    let number = port.Number();
    let port_device_id = port.PortDeviceId();
    let endpoint_device_id = port.AssociatedEndpointDeviceId();
    let group = port.Group()?.Index();
    if port_device_id.is_empty() || endpoint_device_id.is_empty() || group > 15 {
        return None;
    }
    Some((
        number,
        PortInfo {
            port_device_id,
            endpoint_device_id,
            group,
        },
    ))
}

/// The MIDI 1.0 ports in one direction, in WinMM port number order.
fn enumerate_ports(flow: Midi1PortFlow) -> Vec<PortInfo> {
    let ports = match MidiLegacyPortDeviceInformation::FindAll2(flow) {
        Ok(Some(ports)) => ports,
        Ok(None) => {
            error!("Listing MIDI ports failed");
            return Vec::new();
        }
        Err(e) => {
            error!("Listing MIDI ports failed: {e:?}");
            return Vec::new();
        }
    };
    let mut result: Vec<(u32, PortInfo)> =
        ports.into_iter().filter_map(|p| port_info(&p)).collect();
    result.sort_by_key(|(number, _)| *number);
    result.into_iter().map(|(_, info)| info).collect()
}

fn port_name(port_device_id: &HSTRING) -> Result<String, PortInfoError> {
    let port = MidiLegacyPortDeviceInformation::CreateFromPortDeviceId(port_device_id)
        .ok()
        .flatten()
        .ok_or(PortInfoError::InvalidPort)?;
    let name = port.Name();
    if name.is_empty() {
        return Err(PortInfoError::CannotRetrievePortName);
    }
    Ok(name.to_string_lossy())
}

// ---------------------------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
pub struct MidiInputPort {
    port_device_id: HSTRING,
    endpoint_device_id: HSTRING,
    group: u8,
}

impl MidiInputPort {
    pub fn id(&self) -> String {
        self.port_device_id.to_string_lossy()
    }
}

pub struct MidiInput {
    client_name: String,
    ignore_flags: Ignore,
}

impl MidiInput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        ensure_available()?;
        Ok(MidiInput {
            client_name: client_name.to_string(),
            ignore_flags: Ignore::None,
        })
    }

    pub fn ignore(&mut self, flags: Ignore) {
        self.ignore_flags = flags;
    }

    pub(crate) fn ports_internal(&self) -> Vec<crate::common::MidiInputPort> {
        enumerate_ports(Midi1PortFlow::MidiMessageSource)
            .into_iter()
            .map(|p| crate::common::MidiInputPort {
                imp: MidiInputPort {
                    port_device_id: p.port_device_id,
                    endpoint_device_id: p.endpoint_device_id,
                    group: p.group,
                },
            })
            .collect()
    }

    pub fn port_count(&self) -> usize {
        enumerate_ports(Midi1PortFlow::MidiMessageSource).len()
    }

    pub fn port_name(&self, port: &MidiInputPort) -> Result<String, PortInfoError> {
        port_name(&port.port_device_id)
    }

    pub fn connect<F, T: Send + 'static>(
        self,
        port: &MidiInputPort,
        _port_name: &str,
        callback: F,
        data: T,
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        let handler_data = Arc::new(Mutex::new(HandlerData {
            ignore_flags: self.ignore_flags,
            decoder: UmpToBytes::new(port.group),
            ticks: TickConverter::new(),
            callback: Box::new(callback),
            user_data: Some(data),
        }));

        let handler_data_for_event = handler_data.clone();
        let handler = TypedEventHandler::<
            IMidiMessageReceivedEventSource,
            MidiMessageReceivedEventArgs,
        >::new(
            move |_sender, args: Ref<'_, MidiMessageReceivedEventArgs>| {
                if let Some(args) = args.as_ref() {
                    // A panic must not unwind into the service's thread.
                    let handled = catch_unwind(AssertUnwindSafe(|| {
                        handle_input(args, &handler_data_for_event)
                    }));
                    if handled.is_err() {
                        error!("Handling a received MIDI message panicked");
                    }
                }
                Ok(())
            },
        );

        let acquired = ConnectionLease::acquire(&self.client_name, &port.endpoint_device_id, |c| {
            c.MessageReceived(&handler)
                .map_err(|_| "could not listen for messages from the endpoint")
        });
        let (lease, token) = match acquired {
            Ok(acquired) => acquired,
            Err(msg) => return Err(ConnectError::other(msg, self)),
        };

        Ok(MidiInputConnection {
            client_name: self.client_name,
            registration: EventRegistration {
                connection: lease.connection.clone(),
                token,
            },
            lease,
            handler_data,
        })
    }
}

type InputCallback<T> = Box<dyn FnMut(u64, &[u8], &mut T) + Send>;

/// This is all the data that is stored on the heap as long as a connection
/// is opened and passed to the callback handler.
struct HandlerData<T> {
    ignore_flags: Ignore,
    decoder: UmpToBytes,
    ticks: TickConverter,
    callback: InputCallback<T>,
    user_data: Option<T>,
}

fn is_ignored(ignore: Ignore, message: &[u8]) -> bool {
    match message.first() {
        Some(0xF0) => ignore.contains(Ignore::Sysex),
        Some(0xF1) | Some(0xF8) => ignore.contains(Ignore::Time),
        Some(0xFE) => ignore.contains(Ignore::ActiveSense),
        _ => false,
    }
}

/// Runs on a Windows MIDI Services thread. Nothing here may panic across the WinRT boundary.
fn handle_input<T>(args: &MidiMessageReceivedEventArgs, handler_data: &Mutex<HandlerData<T>>) {
    let mut words = [0u32; 4];
    let [w0, w1, w2, w3] = &mut words;
    let count = (args.FillWords(w0, w1, w2, w3) as usize).min(4);
    if count == 0 {
        return;
    }
    let timestamp = args.Timestamp();

    let mut guard = match handler_data.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let HandlerData {
        ignore_flags,
        decoder,
        ticks,
        callback,
        user_data,
    } = &mut *guard;
    // The connection is being closed and the user data has already been handed back.
    let Some(user_data) = user_data.as_mut() else {
        return;
    };
    let ignore = *ignore_flags;
    let microseconds = ticks.to_microseconds(timestamp);

    decoder.decode(&words[..count], |message| {
        if is_ignored(ignore, message) {
            return;
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            callback(microseconds, message, user_data);
        }));
        if result.is_err() {
            error!("A MIDI input callback panicked. The message was dropped.");
        }
    });
}

/// Removes the input handler when dropped.
struct EventRegistration {
    connection: MidiEndpointConnection,
    token: i64,
}

impl Drop for EventRegistration {
    fn drop(&mut self) {
        if let Err(e) = self.connection.RemoveMessageReceived(self.token) {
            warn!("Removing a MIDI input handler failed: {e:?}");
        }
    }
}

pub struct MidiInputConnection<T> {
    client_name: String,
    // Declared before `lease`, so the handler is removed before the connection is released.
    registration: EventRegistration,
    lease: ConnectionLease,
    handler_data: Arc<Mutex<HandlerData<T>>>,
}

impl<T> MidiInputConnection<T> {
    pub fn close(self) -> (MidiInput, T) {
        let MidiInputConnection {
            client_name,
            registration,
            lease,
            handler_data,
        } = self;
        drop(registration);
        drop(lease);

        let mut guard = handler_data.lock().unwrap_or_else(|e| e.into_inner());
        let user_data = guard
            .user_data
            .take()
            .expect("user data is only taken here");
        (
            MidiInput {
                client_name,
                ignore_flags: guard.ignore_flags,
            },
            user_data,
        )
    }
}

// ---------------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
pub struct MidiOutputPort {
    port_device_id: HSTRING,
    endpoint_device_id: HSTRING,
    group: u8,
}

impl MidiOutputPort {
    pub fn id(&self) -> String {
        self.port_device_id.to_string_lossy()
    }
}

pub struct MidiOutput {
    client_name: String,
}

impl MidiOutput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        ensure_available()?;
        Ok(MidiOutput {
            client_name: client_name.to_string(),
        })
    }

    pub(crate) fn ports_internal(&self) -> Vec<crate::common::MidiOutputPort> {
        enumerate_ports(Midi1PortFlow::MidiMessageDestination)
            .into_iter()
            .map(|p| crate::common::MidiOutputPort {
                imp: MidiOutputPort {
                    port_device_id: p.port_device_id,
                    endpoint_device_id: p.endpoint_device_id,
                    group: p.group,
                },
            })
            .collect()
    }

    pub fn port_count(&self) -> usize {
        enumerate_ports(Midi1PortFlow::MidiMessageDestination).len()
    }

    pub fn port_name(&self, port: &MidiOutputPort) -> Result<String, PortInfoError> {
        port_name(&port.port_device_id)
    }

    pub fn connect(
        self,
        port: &MidiOutputPort,
        _port_name: &str,
    ) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        let acquired =
            ConnectionLease::acquire(&self.client_name, &port.endpoint_device_id, |_| Ok(0));
        let (lease, _) = match acquired {
            Ok(acquired) => acquired,
            Err(msg) => return Err(ConnectError::other(msg, self)),
        };
        let max_words = lease
            .connection
            .GetSupportedMaxMidiWordsPerTransmission()
            .max(2) as usize;
        let send_now = MidiClock::TimestampConstantSendImmediately().unwrap_or(0);

        Ok(MidiOutputConnection {
            client_name: self.client_name,
            lease,
            encoder: BytesToUmp::new(port.group),
            packets: Vec::with_capacity(4),
            words: Vec::with_capacity(8),
            max_words,
            send_now,
        })
    }
}

pub struct MidiOutputConnection {
    client_name: String,
    lease: ConnectionLease,
    encoder: BytesToUmp,
    packets: Vec<Ump>,
    words: Vec<u32>,
    max_words: usize,
    send_now: u64,
}

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        let MidiOutputConnection {
            client_name, lease, ..
        } = self;
        drop(lease);
        MidiOutput { client_name }
    }

    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        self.packets.clear();
        self.encoder
            .encode(message, &mut self.packets)
            .map_err(SendError::InvalidData)?;

        let connection = &self.lease.connection;
        if let [only] = self.packets.as_slice() {
            let result = match only.as_slice() {
                [w0] => connection.SendSingleMessageWords(self.send_now, *w0),
                [w0, w1] => connection.SendSingleMessageWords2(self.send_now, *w0, *w1),
                _ => return Err(SendError::Other("unexpected UMP size")),
            };
            return check_send_result(result);
        }

        // Several UMPs, such as a long SysEx message: send as many whole UMPs at once as the
        // connection takes.
        self.words.clear();
        for packet in &self.packets {
            if self.words.len() + packet.as_slice().len() > self.max_words {
                send_words(connection, self.send_now, &self.words)?;
                self.words.clear();
            }
            self.words.extend_from_slice(packet.as_slice());
        }
        if !self.words.is_empty() {
            send_words(connection, self.send_now, &self.words)?;
        }
        Ok(())
    }
}

fn send_words(
    connection: &MidiEndpointConnection,
    timestamp: u64,
    words: &[u32],
) -> Result<(), SendError> {
    let result = connection.SendMultipleMessagesWordArray(timestamp, 0, words.len() as u32, words);
    check_send_result(result)
}

/// `MidiSendMessageResults` is a flags type, so use the API's own test for success.
fn check_send_result(result: MidiSendMessageResults) -> Result<(), SendError> {
    match MidiEndpointConnection::SendMessageSucceeded(result) {
        Ok(true) => Ok(()),
        Ok(false) => Err(SendError::Other(
            "the MIDI service did not accept the message",
        )),
        Err(_) => Err(SendError::Other("sending the message failed")),
    }
}
