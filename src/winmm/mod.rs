use std::{mem, ptr, slice};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::{Mutex};
use std::io::{stderr, Write};
use std::rt::heap;
use std::thread::sleep_ms;

use winapi::*;

use winmm_sys::{
	midiInGetNumDevs,
    midiInGetDevCapsW,
    midiInOpen,
    midiInStart,
    midiInClose,
    midiInReset,
    midiInStop,
    midiInAddBuffer,
    midiInPrepareHeader,
    midiInUnprepareHeader,
    midiOutGetNumDevs,
    midiOutGetDevCapsW,
    midiOutOpen,
    midiOutReset,
    midiOutClose,
    midiOutPrepareHeader,
    midiOutUnprepareHeader,
    midiOutLongMsg,
    midiOutShortMsg,
};

use super::{MidiMessage, Ignore};
use super::{InitError, PortInfoError, ConnectErrorKind, ConnectError, SendError};
use super::traits::*;

mod handler;

const RT_SYSEX_BUFFER_SIZE: usize = 1024;
const RT_SYSEX_BUFFER_COUNT: usize = 4;

// helper for string conversion
fn from_wide_ptr<'a>(ptr: *const u16, max_len: usize) -> OsString {
    unsafe {
        assert!(!ptr.is_null());
        let len = (0..max_len as isize).position(|i| *ptr.offset(i) == 0).unwrap();
        let slice = slice::from_raw_parts(ptr, len);
        OsString::from_wide(slice)
    }
}

// TODO: make sure that these structs are all `Send`

#[derive(Debug)]
pub struct MidiInput {
    ignore_flags: Ignore
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
    message: MidiMessage,
    last_time: Option<u64>,
    sysex_buffer: [LPMIDIHDR; RT_SYSEX_BUFFER_COUNT],
    in_handle: Option<Mutex<HMIDIIN>>,
    ignore_flags: Ignore,
    callback: Box<FnMut(f64, &[u8], &mut T)+Send>,
    user_data: Option<T>
}

impl MidiInput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        Ok(MidiInput { ignore_flags: Ignore::None })
    }
    
    pub fn ignore(&mut self, flags: Ignore) {
        self.ignore_flags = flags;
    }
    
	pub fn port_count(&self) -> u32 {
        unsafe { midiInGetNumDevs() }
    }
    
    pub fn port_name(&self, port_number: u32) -> Result<String, PortInfoError> {
        use std::fmt::Write;
        
        let mut device_caps: MIDIINCAPSW = unsafe { mem::uninitialized() };
        let result = unsafe { midiInGetDevCapsW(port_number as u64, &mut device_caps, mem::size_of::<MIDIINCAPSW>() as u32) };
        if result == MMSYSERR_BADDEVICEID {
            return Err(PortInfoError::PortNumberOutOfRange)
        }
        assert!(result == MMSYSERR_NOERROR, "could not retrieve Windows MM MIDI input port name");
        let mut output = from_wide_ptr(device_caps.szPname.as_ptr(), device_caps.szPname.len()).to_string_lossy().into_owned();
        
        // Next lines added to add the portNumber to the name so that 
        // the device's names are sure to be listed with individual names
        // even when they have the same brand name
        let _ = write!(&mut output, " {}", port_number);
        Ok(output)
    }
    
    pub fn connect<F, T: Send>(
        self, port_number: u32, _port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
        where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        
        let mut handler_data = Box::new(HandlerData {
            message: MidiMessage::new(),
            last_time: None,
            sysex_buffer: unsafe { mem::uninitialized() },
            in_handle: None,
            ignore_flags: self.ignore_flags,
            callback: Box::new(callback),
            user_data: Some(data)
        });
        
        let mut in_handle: HMIDIIN = unsafe { mem::uninitialized() };
        let result = unsafe { midiInOpen(&mut in_handle,
                        port_number,
                        handler::handle_input::<T> as DWORD_PTR,
                        mem::transmute_copy::<_, *mut HandlerData<T>>(&handler_data) as DWORD_PTR,
                        CALLBACK_FUNCTION) };
        if result == MMSYSERR_BADDEVICEID {
            return Err(ConnectError::new(ConnectErrorKind::PortNumberOutOfRange, self));
        } else if result != MMSYSERR_NOERROR {
            return Err(ConnectError::other("could not create Windows MM MIDI input port", self));
        }
        
        // Allocate and init the sysex buffers.
        for i in 0..RT_SYSEX_BUFFER_COUNT {
            handler_data.sysex_buffer[i] = Box::into_raw(Box::new(MIDIHDR {
                lpData: unsafe { heap::allocate(RT_SYSEX_BUFFER_SIZE, mem::align_of::<u8>()) } as *mut i8,
                dwBufferLength: RT_SYSEX_BUFFER_SIZE as u32,
                dwBytesRecorded: 0,
                dwUser: i as u64, // We use the dwUser parameter as buffer indicator
                dwFlags: 0,
                lpNext: ptr::null_mut(),
                reserved: 0,
                dwOffset: 0,
                dwReserved: [0; 4],
            }));
            
            // TODO: are those buffers ever freed if an error occurs here (altough these calls probably only fail with out-of-memory)?
            // TODO: close port in case of error?
            
            let result = unsafe { midiInPrepareHeader(in_handle, handler_data.sysex_buffer[i], mem::size_of::<MIDIHDR>() as u32) };
            if result != MMSYSERR_NOERROR {
                return Err(ConnectError::other("could not initialize Windows MM MIDI input port (PrepareHeader)", self));
            }
            
            // Register the buffer.
            let result = unsafe { midiInAddBuffer(in_handle, handler_data.sysex_buffer[i], mem::size_of::<MIDIHDR>() as u32) };
            if result != MMSYSERR_NOERROR {
                return Err(ConnectError::other("could not initialize Windows MM MIDI input port (AddBuffer)", self));
            }            
        }
        
        handler_data.in_handle = Some(Mutex::new(in_handle));
        
        // We can safely access (a copy of) `in_handle` here, although
        // it has been copied into the Mutex already, because the callback
        // has not been called yet.
        let result = unsafe { midiInStart(in_handle) };
        if result != MMSYSERR_NOERROR {
            unsafe { midiInClose(in_handle) };
            return Err(ConnectError::other("could not start Windows MM MIDI input port", self));
        }
        
        Ok(MidiInputConnection {
            handler_data: handler_data
        })
    }
}

