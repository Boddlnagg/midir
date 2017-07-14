#![allow(dead_code)]
#![allow(unused_variables)]

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
        match Source::from_index(port_number).display_name() {
            Some(name) => Ok(name),
            None => Err(PortInfoError::CannotRetrievePortName)
        }
    }
    
    pub fn connect<F, T: Send + 'static>(
        self, port_number: usize, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
        where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        // TODO: handle failure of from_index for invalid index
        let src = Source::from_index(port_number);
        let data_ref = &mut data;
        let port = match self.client.input_port(port_name, move |packets| {
            // TODO: filtering; maybe merge SysEx (if they can be split)
            for p in packets.iter() {
                callback(0.0, p.data(), data_ref)
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
            handler_data: HandlerData {
                ignore_flags: self.ignore_flags,
                user_data: data
            }
        })
    }

    pub fn create_virtual<F, T: Send>(
        self, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        unimplemented!()
    }
}

enum InputConnectionDetails {
    Explicit(InputPort),
    Virtual(VirtualDestination)
}

pub struct MidiInputConnection<T> {
    client: Client,
    details: InputConnectionDetails,
    handler_data: HandlerData<T>
}

/// This is all the data that is stored on the heap as long as a connection
/// is opened and passed to the callback handler.
///
/// It is important that `user_data` is the last field to not influence
/// offsets after monomorphization.
struct HandlerData<T> {
    //last_time: Option<u64>,
    ignore_flags: Ignore,
    //callback: Box<FnMut(f64, &[u8], &mut T)+Send>,
    user_data: T //Option<T>
}

impl<T> MidiInputConnection<T> {
    pub fn close(self) -> (MidiInput, T) {
        (MidiInput { client: self.client, ignore_flags: self.handler_data.ignore_flags }, self.handler_data.user_data)
    }
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
        match Destination::from_index(port_number).display_name() {
            Some(name) => Ok(name),
            None => Err(PortInfoError::CannotRetrievePortName)
        }
    }
    
    pub fn connect(self, port_number: usize, port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        // TODO: handle failure of from_index for invalid index
        let dest = Destination::from_index(port_number as usize);
        let port = match self.client.output_port(port_name) {
            Ok(p) => p,
            Err(_) => return Err(ConnectError::other("error creating MIDI output port", self))
        };
        // TODO: handle failure of from_index for invalid index
        let dest = Destination::from_index(port_number);
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