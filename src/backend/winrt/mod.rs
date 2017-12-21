extern crate winrt;

use std::sync::{Arc, Mutex};

use self::winrt::{RuntimeContext, ComPtr, HString, RtAsyncOperation, RtDefaultConstructible, IMemoryBufferByteAccess};
use self::winrt::windows::foundation::*;
use self::winrt::windows::devices::enumeration::*;
use self::winrt::windows::devices::midi::*;
use self::winrt::windows::storage::streams::*;

use ::errors::*;
use ::Ignore;

pub struct MidiInput {
    rt: RuntimeContext,
    selector: HString,
    ignore_flags: Ignore
}

impl MidiInput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        let rt = RuntimeContext::init();
        let device_selector = MidiInPort::get_device_selector().map_err(|_| InitError)?;
        Ok(MidiInput { rt: rt, selector: device_selector, ignore_flags: Ignore::None })
    }

    pub fn ignore(&mut self, flags: Ignore) {
        self.ignore_flags = flags;
    }
    
    pub fn port_count(&self) -> usize {
        let device_collection = DeviceInformation::find_all_async_aqs_filter(&self.selector.make_reference()).expect("find_all_async failed").blocking_get();
        unsafe { device_collection.get_size().expect("get_size failed") as usize }
    }
    
    pub fn port_name(&self, port_number: usize) -> Result<String, PortInfoError> {
        let device_collection = DeviceInformation::find_all_async_aqs_filter(&self.selector.make_reference()).expect("find_all_async failed").blocking_get();
        let device_name;
        unsafe {
            let device_info = device_collection.get_at(port_number as u32).map_err(|_| PortInfoError::PortNumberOutOfRange)?;
            device_name = device_info.get_name().map_err(|_| PortInfoError::CannotRetrievePortName)?;
        }
        Ok(device_name.to_string())
    }

    fn handle_input<T>(args: &MidiMessageReceivedEventArgs, handler_data: &mut HandlerData<T>) {
        let ignore = handler_data.ignore_flags;
        let data = &mut handler_data.user_data.as_mut().unwrap();
        let timestamp; 
        let byte_access;
        let message_bytes;
        unsafe {
            let message = args.get_message().expect("get_message failed");
            timestamp = message.get_timestamp().expect("get_timestamp failed").Duration as u64 / 10;
            let buffer = message.get_raw_data().expect("get_raw_data failed");
            let membuffer = Buffer::create_memory_buffer_over_ibuffer(&buffer).expect("create_memory_buffer_over_ibuffer failed");
            byte_access = membuffer.create_reference().expect("create_reference failed").query_interface::<IMemoryBufferByteAccess>().unwrap();
            message_bytes = byte_access.get_buffer();
        }

        // The first byte in the message is the status
        let status = message_bytes[0];

        if !(status == 0xF0 && ignore.contains(Ignore::Sysex) ||
             status == 0xF1 && ignore.contains(Ignore::Time) ||
             status == 0xF8 && ignore.contains(Ignore::Time) ||
             status == 0xFE && ignore.contains(Ignore::ActiveSense))
        {
            (handler_data.callback)(timestamp, message_bytes, data);
        }
    }

    pub fn connect<F, T: Send + 'static>(
        self, port_number: usize, _port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
        where F: FnMut(u64, &[u8], &mut T) + Send + 'static {

        let device_collection = DeviceInformation::find_all_async_aqs_filter(&self.selector.make_reference()).expect("find_all_async failed").blocking_get();
        unsafe {
            let device_info = match device_collection.get_at(port_number as u32) {
                Ok(info) => info,
                Err(_) => return Err(ConnectError::new(ConnectErrorKind::PortNumberOutOfRange, self))
            };
            let device_id = match device_info.get_id() {
                Ok(id) => id,
                Err(_) => return Err(ConnectError::other("get_id failed", self))
            };
            let in_port = match MidiInPort::from_id_async(&device_id.make_reference()) {
                Ok(port) => port.blocking_get(),
                Err(_) => return Err(ConnectError::other("MidiInPort::from_id_async failed", self))
            };

            let handler_data = Arc::new(Mutex::new(HandlerData {
                ignore_flags: self.ignore_flags,
                callback: Box::new(callback),
                user_data: Some(data)
            }));
            let handler_data2 = handler_data.clone();

            let handler = TypedEventHandler::new(move |_sender, args: *mut MidiMessageReceivedEventArgs| {
                MidiInput::handle_input(&*args, &mut *handler_data2.lock().unwrap());
                Ok(())
            });
            let event_token = in_port.add_message_received(&handler).expect("add_message_received failed");

            Ok(MidiInputConnection { rt: self.rt, port: in_port, event_token: event_token, handler_data: handler_data })
        }
    }
}