impl PortInfo for MidiInput {
    fn new(client_name: &str) -> Result<Self, InitError> {
        Self::new(client_name)
    }
    
    fn port_count(&self) -> u32 {
        self.port_count()
    }
    
    fn port_name(&self, port_number: u32) -> Result<String, PortInfoError> {
        self.port_name(port_number)
    }
}

impl<T: Send> InputConnect<T> for MidiInput {
    type Connection = MidiInputConnection<T>; 
    
    fn connect<F>(
        self, port_number: u32, port_name: &str, callback: F, data: T
    ) -> Result<Self::Connection, ConnectError<Self>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        self.connect(port_number, port_name, callback, data)
    }
}

impl<T> MidiInputConnection<T> {
    pub fn close(mut self) -> (MidiInput, T) {
        self.close_internal();
        
        (MidiInput {
            ignore_flags: self.handler_data.ignore_flags,
        }, self.handler_data.user_data.take().unwrap())
    }
    
    fn close_internal(&mut self) {
        // for information about his lock, see https://groups.google.com/forum/#!topic/mididev/6OUjHutMpEo
        let in_handle_lock = self.handler_data.in_handle.as_ref().unwrap().lock().unwrap();
        
        // TODO: Call both reset and stop here? The difference seems to be that
        //       reset "returns all pending input buffers to the callback function"
        unsafe {
            midiInReset(*in_handle_lock);
            midiInStop(*in_handle_lock);
        }
        
        for i in 0..RT_SYSEX_BUFFER_COUNT {
            let result;
            unsafe {
                result = midiInUnprepareHeader(*in_handle_lock, self.handler_data.sysex_buffer[i], mem::size_of::<MIDIHDR>() as u32);
                heap::deallocate((*self.handler_data.sysex_buffer[i]).lpData as *mut u8, RT_SYSEX_BUFFER_SIZE, mem::align_of::<u8>());
                heap::deallocate(self.handler_data.sysex_buffer[i] as *mut u8, mem::size_of::<MIDIHDR>(), mem::align_of::<MIDIHDR>());
            }
            
            if result != MMSYSERR_NOERROR {
                let _ = writeln!(stderr(), "Warning: Ignoring error shutting down Windows MM input port (UnprepareHeader).");
            }
        }
        
        unsafe { midiInClose(*in_handle_lock) };
    }
}

impl<T> Drop for MidiInputConnection<T> {
    fn drop(&mut self) {
        // If user_data has been emptied, we know that we already have closed the connection
        if self.handler_data.user_data.is_some() {
            self.close_internal()
        }
    }
}

impl<T> InputConnection<T> for MidiInputConnection<T> {
    type Input = MidiInput;
    
    fn close(self) -> (Self::Input, T) {
        self.close()
    }
}

#[derive(Debug)]
pub struct MidiOutput;

pub struct MidiOutputConnection {
    out_handle: HMIDIOUT,
}

