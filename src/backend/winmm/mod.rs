extern crate winapi;
extern crate winmm as winmm_sys;

use std::{mem, ptr, slice};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::Mutex;
use std::io::{stderr, Write};
use std::thread::sleep;
use std::time::Duration;
use memalloc::{allocate, deallocate};

use self::winapi::*;

use self::winmm_sys::{
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

use ::{MidiMessage, Ignore};
use ::errors::*;

mod handler;

const RT_SYSEX_BUFFER_SIZE: usize = 1024;
const RT_SYSEX_BUFFER_COUNT: usize = 4;

// helper for string conversion
fn from_wide_ptr(ptr: *const u16, max_len: usize) -> OsString {
    unsafe {
        assert!(!ptr.is_null());
        let len = (0..max_len as isize).position(|i| *ptr.offset(i) == 0).unwrap();
        let slice = slice::from_raw_parts(ptr, len);
        OsString::from_wide(slice)
    }
}

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
    sysex_buffer: [LPMIDIHDR; RT_SYSEX_BUFFER_COUNT],
    in_handle: Option<Mutex<HMIDIIN>>,
    ignore_flags: Ignore,
    callback: Box<FnMut(u64, &[u8], &mut T) + Send + 'static>,
    user_data: Option<T>
}

impl MidiInput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        Ok(MidiInput { ignore_flags: Ignore::None })
    }
    
    pub fn ignore(&mut self, flags: Ignore) {
        self.ignore_flags = flags;
    }
    
    pub fn port_count(&self) -> usize {
        unsafe { midiInGetNumDevs() as usize }
    }
    
    pub fn port_name(&self, port_number: usize) -> Result<String, PortInfoError> {
        let mut device_caps: MIDIINCAPSW = unsafe { mem::uninitialized() };
        let result = unsafe { midiInGetDevCapsW(port_number as UINT_PTR, &mut device_caps, mem::size_of::<MIDIINCAPSW>() as u32) };
        if result == MMSYSERR_BADDEVICEID {
            return Err(PortInfoError::PortNumberOutOfRange)
        } else if result != MMSYSERR_NOERROR {
            return Err(PortInfoError::CannotRetrievePortName)
        }
        let output = from_wide_ptr(device_caps.szPname.as_ptr(), device_caps.szPname.len()).to_string_lossy().into_owned();
        Ok(output)
    }
    
    pub fn connect<F, T: Send>(
        self, port_number: usize, _port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
        where F: FnMut(u64, &[u8], &mut T) + Send + 'static {
        
        let mut handler_data = Box::new(HandlerData {
            message: MidiMessage::new(),
            sysex_buffer: unsafe { mem::uninitialized() },
            in_handle: None,
            ignore_flags: self.ignore_flags,
            callback: Box::new(callback),
            user_data: Some(data)
        });
        
        let mut in_handle: HMIDIIN = unsafe { mem::uninitialized() };
        let handler_data_ptr: *mut HandlerData<T> = &mut *handler_data;
        let result = unsafe { midiInOpen(&mut in_handle,
                        port_number as UINT,
                        handler::handle_input::<T> as DWORD_PTR,
                        handler_data_ptr as DWORD_PTR,
                        CALLBACK_FUNCTION) };
        if result == MMSYSERR_BADDEVICEID {
            return Err(ConnectError::new(ConnectErrorKind::PortNumberOutOfRange, self));
        } else if result != MMSYSERR_NOERROR {
            return Err(ConnectError::other("could not create Windows MM MIDI input port", self));
        }
        
        // Allocate and init the sysex buffers.
        for i in 0..RT_SYSEX_BUFFER_COUNT {
            handler_data.sysex_buffer[i] = Box::into_raw(Box::new(MIDIHDR {
                lpData: unsafe { allocate(RT_SYSEX_BUFFER_SIZE/*, mem::align_of::<u8>()*/) } as *mut i8,
                dwBufferLength: RT_SYSEX_BUFFER_SIZE as u32,
                dwBytesRecorded: 0,
                dwUser: i as DWORD_PTR, // We use the dwUser parameter as buffer indicator
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
                deallocate((*self.handler_data.sysex_buffer[i]).lpData as *mut u8, RT_SYSEX_BUFFER_SIZE/*, mem::align_of::<u8>()*/);
                // recreate the Box so that it will be dropped/deallocated at the end of this scope
                let _ = Box::from_raw(self.handler_data.sysex_buffer[i]);
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

#[derive(Debug)]
pub struct MidiOutput;

pub struct MidiOutputConnection {
    out_handle: HMIDIOUT,
}

impl MidiOutput {
    pub fn new(_client_name: &str) -> Result<Self, InitError> {
        Ok(MidiOutput)
    }
    
    pub fn port_count(&self) -> usize {
        unsafe { midiOutGetNumDevs() as usize }
    }
    
    pub fn port_name(&self, port_number: usize) -> Result<String, PortInfoError> {
        let mut device_caps: MIDIOUTCAPSW = unsafe { mem::uninitialized() };
        let result = unsafe { midiOutGetDevCapsW(port_number as UINT_PTR, &mut device_caps, mem::size_of::<MIDIINCAPSW>() as u32) };
        if result == MMSYSERR_BADDEVICEID {
            return Err(PortInfoError::PortNumberOutOfRange)
        } else if result != MMSYSERR_NOERROR {
            return Err(PortInfoError::CannotRetrievePortName)
        }
        let output = from_wide_ptr(device_caps.szPname.as_ptr(), device_caps.szPname.len()).to_string_lossy().into_owned();
        Ok(output)
    }
    
    pub fn connect(self, port_number: usize, _port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        let mut out_handle = unsafe { mem::uninitialized() };
        
        let result = unsafe { midiOutOpen(&mut out_handle, port_number as UINT, 0, 0, CALLBACK_NULL) };
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

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        // The actual closing is done by the implementation of Drop
        MidiOutput // In this API this is a noop
    }
    
    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
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
                    sleep(Duration::from_millis(1));
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
                    sleep(Duration::from_millis(1));
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
                    sleep(Duration::from_millis(1));
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