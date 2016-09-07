use ::errors::*;
use ::backend::{
    MidiInput as MidiInputImpl,
    MidiInputConnection as MidiInputConnectionImpl,
    MidiOutput as MidiOutputImpl,
    MidiOutputConnection as MidiOutputConnectionImpl
};
use ::Ignore;

// TODO: add documentation to this module

pub struct MidiInput {
    //ignore_flags: Ignore
    imp: MidiInputImpl
}

impl MidiInput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        MidiInputImpl::new(client_name).map(|imp| MidiInput { imp: imp })
    }
    
    pub fn ignore(&mut self, flags: Ignore) {
       self.imp.ignore(flags);
    }
    
    pub fn port_count(&self) -> u32 {
        self.imp.port_count()
    }
    
    pub fn port_name(&self, port_number: u32) -> Result<String, PortInfoError> {
        self.imp.port_name(port_number)
    }
    
    pub fn connect<F, T: Send>(
        self, port_number: u32, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
        where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        match self.imp.connect(port_number, port_name, callback, data) {
            Ok(imp) => Ok(MidiInputConnection { imp: imp }),
            Err(imp) => {
                let kind = imp.kind();
                Err(ConnectError::new(kind, MidiInput { imp: imp.into_inner() }))
            } 
        }
    }
}

#[cfg(unix)]
impl<T: Send> ::os::nix::VirtualInput<T> for MidiInput {
    fn create_virtual<F>(
        self, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<Self>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        match self.imp.create_virtual(port_name, callback, data) {
            Ok(imp) => Ok(MidiInputConnection { imp: imp }),
            Err(imp) => {
                let kind = imp.kind();
                Err(ConnectError::new(kind, MidiInput { imp: imp.into_inner() }))
            } 
        }
    }
}

pub struct MidiInputConnection<T: 'static> {
    imp: MidiInputConnectionImpl<T>
}

impl<T> MidiInputConnection<T> {
    pub fn close(self) -> (MidiInput, T) {
        let (imp, data) = self.imp.close();
        (MidiInput { imp: imp }, data)
    }
}

pub struct MidiOutput {
    imp: MidiOutputImpl
}

impl MidiOutput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        MidiOutputImpl::new(client_name).map(|imp| MidiOutput { imp: imp })
    }
    
    pub fn port_count(&self) -> u32 {
        self.imp.port_count()
    }
    
    pub fn port_name(&self, port_number: u32) -> Result<String, PortInfoError> {
        self.imp.port_name(port_number)
    }
    
    pub fn connect(self, port_number: u32, port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
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
impl ::os::nix::VirtualOutput for MidiOutput {
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

pub struct MidiOutputConnection {
   imp: MidiOutputConnectionImpl
}

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        MidiOutput { imp: self.imp.close() }
    }
    
    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        self.imp.send(message)
    }

    #[cfg(target_os="windows")]
    pub fn send_short_message(&mut self, message: u32) -> Result<(), SendError> {
        self.imp.send_short_message(message)
    }
}