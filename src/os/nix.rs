use ::ConnectError;
use ::{MidiInputConnection, MidiOutputConnection};

pub trait VirtualInput<T: Send> where Self: Sized {
    fn create_virtual<F>(
        self, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<Self>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static;
}

pub trait VirtualOutput where Self: Sized {
    fn create_virtual(
        self, port_name: &str
    ) -> Result<MidiOutputConnection, ConnectError<Self>>;
}