impl MidiOutput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        Ok(MidiOutput)
    }
    
	pub fn port_count(&self) -> u32 {
        unsafe { midiOutGetNumDevs() }
    }
    
    pub fn port_name(&self, port_number: u32) -> Result<String, PortInfoError> {
        use std::fmt::Write;
        
        let mut device_caps: MIDIOUTCAPSW = unsafe { mem::uninitialized() };
        let result = unsafe { midiOutGetDevCapsW(port_number as u64, &mut device_caps, mem::size_of::<MIDIINCAPSW>() as u32) };
        if result == MMSYSERR_BADDEVICEID {
            return Err(PortInfoError::PortNumberOutOfRange)
        }
        assert!(result == MMSYSERR_NOERROR, "could not retrieve Windows MM MIDI output port name");
        let mut output = from_wide_ptr(device_caps.szPname.as_ptr(), device_caps.szPname.len()).to_string_lossy().into_owned();
        
        // Next lines added to add the portNumber to the name so that 
        // the device's names are sure to be listed with individual names
        // even when they have the same brand name
        let _ = write!(&mut output, " {}", port_number);
        Ok(output)
    }
    
    pub fn connect(self, port_number: u32, _port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        let mut out_handle = unsafe { mem::uninitialized() };
        
        let result = unsafe { midiOutOpen(&mut out_handle, port_number, 0, 0, CALLBACK_NULL) };
        if result == MMSYSERR_BADDEVICEID {
            return Err(ConnectError::new(ConnectErrorKind::PortNumberOutOfRange, self));
        } else if result != MMSYSERR_NOERROR {
            return Err(ConnectError::other("could not create Windows MM MIDI output port", self));
        }
        
        Ok(MidiOutputConnection {
            out_handle: out_handle
        })
    }
}

impl PortInfo for MidiOutput {
    fn new(client_name: &str) -> Result<Self, InitError> {
        Self::new(client_name)
    }
    
    fn port_count(&self) -> u32 {
        self.port_count()
    }
    
    fn port_name(&self, port_number: u32) -> Result<String, PortInfoError> {
        self.port_name(port_number)
    }
}

impl OutputConnect for MidiOutput {
    type Connection = MidiOutputConnection; 
    
     fn connect(
        self, port_number: u32, port_name: &str
    ) -> Result<Self::Connection, super::ConnectError<Self>> {
        self.connect(port_number, port_name)
    }
}

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        // The actual closing is done by the implementation of Drop
        MidiOutput // In this API this is a noop
    }
    
    /// This will panic if the message is not a valid MIDI message.
    pub fn send_message(&mut self, message: &[u8]) -> Result<(), SendError> {
        let nbytes = message.len();
        if nbytes == 0 {
            return Err(SendError::InvalidData("message to be sent must not be empty"));
        }
        
        if message[0] == 0xF0 { // Sysex message
            // Allocate buffer for sysex data and copy message
            let mut buffer = message.to_vec();
        
            // Create and prepare MIDIHDR structure.
            let mut sysex = MIDIHDR {
                lpData: buffer.as_mut_ptr() as *mut i8,
                dwBufferLength: nbytes as u32,
                dwBytesRecorded: 0,
                dwUser: 0,
                dwFlags: 0,
                lpNext: ptr::null_mut(),
                reserved: 0,
                dwOffset: 0,
                dwReserved: [0; 4],
            };
            
            let result = unsafe { midiOutPrepareHeader(self.out_handle, &mut sysex, mem::size_of::<MIDIHDR>() as u32) };
            
            if result != MMSYSERR_NOERROR {
                return Err(SendError::Other("preparation for sending sysex message failed (OutPrepareHeader)"));
            }
            
            // Send the message.
            loop {
                let result = unsafe { midiOutLongMsg(self.out_handle, &mut sysex, mem::size_of::<MIDIHDR>() as u32) };
                if result == MIDIERR_NOTREADY {
                    sleep_ms(1);
                    continue;
                } else {
                    if result != MMSYSERR_NOERROR {
                        return Err(SendError::Other("sending sysex message failed"));
                    }
                    break;
                }
            }
            
            loop {
                let result = unsafe { midiOutUnprepareHeader(self.out_handle, &mut sysex, mem::size_of::<MIDIHDR>() as u32) };
                if result == MIDIERR_STILLPLAYING {
                    sleep_ms(1);
                    continue;
                } else { break; }
            }
        } else { // Channel or system message.
            // Make sure the message size isn't too big.
            if nbytes > 3 {
                return Err(SendError::InvalidData("non-sysex message must not be longer than 3 bytes"));
            }
            
            // Pack MIDI bytes into double word.
            let packet: DWORD = 0;
            let ptr = &packet as *const u32 as *mut u8;
            for i in 0..nbytes {
                unsafe { *ptr.offset(i as isize) = message[i] };
            }
            
            // Send the message immediately.
            loop {
                let result = unsafe { midiOutShortMsg(self.out_handle, packet) };
                if result == MIDIERR_NOTREADY {
                    sleep_ms(1);
                    continue;
                } else {
                    if result != MMSYSERR_NOERROR {
                        return Err(SendError::Other("sending non-sysex message failed"));
                    }
                    break;
                }
            }
        }
        
        Ok(())
    }
}

impl Drop for MidiOutputConnection {
    fn drop(&mut self) {
        unsafe {
            midiOutReset(self.out_handle);
            midiOutClose(self.out_handle);
        }
    }
}

impl OutputConnection for MidiOutputConnection {
    type Output = MidiOutput;
    
    fn close(self) -> Self::Output {
        self.close()
    }
    
    fn send_message(&mut self, message: &[u8]) -> Result<(), SendError> {
        self.send_message(message)
    }   
}