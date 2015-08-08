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

use super::Error::*;
use super::{Result, MidiApi, MidiInApi, MidiOutApi, MidiQueue, MidiMessage};

const RT_SYSEX_BUFFER_SIZE: usize = 1024;
const RT_SYSEX_BUFFER_COUNT: usize = 4;

// helpers for string conversion
fn from_wide_ptr<'a>(ptr: *const u16, max_len: usize) -> OsString {
    unsafe {
        assert!(!ptr.is_null());
        let len = (0..max_len as isize).position(|i| *ptr.offset(i) == 0).unwrap();
        let slice = slice::from_raw_parts(ptr, len);
        OsString::from_wide(slice)
    }
}

/*fn to_wide_chars(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    OsStr::new(s).encode_wide().chain(Some(0).into_iter()).collect::<Vec<_>>()
}*/

pub struct MidiInWinMM {
	connected: bool,
    handler_data: WinMMMidiHandlerData,
}

struct WinMMMidiHandlerData {
    first_message: bool,
    message: MidiMessage,
    last_time: u64, // TODO: combine first_message and last_time into single Option<>
    sysex_buffer: [LPMIDIHDR; RT_SYSEX_BUFFER_COUNT],
    in_handle: Option<Mutex<HMIDIIN>>, // TODO: get rid of this in favor of a single mutex
    shared: Mutex<SharedHandlerData>
}

struct SharedHandlerData {
    callback: Option<Box<FnMut(f64, &Vec<u8>)+Send>>,
    ignore_flags: u8,
    queue: MidiQueue,
}

extern "C" fn midi_input_callback(_: HMIDIIN,
                input_status: UINT, 
                instance_ptr: DWORD_PTR,
                midi_message: DWORD_PTR,
                timestamp: DWORD) {
    if input_status != MM_MIM_DATA && input_status != MM_MIM_LONGDATA && input_status != MM_MIM_LONGERROR { return; }
    
    let data: &mut WinMMMidiHandlerData = unsafe { mem::transmute(instance_ptr as *mut WinMMMidiHandlerData) };
    
    // Calculate time stamp.
    let timestamp = timestamp as u64;
    if data.first_message == true {
        data.message.timestamp = 0.0;
        data.first_message = false;
    }
    else {
        data.message.timestamp = (timestamp - data.last_time) as f64 * 0.001;
    }
    data.last_time = timestamp;
    
    let ignore_flags = data.shared.lock().unwrap().ignore_flags;
    
    if input_status == MM_MIM_DATA { // Channel or system message
        // Make sure the first byte is a status byte.
        let status: u8 = (midi_message & 0x000000FF) as u8;
        if !(status & 0x80 != 0) { return; }
        
        // Determine the number of bytes in the MIDI message.
        let nbytes: u16 = if status < 0xC0 { 3 }
        else if status < 0xE0 { 2 }
        else if status < 0xF0 { 3 }
        else if status == 0xF1 {
            if ignore_flags & 0x02 != 0 { return; }
            else  { 2 }
        } else if status == 0xF2 { 3 }
        else if status == 0xF3 { 2 }
        else if status == 0xF8 && (ignore_flags & 0x02 != 0) {
            // A MIDI timing tick message and we're ignoring it.
            return;
        } else if status == 0xFE && (ignore_flags & 0x04 != 0) {
            // A MIDI active sensing message and we're ignoring it.
            return;
        } else { 1 };
        
        // Copy bytes to our MIDI message.
        let ptr = (&midi_message) as *const u64 as *const u8;
        let bytes: &[u8] = unsafe { slice::from_raw_parts(ptr, nbytes as usize) };
        data.message.bytes.push_all(bytes);
    } else { // Sysex message (MIM_LONGDATA or MIM_LONGERROR)
        let sysex = unsafe { &*(midi_message as *const MIDIHDR) };
        if !(ignore_flags & 0x01 != 0) && input_status != MM_MIM_LONGERROR {
            // Sysex message and we're not ignoring it
            let bytes: &[u8] = unsafe { slice::from_raw_parts(sysex.lpData as *const u8, sysex.dwBytesRecorded as usize) };
            data.message.bytes.push_all(bytes);
            // TODO: If sysex messages are longer than RT_SYSEX_BUFFER_SIZE, they
            //       are split in chunks. We could reassemble a single message.
        }
    
        // The WinMM API requires that the sysex buffer be requeued after
        // input of each sysex message.  Even if we are ignoring sysex
        // messages, we still need to requeue the buffer in case the user
        // decides to not ignore sysex messages in the future.  However,
        // it seems that WinMM calls this function with an empty sysex
        // buffer when an application closes and in this case, we should
        // avoid requeueing it, else the computer suddenly reboots after
        // one or two minutes.
        if (unsafe {*data.sysex_buffer[sysex.dwUser as usize]}).dwBytesRecorded > 0 {
        //if ( sysex->dwBytesRecorded > 0 ) {
            let in_handle = data.in_handle.as_mut().unwrap().lock().unwrap();
            let result = unsafe { midiInAddBuffer(*in_handle, data.sysex_buffer[sysex.dwUser as usize], mem::size_of::<MIDIHDR>() as u32) };
            drop(in_handle);
            if result != MMSYSERR_NOERROR {
                let _ = write!(stderr(), "\nRtMidiIn::midiInputCallback: error sending sysex to Midi device!!\n\n");
            }
            
            if ignore_flags & 0x01 != 0 { return; }
        } else { return; }
    }
    
    let mut shared = data.shared.lock().unwrap();
    
    if shared.callback.is_some() {
        shared.callback.as_mut().unwrap()(data.message.timestamp, &data.message.bytes);
    } else {
        // As long as we haven't reached our queue size limit, push the message.
        let mut queue = &mut shared.queue;
        if queue.size < queue.ring.len() {
            // TODO: optimize so the message does not need to be cloned?
            queue.ring[queue.back as usize] = data.message.clone();
            queue.back += 1;
            if queue.back == queue.ring.len() {
                queue.back = 0;
            }
            queue.size += 1;
        }
        else {
            let _ = write!(stderr(), "\nMidiInAlsa: message queue limit reached!!\n\n");
        }
    }    
    
    // Clear the vector for the next input message.
    data.message.bytes.clear();
}

