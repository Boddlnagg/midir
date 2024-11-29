use jack_sys::jack_nframes_t;
use libc::c_void;
use std::fmt;

use std::ffi::CString;
use std::fmt::{Debug, Formatter};
use std::{mem, slice};

mod wrappers;
use self::wrappers::*;

use crate::errors::*;
use crate::{Ignore, MidiMessage};

const OUTPUT_RINGBUFFER_SIZE: usize = 16384;

struct InputHandlerData<T> {
    port: Option<MidiPort>,
    ignore_flags: Ignore,
    callback: Box<dyn FnMut(u64, &[u8], &mut T) + Send>,
    user_data: Option<T>,
}

pub struct MidiInput {
    ignore_flags: Ignore,
    client: Option<Client>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MidiInputPort {
    name: CString,
}

impl MidiInputPort {
    pub fn id(&self) -> String {
        self.name.to_string_lossy().to_string()
    }
}

pub struct MidiInputConnection<T> {
    handler_data: Box<InputHandlerData<T>>,
    client: Option<Client>,
}

impl MidiInput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        let client = match Client::open(client_name, JackOpenOptions::NoStartServer) {
            Ok(c) => c,
            Err(_) => {
                return Err(InitError);
            } // TODO: maybe add message that Jack server might not be running
        };

        Ok(MidiInput {
            ignore_flags: Ignore::None,
            client: Some(client),
        })
    }

    pub fn ignore(&mut self, flags: Ignore) {
        self.ignore_flags = flags;
    }

    pub(crate) fn ports_internal(&self) -> Vec<crate::common::MidiInputPort> {
        let ports = self
            .client
            .as_ref()
            .unwrap()
            .get_midi_ports(PortFlags::PortIsOutput);
        let mut result = Vec::with_capacity(ports.count());
        for i in 0..ports.count() {
            result.push(crate::common::MidiInputPort {
                imp: MidiInputPort {
                    name: ports.get_c_name(i).into(),
                },
            })
        }
        result
    }

    pub fn port_count(&self) -> usize {
        self.client
            .as_ref()
            .unwrap()
            .get_midi_ports(PortFlags::PortIsOutput)
            .count()
    }

    pub fn port_name(&self, port: &MidiInputPort) -> Result<String, PortInfoError> {
        Ok(port.name.to_string_lossy().into())
    }

    fn activate_callback<F, T: Send>(&mut self, callback: F, data: T) -> Box<InputHandlerData<T>>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        let handler_data = Box::new(InputHandlerData {
            port: None,
            ignore_flags: self.ignore_flags,
            callback: Box::new(callback),
            user_data: Some(data),
        });

        let data_ptr = unsafe { mem::transmute_copy::<_, *mut InputHandlerData<T>>(&handler_data) };

        self.client
            .as_mut()
            .unwrap()
            .set_process_callback(handle_input::<T>, data_ptr as *mut c_void);
        self.client.as_mut().unwrap().activate();
        handler_data
    }

    pub fn connect<F, T: Send>(
        mut self,
        port: &MidiInputPort,
        port_name: &str,
        callback: F,
        data: T,
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        let mut handler_data = self.activate_callback(callback, data);

        // Create port ...
        let dest_port = match self
            .client
            .as_mut()
            .unwrap()
            .register_midi_port(port_name, PortFlags::PortIsInput)
        {
            Ok(p) => p,
            Err(()) => {
                return Err(ConnectError::other("could not register JACK port", self));
            }
        };

        // ... and connect it to the output
        if let Err(_) = self
            .client
            .as_mut()
            .unwrap()
            .connect(&port.name, dest_port.get_name())
        {
            return Err(ConnectError::new(ConnectErrorKind::InvalidPort, self));
        }

        handler_data.port = Some(dest_port);

        Ok(MidiInputConnection {
            handler_data: handler_data,
            client: self.client.take(),
        })
    }

    pub fn create_virtual<F, T: Send>(
        mut self,
        port_name: &str,
        callback: F,
        data: T,
    ) -> Result<MidiInputConnection<T>, ConnectError<Self>>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        let mut handler_data = self.activate_callback(callback, data);

        // Create port
        let port = match self
            .client
            .as_mut()
            .unwrap()
            .register_midi_port(port_name, PortFlags::PortIsInput)
        {
            Ok(p) => p,
            Err(()) => {
                return Err(ConnectError::other("could not register JACK port", self));
            }
        };

        handler_data.port = Some(port);

        Ok(MidiInputConnection {
            handler_data: handler_data,
            client: self.client.take(),
        })
    }
}

