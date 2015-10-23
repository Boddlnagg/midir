use ::errors::*;

pub trait PortInfo where Self: Sized {
    fn new(client_name: &str) -> Result<Self, InitError>;
    fn port_count(&self) -> u32;
    fn port_name(&self, port_number: u32) -> Result<String, PortInfoError>;
}

pub trait InputConnect<T: Send> where Self: Sized {
    type Connection: InputConnection<T>;
    
    fn connect<F>(
        self, port_number: u32, port_name: &str, callback: F, data: T
    ) -> Result<Self::Connection, ConnectError<Self>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static;
}

pub trait InputConnection<T> {
    type Input;
    
    fn close(mut self) -> (Self::Input, T);
}

pub trait OutputConnect where Self: Sized {
    type Connection: OutputConnection;
    
    fn connect(
        self, port_number: u32, port_name: &str
    ) -> Result<Self::Connection, ConnectError<Self>>;
}

pub trait OutputConnection {
    type Output;
    
    fn close(self) -> Self::Output;
    fn send(&mut self, message: &[u8]) -> Result<(), SendError>;   
}