#![allow(dead_code)]
#![allow(unused_variables)]

use std::sync::{Arc, Mutex};

use ::errors::*;
use ::Ignore;

use ::coremidi::*;

pub struct MidiInput {
    client: Client,
    ignore_flags: Ignore
}

impl MidiInput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        match Client::new(client_name) {
            Ok(cl) => Ok(MidiInput { client: cl, ignore_flags: Ignore::None }),
            Err(_) => Err(InitError)
        }
    }

    pub fn ignore(&mut self, flags: Ignore) {
        self.ignore_flags = flags;
    }
    
    pub fn port_count(&self) -> usize {
        Sources::count()
    }
    
    pub fn port_name(&self, port_number: usize) -> Result<String, PortInfoError> {
        let endpoint = try!(Source::from_index(port_number).ok_or(PortInfoError::PortNumberOutOfRange));
        match endpoint.display_name() {
            Some(name) => Ok(name),
            None => Err(PortInfoError::CannotRetrievePortName)
        }
    }
    
    pub fn connect<F, T: Send + 'static>(
        self, port_number: usize, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
        where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        // TODO: handle failure of from_index for invalid index
        let src = match Source::from_index(port_number) {
            Some(src) => src,
            None => return Err(ConnectError::new(ConnectErrorKind::PortNumberOutOfRange, self))
        };

        let handler_data = Arc::new(Mutex::new(HandlerData {
            ignore_flags: self.ignore_flags,
            callback: Box::new(callback),
            user_data: Some(data)
        }));
        let handler_data2 = handler_data.clone();
        let port = match self.client.input_port(port_name, move |packets| {
            // TODO: implement message filtering
            // TODO: calculate timestamps
            // TODO: maybe merge SysEx (if they can be split)
            // TODO: synchronization is required because the borrow checker does not
            //       know that the callback we're in here is never called concurrently
            //       (always in sequence)
            let mut data_locked = handler_data2.lock().unwrap();
            let dataref: &mut HandlerData<T> = &mut *data_locked;
            let data = &mut dataref.user_data.as_mut().unwrap();
            for p in packets.iter() {
                (dataref.callback)(0.0, p.data(), data)
            }
        }) {
            Ok(p) => p,
            Err(_) => return Err(ConnectError::other("error creating MIDI input port", self))
        };
        if let Err(_) = port.connect_source(&src) {
            return Err(ConnectError::other("error connecting MIDI input port", self));
        }
        Ok(MidiInputConnection {
            client: self.client,
            details: InputConnectionDetails::Explicit(port),
            handler_data: handler_data
        })
    }

    // FIXME: get rid of code duplication with above method
    // FIXME: the test_virtual example currently fails for some reason 
    pub fn create_virtual<F, T: Send + 'static>(
        self, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static {

        let handler_data = Arc::new(Mutex::new(HandlerData {
            ignore_flags: self.ignore_flags,
            callback: Box::new(callback),
            user_data: Some(data)
        }));
        let handler_data2 = handler_data.clone();
        let vrt = match self.client.virtual_destination(port_name, move |packets| {
            // TODO: implement message filtering
            // TODO: calculate timestamps
            // TODO: maybe merge SysEx (if they can be split)
            // TODO: synchronization is required because the borrow checker does not
            //       know that the callback we're in here is never called concurrently
            //       (always in sequence)
            let mut data_locked = handler_data2.lock().unwrap();
            let dataref: &mut HandlerData<T> = &mut *data_locked;
            let data = &mut dataref.user_data.as_mut().unwrap();
            for p in packets.iter() {
                (dataref.callback)(0.0, p.data(), data)
            }
        }) {
            Ok(p) => p,
            Err(_) => return Err(ConnectError::other("error creating MIDI input port", self))
        };
        Ok(MidiInputConnection {
            client: self.client,
            details: InputConnectionDetails::Virtual(vrt),
            handler_data: handler_data
        })
    }
}

enum InputConnectionDetails {
    Explicit(InputPort),
    Virtual(VirtualDestination)
}

pub struct MidiInputConnection<T> {
    client: Client,
    details: InputConnectionDetails,
    handler_data: Arc<Mutex<HandlerData<T>>> // TODO: get rid of Arc & Mutex?
}

impl<T> MidiInputConnection<T> {
    pub fn close(self) -> (MidiInput, T) {
        let mut handler_data_locked = self.handler_data.lock().unwrap();
        (MidiInput {
            client: self.client,
            ignore_flags: handler_data_locked.ignore_flags
        }, handler_data_locked.user_data.take().unwrap())
    }
}

/// This is all the data that is stored on the heap as long as a connection
/// is opened and passed to the callback handler.
///
/// It is important that `user_data` is the last field to not influence
/// offsets after monomorphization.
struct HandlerData<T> {
    //last_time: Option<u64>,
    ignore_flags: Ignore,
    callback: Box<FnMut(f64, &[u8], &mut T)+Send>,
    user_data: Option<T>
}

pub struct MidiOutput {
    client: Client
}

impl MidiOutput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        match Client::new(client_name) {
            Ok(cl) => Ok(MidiOutput { client: cl }),
            Err(_) => Err(InitError)
        }
    }
    
    pub fn port_count(&self) -> usize {
        Destinations::count()
    }
    
    pub fn port_name(&self, port_number: usize) -> Result<String, PortInfoError> {
        let endpoint = try!(Destination::from_index(port_number).ok_or(PortInfoError::PortNumberOutOfRange));
        match endpoint.display_name() {
            Some(name) => Ok(name),
            None => Err(PortInfoError::CannotRetrievePortName)
        }
    }
    
    pub fn connect(self, port_number: usize, port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        // TODO: handle failure of from_index for invalid index
        let dest = match Destination::from_index(port_number) {
            Some(dest) => dest,
            None => return Err(ConnectError::new(ConnectErrorKind::PortNumberOutOfRange, self))
        };

        let port = match self.client.output_port(port_name) {
            Ok(p) => p,
            Err(_) => return Err(ConnectError::other("error creating MIDI output port", self))
        };
        Ok(MidiOutputConnection {
            client: self.client,
            details: OutputConnectionDetails::Explicit(port, dest)
        })
    }

    pub fn create_virtual(self, port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        let vrt = match self.client.virtual_source(port_name) {
            Ok(p) => p,
            Err(_) => return Err(ConnectError::other("error creating virtual MIDI source", self))
        };
        Ok(MidiOutputConnection {
            client: self.client,
            details: OutputConnectionDetails::Virtual(vrt)
        })
    }
}

enum OutputConnectionDetails {
    Explicit(OutputPort, Destination),
    Virtual(VirtualSource)
}

pub struct MidiOutputConnection {
    client: Client,
    details: OutputConnectionDetails
}

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        MidiOutput { client: self.client }
    }
    
    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        // TODO: get rid of to_vec(), there should be no need to allocate here
        let packets = PacketBuffer::from_data(0, message.to_vec());
        match self.details {
            OutputConnectionDetails::Explicit(ref port, ref dest) => {
                port.send(&dest, &packets).map_err(|_| SendError::Other("error sending MIDI message to port"))
            },
            OutputConnectionDetails::Virtual(ref vrt) => {
                vrt.received(&packets).map_err(|_| SendError::Other("error sending MIDI to virtual destinations"))
            }
        }
        
    }
}