impl MidiApi for MidiInWinMM {
	fn get_port_count(&self) -> u32 {
        unsafe { midiInGetNumDevs() }
    }
    
    fn get_port_name(&self, port_number: u32 /*= 0*/) -> Result<String> {
        use std::fmt::Write;
        
        let ndevices = self.get_port_count();
        if port_number >= ndevices {
            use std::fmt::Write; 
            let mut error_string = String::new();
            let _ = write!(error_string, "MidiInWinMM::getPortName: the 'portNumber' argument ({}) is invalid.", port_number); 
            return Err(InvalidParameter(error_string));
        }
        
        let mut device_caps: MIDIINCAPSW = unsafe { mem::uninitialized() };
        unsafe { midiInGetDevCapsW(port_number as u64, &mut device_caps, mem::size_of::<MIDIINCAPSW>() as u32) };
        let mut output = from_wide_ptr(device_caps.szPname.as_ptr(), device_caps.szPname.len()).to_string_lossy().into_owned();
        
        // Next lines added to add the portNumber to the name so that 
        // the device's names are sure to be listed with individual names
        // even when they have the same brand name
        let _ = write!(&mut output, " {}", port_number);
        Ok(output)
    }
    
    fn open_port(&mut self, port_number: u32 /*= 0*/, _port_name: &str /*= "RtMidi"*/) -> Result<()> {
        if self.connected {
            let error_string = "MidiInWinMM::openPort: a valid connection already exists!";
            return Err(Warning(error_string));
        }
        
        let ndevices = self.get_port_count();
        if port_number >= ndevices {
            use std::fmt::Write; 
            let mut error_string = String::new();
            let _ = write!(error_string, "MidiInWinMM::openPort: the 'portNumber' argument ({}) is invalid.", port_number); 
            return Err(InvalidParameter(error_string));
        } 
        
        let mut in_handle: HMIDIIN = unsafe { mem::uninitialized() };
        let result = unsafe { midiInOpen(&mut in_handle,
                        port_number,
                        midi_input_callback as DWORD_PTR,
                        (&mut self.handler_data as *mut _) as DWORD_PTR,
                        CALLBACK_FUNCTION) };
        if result != MMSYSERR_NOERROR {
            let error_string = "MidiInWinMM::openPort: error creating Windows MM MIDI input port.";
            return Err(DriverError(error_string));
        }
        
        // Allocate and init the sysex buffers.
        for i in 0..RT_SYSEX_BUFFER_COUNT {                
            self.handler_data.sysex_buffer[i] = Box::into_raw(Box::new(MIDIHDR {
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
            
            // TODO: are those buffers ever freed if an error occurs here?
            
            let result = unsafe { midiInPrepareHeader(in_handle, self.handler_data.sysex_buffer[i], mem::size_of::<MIDIHDR>() as u32) };
            if result != MMSYSERR_NOERROR {
                unsafe { midiInClose(in_handle) };
                let error_string = "MidiInWinMM::openPort: error starting Windows MM MIDI input port (PrepareHeader).";
                return Err(DriverError(error_string));
            }
            
            // Register the buffer.
            let result = unsafe { midiInAddBuffer(in_handle, self.handler_data.sysex_buffer[i], mem::size_of::<MIDIHDR>() as u32) };
            if result != MMSYSERR_NOERROR {
                unsafe { midiInClose(in_handle) };
                let error_string = "MidiInWinMM::openPort: error starting Windows MM MIDI input port (AddBuffer).";
                return Err(DriverError(error_string));
            }
        }
        
        let result = unsafe { midiInStart(in_handle) };
        if result != MMSYSERR_NOERROR {
            unsafe { midiInClose(in_handle) };
            let error_string = "MidiInWinMM::openPort: error starting Windows MM MIDI input port.";
            return Err(DriverError(error_string));
        }
        
        self.handler_data.in_handle = Some(Mutex::new(in_handle));
        self.connected = true;
        Ok(())
    }
    
    //fn open_virtual_port(port_name: &str/*= "RtMidi"*/);

    fn close_port(&mut self) {
        if self.connected {
            // for information about his lock, see https://groups.google.com/forum/#!topic/mididev/6OUjHutMpEo
            let mut in_handle = self.handler_data.in_handle.take();
            let in_handle_lock = in_handle.as_mut().unwrap().lock().unwrap();
            
            // TODO: Call both reset and stop here? The difference seems to be that
            //       reset "returns all pending input buffers to the callback function"
            unsafe {
                midiInReset(*in_handle_lock);
                midiInStop(*in_handle_lock);
            }
            
            for i in 0..RT_SYSEX_BUFFER_COUNT {
                let _result;
                unsafe {
                    _result = midiInUnprepareHeader(*in_handle_lock, self.handler_data.sysex_buffer[i], mem::size_of::<MIDIHDR>() as u32);
                    heap::deallocate((*self.handler_data.sysex_buffer[i]).lpData as *mut u8, RT_SYSEX_BUFFER_SIZE, mem::align_of::<u8>());
                    heap::deallocate(self.handler_data.sysex_buffer[i] as *mut u8, mem::size_of::<MIDIHDR>(), mem::align_of::<MIDIHDR>());
                }
                
                // TODO: what to do if closing fails?
                /*if result != MMSYSERR_NOERROR {
                    midiInClose(*in_handle);
                    let error_string = "MidiInWinMM::closePort: error closing Windows MM MIDI input port (midiInUnprepareHeader).";
                    return Err(DriverError(error_string));
                }*/
            }
            
            unsafe { midiInClose(*in_handle_lock) };
            self.connected = false;
        }
    }
    
    fn is_port_open(&self) -> bool {
        unreachable!()
    }
}

impl MidiInApi for MidiInWinMM {
    fn new(_client_name: &str /*= "RtMidi Input Client"*/, queue_size_limit: usize /*= 100*/) -> Result<Self> {
        // We'll issue a warning here if no devices are available but not
        // throw an error since the user can plugin something later.
        /*unsigned int nDevices = midiInGetNumDevs();
        if ( nDevices == 0 ) {
        errorString_ = "MidiInWinMM::initialize: no MIDI input devices currently available.";
        error( RtMidiError::WARNING, errorString_ );
        }*/
        
        Ok(MidiInWinMM {
            connected: false,
            handler_data: WinMMMidiHandlerData {
                first_message: true,
                message: MidiMessage::new(),
                last_time: 0,
                sysex_buffer: unsafe { mem::uninitialized() }, // TODO!
                in_handle: None,
                shared: Mutex::new(SharedHandlerData {
                    callback: None,
                    ignore_flags: 7,
                    queue: MidiQueue::new(queue_size_limit)
                })
            }
        })
    }
    
    // TODO: get rid of code duplication among backends
    
    fn set_callback<F>(&mut self, callback: F) -> Result<()> where F: FnMut(f64, &Vec<u8>)+Send+'static {
        let mut previous = &mut self.handler_data.shared.lock().unwrap().callback;
        if previous.is_some() {
            let error_string = "MidiInApi::setCallback: a callback function is already set!";
            return Err(Warning(error_string));
        }
        
        *previous = Some(Box::new(callback));
        Ok(())
    }
    
    fn cancel_callback(&mut self) -> Result<()> {
        let mut previous = &mut self.handler_data.shared.lock().unwrap().callback;
        if !previous.is_some() {
            let error_string = "RtMidiIn::cancelCallback: no callback function was set!";
            return Err(Warning(error_string));
        }
      
        *previous = None;
        Ok(())
    }
    
    fn ignore_types(&mut self, sysex: bool /*= true*/, time: bool /*= true*/, active_sense: bool /*= true*/) {
        let mut flags = &mut self.handler_data.shared.lock().unwrap().ignore_flags;
        *flags = 0;
        if sysex { *flags = 0x01 };
        if time { *flags |= 0x02 };
        if active_sense { *flags |= 0x04 };
    }

    fn get_message(&mut self, message: &mut Vec<u8>) -> f64 {
        // If a callback is set, this function will return an empty message
        message.clear();
        let mut queue = &mut self.handler_data.shared.lock().unwrap().queue;
        if queue.size == 0 { return 0.0; }
    
        // Copy queued message to the vector pointer argument and then "pop" it.
        message.push_all(&queue.ring[queue.front].bytes[..]);
        let delta_time = queue.ring[queue.front].timestamp;
        queue.size -= 1;
        queue.front += 1;
        if queue.front == queue.ring.len() {
            queue.front = 0;
        }
    
        delta_time
    }
}

impl Drop for MidiInWinMM {
    fn drop(&mut self) {
        self.close_port();
    }
}

pub struct MidiOutWinMM {
	connected: bool,
    out_handle: Option<HMIDIOUT>,
}

impl MidiApi for MidiOutWinMM {
    fn get_port_count(&self) -> u32 {
        unsafe { midiOutGetNumDevs() }
    }
    
    fn get_port_name(&self, port_number: u32 /*= 0*/) -> Result<String> {
        use std::fmt::Write;
        
        let ndevices = self.get_port_count();
        if port_number >= ndevices {
            use std::fmt::Write; 
            let mut error_string = String::new();
            let _ = write!(error_string, "MidiInWinMM::getPortName: the 'portNumber' argument ({}) is invalid.", port_number); 
            return Err(InvalidParameter(error_string));
        }
        
        let mut device_caps: MIDIOUTCAPSW = unsafe { mem::uninitialized() };
        unsafe { midiOutGetDevCapsW(port_number as u64, &mut device_caps, mem::size_of::<MIDIOUTCAPSW>() as u32) };
        let mut output = from_wide_ptr(device_caps.szPname.as_ptr(), device_caps.szPname.len()).to_string_lossy().into_owned();
        
        // Next lines added to add the portNumber to the name so that 
        // the device's names are sure to be listed with individual names
        // even when they have the same brand name
        let _ = write!(&mut output, " {}", port_number);
        Ok(output)
    }
    
    fn open_port(&mut self, port_number: u32 /*= 0*/, _port_name: &str /*= "RtMidi"*/) -> Result<()> {
        if self.connected {
            let error_string = "MidiOutWinMM::openPort: a valid connection already exists!";
            return Err(Warning(error_string));
        }
       
        let ndevices = self.get_port_count();
        
        if port_number >= ndevices {
            use std::fmt::Write; 
            let mut error_string = String::new();
            let _ = write!(error_string, "MidiOutWinMM::openPort: the 'portNumber' argument ({}) is invalid.", port_number); 
            return Err(InvalidParameter(error_string));
        }
        
        let mut out_handle = unsafe { mem::uninitialized() };
        
        let result = unsafe { midiOutOpen(&mut out_handle, port_number, 0, 0, CALLBACK_NULL) };
        if result != MMSYSERR_NOERROR {
            let error_string = "MidiOutWinMM::openPort: error creating Windows MM MIDI output port.";
            return Err(DriverError(error_string));
        }
        
        self.connected = true;
        self.out_handle = Some(out_handle);
        Ok(())
    }
    
    fn close_port(&mut self) {
        if self.connected {
            let out_handle = self.out_handle.take().unwrap();
            unsafe {
                midiOutReset(out_handle);
                midiOutClose(out_handle);
            }
            self.connected = false;
        }
    }
    
    fn is_port_open(&self) -> bool {
        self.connected
    }
}

impl MidiOutApi for MidiOutWinMM {
    fn new(_client_name: &str /*= "RtMidi Output Client"*/) -> Result<Self> {
        Ok(MidiOutWinMM {
            connected: false,
            out_handle: None
        })
    }
    
    fn send_message(&mut self, message: &[u8]) -> Result<()> {
        // TODO: replace this with static guarantee (sending only possible on open port)
        if !self.connected { return Err(InvalidUse); }
        
        let nbytes = message.len();
        
        if nbytes == 0 {
            // TODO: probably replace with debug_assert
            let error_string = "MidiOutWinMM::sendMessage: message argument is empty!";
            return Err(Warning(error_string));
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
            let result = unsafe { midiOutPrepareHeader(*self.out_handle.as_ref().unwrap(), &mut sysex, mem::size_of::<MIDIHDR>() as u32) }; 
            if result != MMSYSERR_NOERROR {
                let error_string = "MidiOutWinMM::sendMessage: error preparing sysex header.";
                return Err(DriverError(error_string));
            }
            
            // Send the message.
            let result = unsafe { midiOutLongMsg(*self.out_handle.as_ref().unwrap(), &mut sysex, mem::size_of::<MIDIHDR>() as u32) };
            if result != MMSYSERR_NOERROR {
                let error_string = "MidiOutWinMM::sendMessage: error sending sysex message.";
                return Err(DriverError(error_string));
            }
        
            // Unprepare the buffer and MIDIHDR.
            while MIDIERR_STILLPLAYING == unsafe { midiOutUnprepareHeader(*self.out_handle.as_ref().unwrap(), &mut sysex, mem::size_of::<MIDIHDR>() as u32) } {
                sleep_ms(1);
            }
        } else { // Channel or system message.
            // Make sure the message size isn't too big.
            if nbytes > 3 {
                // TODO: change this into assert
                let error_string = "MidiOutWinMM::sendMessage: message size is greater than 3 bytes (and not sysex)!";
                return Err(Warning(error_string));
            }
            
            // Pack MIDI bytes into double word.
            let packet: DWORD = 0;
            let ptr = &packet as *const u32 as *mut u8;
            for i in 0..nbytes {
                unsafe { *ptr.offset(i as isize) = message[i] };
            }
            
            // Send the message immediately.
            let result = unsafe { midiOutShortMsg(*self.out_handle.as_ref().unwrap(), packet) };
            if result != MMSYSERR_NOERROR {
                let error_string = "MidiOutWinMM::sendMessage: error sending MIDI message.";
                return Err(DriverError(error_string));
            }
        }
        Ok(())
    }
}

impl Drop for MidiOutWinMM {
    fn drop(&mut self) {
        self.close_port();
    }
}