impl<T> MidiInputConnection<T> {
    pub fn close(mut self) -> (MidiInput, T) {
        self.close_internal();

        (
            MidiInput {
                client: self.client.take(),
                ignore_flags: self.handler_data.ignore_flags,
            },
            self.handler_data.user_data.take().unwrap(),
        )
    }

    fn close_internal(&mut self) {
        let port = self.handler_data.port.take().unwrap();
        self.client.as_mut().unwrap().unregister_midi_port(port);
        self.client.as_mut().unwrap().deactivate();
    }
}

impl<T> Drop for MidiInputConnection<T> {
    fn drop(&mut self) {
        if self.client.is_some() {
            self.close_internal();
        }
    }
}

impl<T> Debug for MidiInputConnection<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiInputConnection")
            .field("port", &self.handler_data.port)
            .finish()
    }
}

extern "C" fn handle_input<T>(nframes: jack_nframes_t, arg: *mut c_void) -> i32 {
    let data: &mut InputHandlerData<T> = unsafe { &mut *(arg as *mut InputHandlerData<T>) };

    // Is port created?
    if let Some(ref port) = data.port {
        let buff = port.get_midi_buffer(nframes);

        let mut message = MidiMessage::new(); // TODO: create MidiMessage once and reuse its buffer for every handle_input call

        // We have midi events in buffer
        let evcount = buff.get_event_count();
        let mut event = mem::MaybeUninit::uninit();

        for j in 0..evcount {
            message.bytes.clear();
            unsafe { buff.get_event(event.as_mut_ptr(), j) };
            let event = unsafe { event.assume_init() };

            for i in 0..event.size {
                message
                    .bytes
                    .push(unsafe { *event.buffer.offset(i as isize) });
            }

            message.timestamp = Client::get_time(); // this is in microseconds
            (data.callback)(
                message.timestamp,
                &message.bytes,
                data.user_data.as_mut().unwrap(),
            );
        }
    }

    return 0;
}

struct OutputHandlerData {
    port: Option<MidiPort>,
    buff_size: Ringbuffer,
    buff_message: Ringbuffer,
}

pub struct MidiOutput {
    client: Option<Client>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MidiOutputPort {
    name: CString,
}

impl MidiOutputPort {
    pub fn id(&self) -> String {
        self.name.to_string_lossy().to_string()
    }
}

pub struct MidiOutputConnection {
    handler_data: Box<OutputHandlerData>,
    client: Option<Client>,
}

impl MidiOutput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        let client = match Client::open(client_name, JackOpenOptions::NoStartServer) {
            Ok(c) => c,
            Err(_) => {
                return Err(InitError);
            } // TODO: maybe add message that Jack server might not be running
        };

        Ok(MidiOutput {
            client: Some(client),
        })
    }

    pub(crate) fn ports_internal(&self) -> Vec<crate::common::MidiOutputPort> {
        let ports = self
            .client
            .as_ref()
            .unwrap()
            .get_midi_ports(PortFlags::PortIsInput);
        let mut result = Vec::with_capacity(ports.count());
        for i in 0..ports.count() {
            result.push(crate::common::MidiOutputPort {
                imp: MidiOutputPort {
                    name: ports.get_c_name(i).into(),
                },
            })
        }
        result
    }

    pub fn port_count(&self) -> usize {
        self.client
            .as_ref()
            .unwrap()
            .get_midi_ports(PortFlags::PortIsInput)
            .count()
    }

    pub fn port_name(&self, port: &MidiOutputPort) -> Result<String, PortInfoError> {
        Ok(port.name.to_string_lossy().into())
    }

    fn activate_callback(&mut self) -> Box<OutputHandlerData> {
        let handler_data = Box::new(OutputHandlerData {
            port: None,
            buff_size: Ringbuffer::new(OUTPUT_RINGBUFFER_SIZE),
            buff_message: Ringbuffer::new(OUTPUT_RINGBUFFER_SIZE),
        });

        let data_ptr = unsafe { mem::transmute_copy::<_, *mut OutputHandlerData>(&handler_data) };

        self.client
            .as_mut()
            .unwrap()
            .set_process_callback(handle_output, data_ptr as *mut c_void);
        self.client.as_mut().unwrap().activate();
        handler_data
    }

    pub fn connect(
        mut self,
        port: &MidiOutputPort,
        port_name: &str,
    ) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        let mut handler_data = self.activate_callback();

        // Create port ...
        let source_port = match self
            .client
            .as_mut()
            .unwrap()
            .register_midi_port(port_name, PortFlags::PortIsOutput)
        {
            Ok(p) => p,
            Err(()) => {
                return Err(ConnectError::other("could not register JACK port", self));
            }
        };

        // ... and connect it to the input
        if let Err(_) = self
            .client
            .as_mut()
            .unwrap()
            .connect(source_port.get_name(), &port.name)
        {
            return Err(ConnectError::new(ConnectErrorKind::InvalidPort, self));
        }

        handler_data.port = Some(source_port);

        Ok(MidiOutputConnection {
            handler_data: handler_data,
            client: self.client.take(),
        })
    }

    pub fn create_virtual(
        mut self,
        port_name: &str,
    ) -> Result<MidiOutputConnection, ConnectError<Self>> {
        let mut handler_data = self.activate_callback();

        // Create port
        let port = match self
            .client
            .as_mut()
            .unwrap()
            .register_midi_port(port_name, PortFlags::PortIsOutput)
        {
            Ok(p) => p,
            Err(()) => {
                return Err(ConnectError::other("could not register JACK port", self));
            }
        };

        handler_data.port = Some(port);

        Ok(MidiOutputConnection {
            handler_data: handler_data,
            client: self.client.take(),
        })
    }
}

