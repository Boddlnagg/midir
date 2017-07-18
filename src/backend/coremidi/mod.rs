use std::sync::{Arc, Mutex};

use ::errors::*;
use ::Ignore;
use ::MidiMessage;

use ::coremidi::*;

mod external {
    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        pub fn AudioConvertHostTimeToNanos(inHostTime: u64) -> u64;
        pub fn AudioGetCurrentHostTime() -> u64;
    }
}

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

    fn handle_input<T>(packets: &PacketList, handler_data: &mut HandlerData<T>) {
        let continue_sysex =  &mut handler_data.continue_sysex;
        let ignore = handler_data.ignore_flags;
        let message = &mut handler_data.message;
        let data = &mut handler_data.user_data.as_mut().unwrap();
        for p in packets.iter() {
            let pdata = p.data();
            if pdata.len() == 0 { continue; }

            let mut timestamp = p.timestamp();
            if timestamp == 0 { // this might happen for asnychronous sysex messages (?)
                timestamp = unsafe { external::AudioGetCurrentHostTime() };
            }

            let relative_timestamp = match handler_data.last_time {
                None => 0,
                Some(last) => timestamp - last
            };
            handler_data.last_time = Some(timestamp);

            if !*continue_sysex {
                message.timestamp = unsafe { external::AudioConvertHostTimeToNanos(relative_timestamp) } as f64 * 0.000000001;
            }

            let mut cur_byte = 0;
            if *continue_sysex {
                // We have a continuing, segmented sysex message.
                if !ignore.contains(Ignore::Sysex) {
                    // If we're not ignoring sysex messages, copy the entire packet.
                    message.bytes.extend_from_slice(pdata);
                }
                *continue_sysex = pdata[pdata.len() - 1] != 0xF7;

                if !ignore.contains(Ignore::Sysex) && !*continue_sysex {
                    // If we reached the end of the sysex, invoke the user callback
                    (handler_data.callback)(message.timestamp, &message.bytes, data);
                    message.bytes.clear();
                }
            } else {
                while cur_byte < pdata.len() {
                    // We are expecting that the next byte in the packet is a status byte.
                    let status = pdata[cur_byte];
                    if status & 0x80 == 0 { break; }
                    // Determine the number of bytes in the MIDI message.
                    let size;
                    if status < 0xC0 { size = 3; }
                    else if status < 0xE0 { size = 2; }
                    else if status < 0xF0 { size = 3; }
                    else if status == 0xF0 {
                        // A MIDI sysex
                        if ignore.contains(Ignore::Sysex) {
                            size = 0;
                            cur_byte = pdata.len();
                        } else {
                            size = pdata.len() - cur_byte;
                        }
                        *continue_sysex = pdata[pdata.len() - 1] != 0xF7;
                    }
                    else if status == 0xF1 {
                        // A MIDI time code message
                        if ignore.contains(Ignore::Time) {
                            size = 0;
                            cur_byte += 2;
                        } else {
                            size = 2;
                        }
                    }
                    else if status == 0xF2 { size = 3; }
                    else if status == 0xF3 { size = 2; }
                    else if status == 0xF8 && ignore.contains(Ignore::Time) {
                        // A MIDI timing tick message and we're ignoring it.
                        size = 0;
                        cur_byte += 1;
                    }
                    else if status == 0xFE && ignore.contains(Ignore::ActiveSense) {
                        // A MIDI active sensing message and we're ignoring it.
                        size = 0;
                        cur_byte += 1;
                    }
                    else { size = 1; }

                    // Copy the MIDI data to our vector.
                    if size > 0 {
                        let message_bytes = &pdata[cur_byte..(cur_byte + size)];
                        if !*continue_sysex {
                            // This is either a non-sysex message or a non-segmented sysex message
                            (handler_data.callback)(message.timestamp, message_bytes, data);
                            message.bytes.clear();
                        } else {
                            // This is the beginning of a segmented sysex message
                            message.bytes.extend_from_slice(message_bytes);
                        }
                        cur_byte += size;
                    }
                }
            }
        }
    }
    
    pub fn connect<F, T: Send + 'static>(
        self, port_number: usize, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
        where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        let src = match Source::from_index(port_number) {
            Some(src) => src,
            None => return Err(ConnectError::new(ConnectErrorKind::PortNumberOutOfRange, self))
        };

        let handler_data = Arc::new(Mutex::new(HandlerData {
            message: MidiMessage::new(),
            last_time: None,
            ignore_flags: self.ignore_flags,
            continue_sysex: false,
            callback: Box::new(callback),
            user_data: Some(data)
        }));
        let handler_data2 = handler_data.clone();
        let port = match self.client.input_port(port_name, move |packets| {
            MidiInput::handle_input(packets, &mut *handler_data2.lock().unwrap())
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

    pub fn create_virtual<F, T: Send + 'static>(
        self, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static {

        let handler_data = Arc::new(Mutex::new(HandlerData {
            message: MidiMessage::new(),
            last_time: None,
            ignore_flags: self.ignore_flags,
            continue_sysex: false,
            callback: Box::new(callback),
            user_data: Some(data)
        }));
        let handler_data2 = handler_data.clone();
        let vrt = match self.client.virtual_destination(port_name, move |packets| {
            MidiInput::handle_input(packets, &mut *handler_data2.lock().unwrap())
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
    #[allow(dead_code)]
    details: InputConnectionDetails,
    // TODO: get rid of Arc & Mutex?
    //       synchronization is required because the borrow checker does not
    //       know that the callback we're in here is never called concurrently
    //       (always in sequence)
    handler_data: Arc<Mutex<HandlerData<T>>>
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
    message: MidiMessage,
    last_time: Option<u64>,
    ignore_flags: Ignore,
    continue_sysex: bool,
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