pub trait PortInfo {
    fn new(client_name: &str) -> Result<Self, super::InitError>;
	fn port_count(&self) -> u32;
    fn port_name(&self, port_number: u32) -> Result<String, super::PortInfoError>;
}

pub trait InputConnect<T: Send> {
    type Connection: InputConnection<T>;
    
    fn connect<F>(
        self, port_number: u32, port_name: &str, callback: F, data: T
    ) -> Result<Self::Connection, super::ConnectError<Self>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static;
}

pub trait InputConnection<T> {
    type Input;
    
    fn close(mut self) -> (Self::Input, T);
}

pub trait OutputConnect {
    type Connection: OutputConnection;
    
    fn connect(
        self, port_number: u32, port_name: &str
    ) -> Result<Self::Connection, super::ConnectError<Self>>;
}

pub trait OutputConnection {
    type Output;
    
    fn close(self) -> Self::Output;
    fn send_message(&mut self, message: &[u8]) -> Result<(), super::SendError>;   
}