impl MidiOutputConnection {
    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        let nbytes = message.len();

        // Write full message to buffer
        let written = self.handler_data.buff_message.write(message);
        debug_assert!(
            written == nbytes,
            "not enough bytes written to ALSA ringbuffer `message`"
        );
        let nbytes_slice = unsafe {
            slice::from_raw_parts(
                &nbytes as *const usize as *const u8,
                mem::size_of_val(&nbytes),
            )
        };
        let written = self.handler_data.buff_size.write(nbytes_slice);
        debug_assert!(
            written == mem::size_of_val(&nbytes),
            "not enough bytes written to ALSA ringbuffer `size`"
        );
        Ok(())
    }

    pub fn close(mut self) -> MidiOutput {
        self.close_internal();

        MidiOutput {
            client: self.client.take(),
        }
    }

    fn close_internal(&mut self) {
        let port = self.handler_data.port.take().unwrap();
        self.client.as_mut().unwrap().unregister_midi_port(port);
        self.client.as_mut().unwrap().deactivate();
    }
}

impl Drop for MidiOutputConnection {
    fn drop(&mut self) {
        if self.client.is_some() {
            self.close_internal();
        }
    }
}

impl Debug for MidiOutputConnection {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiOutputConnection")
            .field("port", &self.handler_data.port)
            .finish()
    }
}

extern "C" fn handle_output(nframes: jack_nframes_t, arg: *mut c_void) -> i32 {
    let data: &mut OutputHandlerData = unsafe { mem::transmute(arg) };

    // Is port created?
    if let Some(ref port) = data.port {
        let mut space: usize = 0;

        let mut buff = port.get_midi_buffer(nframes);
        buff.clear();

        while data.buff_size.get_read_space() > 0 {
            let read = data
                .buff_size
                .read(&mut space as *mut usize as *mut u8, mem::size_of::<usize>());
            debug_assert!(
                read == mem::size_of::<usize>(),
                "not enough bytes read from `size` ringbuffer"
            );
            let midi_data = buff.event_reserve(0, space);
            let read = data.buff_message.read(midi_data, space);
            debug_assert!(
                read == space,
                "not enough bytes read from `message` ringbuffer"
            );
        }
    }

    return 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::ffi::CString;
    use std::hash::{Hash, Hasher};

    fn create_test_port() -> Result<MidiInputPort, ()> {
        Ok(MidiInputPort {
            name: CString::new("test:0").unwrap(),
        })
    }

    #[test]
    fn test_backend_port_traits() {
        let port = create_test_port().unwrap();
        let port_clone = port.clone();

        // Test PartialEq
        assert_eq!(port, port_clone);

        // Test Hash consistency
        let hash1 = {
            let mut hasher = DefaultHasher::new();
            port.hash(&mut hasher);
            hasher.finish()
        };

        let hash2 = {
            let mut hasher = DefaultHasher::new();
            port_clone.hash(&mut hasher);
            hasher.finish()
        };

        assert_eq!(hash1, hash2);
    }
}