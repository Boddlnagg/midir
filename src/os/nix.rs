use traits::{InputConnection, OutputConnection};
use ::ConnectError;

pub trait VirtualInput<T: Send> {
    type Connection: InputConnection<T>;
    
    fn create_virtual<F>(
        self, port_name: &str, callback: F, data: T
    ) -> Result<Self::Connection, ConnectError<Self>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static;
}

pub trait VirtualOutput {
    type Connection: OutputConnection;
    
    fn create_virtual(
        self, port_name: &str
    ) -> Result<Self::Connection, ConnectError<Self>>;
}