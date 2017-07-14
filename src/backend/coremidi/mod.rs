#![allow(dead_code)]
#![allow(unused_variables)]

use ::errors::*;
use ::Ignore;

pub struct MidiInput {
    ignore_flags: Ignore
}

impl MidiInput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        unimplemented!()
    }

    pub fn ignore(&mut self, flags: Ignore) {
        unimplemented!()
    }
    
    pub fn port_count(&self) -> usize {
        unimplemented!()
    }
    
    pub fn port_name(&self, port_number: usize) -> Result<String, PortInfoError> {
        unimplemented!()
    }
    
    pub fn connect<F, T: Send>(
        self, port_number: usize, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
        where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        unimplemented!()
    }

    pub fn create_virtual<F, T: Send>(
        self, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<Self>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        unimplemented!()
    }
}

pub struct MidiInputConnection<T> {
    handler_data: Box<HandlerData<T>>,
}

/// This is all the data that is stored on the heap as long as a connection
/// is opened and passed to the callback handler.
///
/// It is important that `user_data` is the last field to not influence
/// offsets after monomorphization.
struct HandlerData<T> {
    last_time: Option<u64>,
    ignore_flags: Ignore,
    //callback: Box<FnMut(f64, &[u8], &mut T)+Send>,
    user_data: Option<T>
}

impl<T> MidiInputConnection<T> {
    pub fn close(self) -> (MidiInput, T) {
        unimplemented!()
    }
}

pub struct MidiOutput {
    
}

impl MidiOutput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        unimplemented!()
    }
    
    pub fn port_count(&self) -> usize {
        unimplemented!()
    }
    
    pub fn port_name(&self, port_number: usize) -> Result<String, PortInfoError> {
        unimplemented!()
    }
    
    pub fn connect(self, port_number: usize, port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        unimplemented!()
    }

    pub fn create_virtual(self, port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        unimplemented!()
    }
}

pub struct MidiOutputConnection {
   
}

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        unimplemented!()
    }
    
    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        unimplemented!()
    }
}