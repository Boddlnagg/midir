#![feature(vec_push_all, box_raw, heap_api)]

#[cfg(target_os="linux")] extern crate libc;
#[cfg(target_os="linux")] extern crate alsa_sys;
#[cfg(target_os="windows")] extern crate winapi;
#[cfg(target_os="windows")] extern crate winmm as winmm_sys;

use std::ops::BitOr;
use std::mem;

// TODO: use Cow<str> instead of String?
// TODO: get rid of unused error types
#[derive(Debug)]
pub enum Error {
    Warning(&'static str),        // A non-critical error.
    DebugWarning,                 // A non-critical error which might be useful for debugging.
    Unspecified,                  // The default, unspecified error type.
    NoDevicesFound(&'static str), // No devices found on system.
    InvalidDevice,                // An invalid device ID was specified.
    MemoryError,                  // An error occured during memory allocation.
    InvalidParameter(String),     // An invalid parameter was specified to a function.
    InvalidUse,                   // The function was called incorrectly.
    DriverError(&'static str),    // A system driver error occured.
    SystemError,                  // A system error occured.
    ThreadError(&'static str)     // A thread error occured.
}

pub type Result<T> = std::result::Result<T, Error>;

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum Ignore {
    None = 0x00,
    Sysex = 0x01,
    Time = 0x02,
    SysexAndTime = 0x03,
    ActiveSense = 0x04,
    SysexAndActiveSense = 0x05,
    TimeAndActiveSense = 0x06,
    All = 0x07
}

impl BitOr for Ignore {
    type Output = Ignore;
    #[inline(always)]
    fn bitor(self, rhs: Self) -> Self::Output {
        // this is safe because all combinations also exist as variants
        unsafe { mem::transmute(self as u8 | rhs as u8) }
    }
}

impl Ignore {
    #[inline(always)]
    pub fn contains(self, other: Ignore) -> bool {
        self as u8 & other as u8 != 0 
    }
}

pub trait MidiApi {
    fn get_port_count(&self) -> u32;
    fn get_port_name(&self, port_number: u32 /*= 0*/) -> Result<String>;
    fn open_port(&mut self, port_number: u32 /*= 0*/, port_name: &str /*= "RtMidi"*/) -> Result<()>;
    //fn open_virtual_port(port_name: &str/*= "RtMidi"*/);
    fn close_port(&mut self);
    fn is_port_open(&self) -> bool;
}

// TODO: create helper function that creates an instance (trait object)
//       of the correct system API

pub trait MidiInApi : MidiApi {
    fn new(client_name: &str /*= "RtMidi Input Client"*/, queue_size_limit: usize /*= 100*/) -> Result<Self>;
    fn set_callback<F>(&mut self, callback: F) -> Result<()> where F: FnMut(f64, &Vec<u8>)+Send;
    fn cancel_callback(&mut self) -> Result<()>;
    fn ignore_types(&mut self, sysex: bool /*= true*/, time: bool /*= true*/, active_sense: bool /*= true*/);

    /// Fill the user-provided vector with the data bytes for the next available
    /// MIDI message in  the input queue and return the event delta-time in seconds.
    /// 
    /// This function returns immediately whether a new message is
    /// available or not.  A valid message is indicated by a non-zero
    /// vector size.  An exception is thrown if an error occurs during
    /// message retrieval or an input connection was not previously
    /// established.
    fn get_message(&mut self, message: &mut Vec<u8>) -> f64;
}

/*pub struct MidiIn<Impl> where Impl: MidiInApi {
    inputData: MidiInData
}*/

// A MIDI structure used internally by the class to store incoming
// messages.  Each message represents one and only one MIDI message.
#[derive(Debug, Clone)]
struct MidiMessage {
    bytes: Vec<u8>,
    timestamp: f64
}

impl MidiMessage {
    // TODO: probably not needed
    pub fn new() -> MidiMessage {
        MidiMessage {
            bytes: vec![],
            timestamp: 0.0
        }
    }
}

#[derive(Debug)]
struct MidiQueue {
    front: usize,
    back: usize,
    size: usize,
    ring: Box<[MidiMessage]>
}

impl MidiQueue {
    pub fn new(ring_size: usize) -> MidiQueue {
        MidiQueue {
            front: 0,
            back: 0,
            size: 0,
            ring: unsafe {
                let mut vec = Vec::with_capacity(ring_size);
                vec.set_len(ring_size);
                vec.into_boxed_slice()
            }
        }
    }
}


pub trait MidiOutApi : MidiApi {
    fn new(client_name: &str /*= "RtMidi Output Client"*/) -> Result<Self>;
    fn send_message(&mut self, message: &[u8]) -> Result<()>;
}

// TODO: allow feature selection (ALSA and/or Jack)
#[cfg(target_os="linux")]
pub mod alsa;

#[cfg(target_os="windows")]
pub mod winmm;