extern crate winrt;

use std::sync::{Arc, Mutex};

use self::winrt::{RuntimeContext, ComPtr, HString, RtAsyncOperation, RtDefaultConstructible, IMemoryBufferByteAccess};
use self::winrt::windows::foundation::*;
use self::winrt::windows::devices::enumeration::*;
use self::winrt::windows::devices::midi::*;
use self::winrt::windows::storage::streams::*;

use ::errors::*;
use ::Ignore;

pub struct MidiInputPort {
    id: HString
}

unsafe impl Send for MidiInputPort {} // because HString doesn't ...

pub struct MidiInput {
    rt: RuntimeContext,
    selector: HString,
    ignore_flags: Ignore
}

unsafe impl Send for MidiInput {} // because HString doesn't ...

impl MidiInput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        let rt = RuntimeContext::init();
        let device_selector = MidiInPort::get_device_selector().map_err(|_| InitError)?;
        Ok(MidiInput { rt: rt, selector: device_selector, ignore_flags: Ignore::None })
    }

    pub fn ignore(&mut self, flags: Ignore) {
        self.ignore_flags = flags;
    }

    pub(crate) fn ports_internal(&self) -> Vec<::common::MidiInputPort> {
        let device_collection = DeviceInformation::find_all_async_aqs_filter(&self.selector.make_reference()).unwrap().blocking_get().expect("find_all_async failed").expect("find_all_async returned null");
        let count = device_collection.get_size().expect("get_size failed") as usize;
        let mut result = Vec::with_capacity(count as usize);
        for device_info in device_collection.into_iter() {
            let device_info = device_info.expect("device_info was null");
            let device_id = device_info.get_id().expect("get_id failed");
            result.push(::common::MidiInputPort {
                imp: MidiInputPort { id: device_id }
            });
        }
        result
    }
    
    pub fn port_count(&self) -> usize {
        let device_collection = DeviceInformation::find_all_async_aqs_filter(&self.selector.make_reference()).unwrap().blocking_get().expect("find_all_async failed").expect("find_all_async returned null");
        device_collection.get_size().expect("get_size failed") as usize
    }
    
    pub fn port_name(&self, port: &MidiInputPort) -> Result<String, PortInfoError> {
        let device_info_async = DeviceInformation::create_from_id_async(&port.id.make_reference()).map_err(|_| PortInfoError::InvalidPort)?;
        let device_info = device_info_async.blocking_get().map_err(|_| PortInfoError::InvalidPort)?.expect("device_info was null");
        let device_name = device_info.get_name().map_err(|_| PortInfoError::CannotRetrievePortName)?;
        Ok(device_name.to_string())
    }

    fn handle_input<T>(args: &MidiMessageReceivedEventArgs, handler_data: &mut HandlerData<T>) {
        let ignore = handler_data.ignore_flags;
        let data = &mut handler_data.user_data.as_mut().unwrap();
        let timestamp; 
        let byte_access;
        let message_bytes;
        unsafe {
            let message = args.get_message().expect("get_message failed").expect("get_message returned null");
            timestamp = message.get_timestamp().expect("get_timestamp failed").Duration as u64 / 10;
            let buffer = message.get_raw_data().expect("get_raw_data failed").expect("get_raw_data returned null");
            let membuffer = Buffer::create_memory_buffer_over_ibuffer(&buffer).expect("create_memory_buffer_over_ibuffer failed").expect("create_memory_buffer_over_ibuffer returned null");
            byte_access = membuffer.create_reference().expect("create_reference failed").expect("create_reference returned null").query_interface::<IMemoryBufferByteAccess>().unwrap();
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
        self, port: &MidiInputPort, _port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
        where F: FnMut(u64, &[u8], &mut T) + Send + 'static {
        
        let in_port = match MidiInPort::from_id_async(&port.id.make_reference()) {
            Ok(port_async) => match port_async.blocking_get() {
                Ok(Some(port)) => port,
                _ => return Err(ConnectError::new(ConnectErrorKind::InvalidPort, self))
            }
            Err(_) => return Err(ConnectError::new(ConnectErrorKind::InvalidPort, self))
        };
        
        let handler_data = Arc::new(Mutex::new(HandlerData {
            ignore_flags: self.ignore_flags,
            callback: Box::new(callback),
            user_data: Some(data)
        }));
        let handler_data2 = handler_data.clone();

        let handler = TypedEventHandler::new(move |_sender, args: *mut MidiMessageReceivedEventArgs| {
            unsafe { MidiInput::handle_input(&*args, &mut *handler_data2.lock().unwrap()) };
            Ok(())
        });
        
        let event_token = in_port.add_message_received(&handler).expect("add_message_received failed");

        Ok(MidiInputConnection { rt: self.rt, port: RtMidiInPort(in_port), event_token: event_token, handler_data: handler_data })
    }
}

