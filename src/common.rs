use ::errors::*;
use ::backend::{
    MidiInput as MidiInputImpl,
    MidiInputConnection as MidiInputConnectionImpl,
    MidiOutput as MidiOutputImpl,
    MidiOutputConnection as MidiOutputConnectionImpl
};
use ::Ignore;

/// An instance of `MidiInput` is required for anything related to MIDI input.
/// Create one with `MidiInput::new`.
pub struct MidiInput {
    //ignore_flags: Ignore
    imp: MidiInputImpl
}

impl MidiInput {
    /// Creates a new `MidiInput` object that is required for any MIDI input functionality.
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        MidiInputImpl::new(client_name).map(|imp| MidiInput { imp: imp })
    }
    
    /// Set flags to decide what kind of messages should be ignored (i.e., filtered out)
    /// by this `MidiInput`. By default, no messages are ignored.
    pub fn ignore(&mut self, flags: Ignore) {
       self.imp.ignore(flags);
    }
    
    /// Get the number of available MIDI input ports that *midir* can connect to.
    pub fn port_count(&self) -> usize {
        self.imp.port_count()
    }
    
    /// Get the name of a specified MIDI input port.
    pub fn port_name(&self, port_number: usize) -> Result<String, PortInfoError> {
        self.imp.port_name(port_number)
    }
    
    /// Connect to a specified MIDI input port in order to receive messages.
    /// For each incoming MIDI message, the provided `callback` closure will
    /// be called.
    ///
    /// The connection will be kept open as long as the returned
    /// `MidiInputConnection` is kept alive.
    ///
    /// The `port_name` is an additional name that will be assigned to the
    /// connection. It is only used by some backends.
    pub fn connect<F>(
        self, port_number: usize, port_name: &str, callback: F
    ) -> Result<MidiInputConnection, ConnectError<MidiInput>>
        where F: FnMut(f64, &[u8]) + Send + 'static {
        match self.imp.connect(port_number, port_name, callback) {
            Ok(imp) => Ok(MidiInputConnection { imp: imp }),
            Err(imp) => {
                let kind = imp.kind();
                Err(ConnectError::new(kind, MidiInput { imp: imp.into_inner() }))
            } 
        }
    }
}

#[cfg(unix)]
impl ::os::unix::VirtualInput for MidiInput {
    fn create_virtual<F>(
        self, port_name: &str, callback: F
    ) -> Result<MidiInputConnection, ConnectError<Self>>
    where F: FnMut(f64, &[u8]) + Send + 'static {
        match self.imp.create_virtual(port_name, callback) {
            Ok(imp) => Ok(MidiInputConnection { imp: imp }),
            Err(imp) => {
                let kind = imp.kind();
                Err(ConnectError::new(kind, MidiInput { imp: imp.into_inner() }))
            } 
        }
    }
}

/// Represents an open connection to a MIDI input port.
pub struct MidiInputConnection {
    imp: MidiInputConnectionImpl
}

impl MidiInputConnection {
    /// Closes the connection. The returned values allow you to
    /// inspect the additional data passed to the callback (the `data`
    /// parameter of `connect`), or to reuse the `MidiInput` object,
    /// but they can be safely ignored.
    pub fn close(self) -> MidiInput {
        let imp = self.imp.close();
        MidiInput { imp: imp }
    }
}

/// An instance of `MidiOutput` is required for anything related to MIDI output.
/// Create one with `MidiOutput::new`.
pub struct MidiOutput {
    imp: MidiOutputImpl
}

impl MidiOutput {
    /// Creates a new `MidiOutput` object that is required for any MIDI output functionality.
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        MidiOutputImpl::new(client_name).map(|imp| MidiOutput { imp: imp })
    }
    
    /// Get the number of available MIDI output ports that *midir* can connect to.
    pub fn port_count(&self) -> usize {
        self.imp.port_count()
    }
    
    /// Get the name of a specified MIDI output port.
    pub fn port_name(&self, port_number: usize) -> Result<String, PortInfoError> {
        self.imp.port_name(port_number)
    }
    
    /// Connect to a specified MIDI output port in order to send messages.
    /// The connection will be kept open as long as the returned
    /// `MidiOutputConnection` is kept alive.
    ///
    /// The `port_name` is an additional name that will be assigned to the
    /// connection. It is only used by some backends.
    pub fn connect(self, port_number: usize, port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        match self.imp.connect(port_number, port_name) {
            Ok(imp) => Ok(MidiOutputConnection { imp: imp }),
            Err(imp) => {
                let kind = imp.kind();
                Err(ConnectError::new(kind, MidiOutput { imp: imp.into_inner() }))
            } 
        }
    }
}

#[cfg(unix)]
impl ::os::unix::VirtualOutput for MidiOutput {
    fn create_virtual(self, port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        match self.imp.create_virtual(port_name) {
            Ok(imp) => Ok(MidiOutputConnection { imp: imp }),
            Err(imp) => {
                let kind = imp.kind();
                Err(ConnectError::new(kind, MidiOutput { imp: imp.into_inner() }))
            } 
        }
    }
}

/// Represents an open connection to a MIDI output port.
pub struct MidiOutputConnection {
   imp: MidiOutputConnectionImpl
}

impl MidiOutputConnection {
    /// Closes the connection. The returned value allows you to
    /// reuse the `MidiOutput` object, but it can be safely ignored.
    pub fn close(self) -> MidiOutput {
        MidiOutput { imp: self.imp.close() }
    }
    
    /// Send a message to the port that this output connection is connected to.
    /// The message must be a valid MIDI message (see https://www.midi.org/specifications/item/table-1-summary-of-midi-message).
    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        self.imp.send(message)
    }
}