pub struct MidiInputConnection<T> {
    rt: RuntimeContext,
    port: ComPtr<MidiInPort>,
    event_token: EventRegistrationToken,
    // TODO: get rid of Arc & Mutex?
    //       synchronization is required because the borrow checker does not
    //       know that the callback we're in here is never called concurrently
    //       (always in sequence)
    handler_data: Arc<Mutex<HandlerData<T>>>
}

impl<T> MidiInputConnection<T> {
    pub fn close(self) -> (MidiInput, T) {
        let _ = unsafe { self.port.remove_message_received(self.event_token) };
        let _ = unsafe { self.port.query_interface::<IClosable>().unwrap().close() };
        let device_selector = MidiInPort::get_device_selector().expect("get_device_selector failed"); // probably won't ever fail here, because it worked previously
        let mut handler_data_locked = self.handler_data.lock().unwrap();
        (MidiInput {
            rt: self.rt,
            selector: device_selector,
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
    ignore_flags: Ignore,
    callback: Box<FnMut(u64, &[u8], &mut T) + Send>,
    user_data: Option<T>
}

pub struct MidiOutput {
    rt: RuntimeContext,
    selector: HString // TODO: change to FastHString?
}

impl MidiOutput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        let rt = RuntimeContext::init();
        let device_selector = MidiOutPort::get_device_selector().map_err(|_| InitError)?;
        Ok(MidiOutput { rt: rt, selector: device_selector })
    }
    
    pub fn port_count(&self) -> usize {
        let device_collection = DeviceInformation::find_all_async_aqs_filter(&self.selector.make_reference()).expect("find_all_async failed").blocking_get();
        unsafe { device_collection.get_size().expect("get_size failed") as usize }
    }
    
    pub fn port_name(&self, port_number: usize) -> Result<String, PortInfoError> {
        let device_collection = DeviceInformation::find_all_async_aqs_filter(&self.selector.make_reference()).expect("find_all_async failed").blocking_get();
        let device_name;
        unsafe {
            let device_info = device_collection.get_at(port_number as u32).map_err(|_| PortInfoError::PortNumberOutOfRange)?;
            device_name = device_info.get_name().map_err(|_| PortInfoError::CannotRetrievePortName)?;
        }
        Ok(device_name.to_string())
    }
    
    pub fn connect(self, port_number: usize, _port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        let device_collection = DeviceInformation::find_all_async_aqs_filter(&self.selector.make_reference()).expect("find_all_async failed").blocking_get();
        unsafe {
            let device_info = match device_collection.get_at(port_number as u32) {
                Ok(info) => info,
                Err(_) => return Err(ConnectError::new(ConnectErrorKind::PortNumberOutOfRange, self))
            };
            let device_id = match device_info.get_id() {
                Ok(id) => id,
                Err(_) => return Err(ConnectError::other("get_id failed", self))
            };
            let out_port = match MidiOutPort::from_id_async(&device_id.make_reference()) {
                Ok(port) => port.blocking_get(),
                Err(_) => return Err(ConnectError::other("MidiOutPort::from_id_async failed", self))
            };
            Ok(MidiOutputConnection { rt: self.rt, port: out_port })
        }
    }
}

pub struct MidiOutputConnection {
    rt: RuntimeContext,
    port: ComPtr<IMidiOutPort>
}

unsafe impl Send for MidiOutputConnection {}

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        let _ = unsafe { self.port.query_interface::<IClosable>().unwrap().close() };
        let device_selector = MidiOutPort::get_device_selector().expect("get_device_selector failed"); // probably won't ever fail here, because it worked previously
        MidiOutput { rt: self.rt, selector: device_selector }
    }
    
    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        let data_writer: ComPtr<DataWriter> = DataWriter::new();
        unsafe {
            data_writer.write_bytes(message).map_err(|_| SendError::Other("write_bytes failed"))?;
            let buffer = data_writer.detach_buffer().map_err(|_| SendError::Other("detach_buffer failed"))?;
            self.port.send_buffer(&buffer).map_err(|_| SendError::Other("send_buffer failed"))?;
        }
        Ok(())
    }
}