struct RtMidiInPort(ComPtr<MidiInPort>);
unsafe impl Send for RtMidiInPort {}

pub struct MidiInputConnection<T> {
    rt: RuntimeContext,
    port: RtMidiInPort,
    event_token: EventRegistrationToken,
    // TODO: get rid of Arc & Mutex?
    //       synchronization is required because the borrow checker does not
    //       know that the callback we're in here is never called concurrently
    //       (always in sequence)
    handler_data: Arc<Mutex<HandlerData<T>>>
}


impl<T> MidiInputConnection<T> {
    pub fn close(self) -> (MidiInput, T) {
        let _ = self.port.0.remove_message_received(self.event_token);
        let _ = self.port.0.query_interface::<IClosable>().unwrap().close();
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

pub struct MidiOutputPort {
    id: HString
}

unsafe impl Send for MidiOutputPort {} // because HString doesn't ...

pub struct MidiOutput {
    rt: RuntimeContext,
    selector: HString // TODO: change to FastHString?
}

unsafe impl Send for MidiOutput {} // because HString doesn't ...

impl MidiOutput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        let rt = RuntimeContext::init();
        let device_selector = MidiOutPort::get_device_selector().map_err(|_| InitError)?;
        Ok(MidiOutput { rt: rt, selector: device_selector })
    }

    pub(crate) fn ports_internal(&self) -> Vec<::common::MidiOutputPort> {
        let device_collection = DeviceInformation::find_all_async_aqs_filter(&self.selector.make_reference()).unwrap().blocking_get().expect("find_all_async failed").expect("find_all_async returned null");
        let count = device_collection.get_size().expect("get_size failed") as usize;
        let mut result = Vec::with_capacity(count as usize);
        for device_info in device_collection.into_iter() {
            let device_info = device_info.expect("device_info was null");
            let device_id = device_info.get_id().expect("get_id failed");
            result.push(::common::MidiOutputPort {
                imp: MidiOutputPort { id: device_id }
            });
        }
        result
    }
    
    pub fn port_count(&self) -> usize {
        let device_collection = DeviceInformation::find_all_async_aqs_filter(&self.selector.make_reference()).unwrap().blocking_get().expect("find_all_async failed").expect("find_all_async returned null");
        device_collection.get_size().expect("get_size failed") as usize
    }
    
    pub fn port_name(&self, port: &MidiOutputPort) -> Result<String, PortInfoError> {
        let device_info_async = DeviceInformation::create_from_id_async(&port.id.make_reference()).map_err(|_| PortInfoError::InvalidPort)?;
        let device_info = device_info_async.blocking_get().map_err(|_| PortInfoError::InvalidPort)?.expect("device_info_async was null");
        let device_name = device_info.get_name().map_err(|_| PortInfoError::CannotRetrievePortName)?;
        Ok(device_name.to_string())
    }
    
    pub fn connect(self, port: &MidiOutputPort, _port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {        
        let out_port = match MidiOutPort::from_id_async(&port.id.make_reference()) {
            Ok(port_async) => match port_async.blocking_get() {
                Ok(Some(port)) => port,
                _ => return Err(ConnectError::new(ConnectErrorKind::InvalidPort, self))
            }
            Err(_) => return Err(ConnectError::new(ConnectErrorKind::InvalidPort, self))
        };
        Ok(MidiOutputConnection { rt: self.rt, port: out_port })
    }
}

pub struct MidiOutputConnection {
    rt: RuntimeContext,
    port: ComPtr<IMidiOutPort>
}

unsafe impl Send for MidiOutputConnection {}

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        let _ = self.port.query_interface::<IClosable>().unwrap().close();
        let device_selector = MidiOutPort::get_device_selector().expect("get_device_selector failed"); // probably won't ever fail here, because it worked previously
        MidiOutput { rt: self.rt, selector: device_selector }
    }
    
    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        let data_writer: ComPtr<DataWriter> = DataWriter::new();
        data_writer.write_bytes(message).map_err(|_| SendError::Other("write_bytes failed"))?;
        let buffer = data_writer.detach_buffer().map_err(|_| SendError::Other("detach_buffer failed"))?.expect("detach buffer returned null");
        self.port.send_buffer(&buffer).map_err(|_| SendError::Other("send_buffer failed"))?;
        Ok(())
    }
}