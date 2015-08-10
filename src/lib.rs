#![feature(vec_push_all, box_raw, heap_api)]

#[cfg(target_os="linux")] extern crate libc;
#[cfg(target_os="linux")] extern crate alsa_sys;
#[cfg(target_os="windows")] extern crate winapi;
#[cfg(target_os="windows")] extern crate winmm as winmm_sys;

use std::ops::BitOr;
use std::mem;
use std::fmt;
use std::error::Error;

const PORT_OUT_OF_RANGE_MSG: &'static str = "provided port number was out of range";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitError;

impl Error for InitError {
    fn description(&self) -> &str {
        "MIDI support could not be initialized"
    }
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.description().fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortInfoError {
    PortNumberOutOfRange,
}

impl Error for PortInfoError {
    fn description(&self) -> &str {
        match *self {
            PortInfoError::PortNumberOutOfRange => PORT_OUT_OF_RANGE_MSG,
        }
    }
}

impl fmt::Display for PortInfoError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.description().fmt(f)
    }
}

// TODO: implement (not derive) Debug, Show, ... without using inner T

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectErrorKind {
    PortNumberOutOfRange,
    Other(&'static str)
}

impl Error for ConnectErrorKind {
    fn description(&self) -> &str {
        match *self {
            ConnectErrorKind::PortNumberOutOfRange => PORT_OUT_OF_RANGE_MSG,
            ConnectErrorKind::Other(msg) => msg
        }
    }
}

impl fmt::Display for ConnectErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.description().fmt(f)
    }
}

pub struct ConnectError<T> {
    kind: ConnectErrorKind,
    inner: T
}

impl<T> ConnectError<T> {
    pub fn new(kind: ConnectErrorKind, inner: T) -> ConnectError<T> {
        ConnectError { kind: kind, inner: inner }
    }
    
    /// Helper method to create ConnectErrorKind::Other.
    pub fn other(msg: &'static str, inner: T) -> ConnectError<T> {
        Self::new(ConnectErrorKind::Other(msg), inner)
    }
    
    pub fn kind(&self) -> ConnectErrorKind {
        self.kind
    }
    
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> fmt::Debug for ConnectError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.kind.fmt(f)
    }
}

impl<T> fmt::Display for ConnectError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.kind.fmt(f)
    }
}

// This is currently not possible in stable Rust, but instead we can directly
// implement a conversion to Box<Error> by boxing just the error kind.

//impl<T: Reflect> Error for ConnectError<T>

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    InvalidData(&'static str),
    Other(&'static str)
}

impl Error for SendError {
    fn description(&self) -> &str {
        match *self {
            SendError::InvalidData(msg) => msg,
            SendError::Other(msg) => msg
        }
    }
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.description().fmt(f)
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

mod traits;
pub use traits::*;

// TODO: allow feature selection (ALSA and/or Jack)
#[cfg(target_os="linux")] pub mod alsa;
#[cfg(target_os="linux")] pub type MidiInput = alsa::MidiInput;
#[cfg(target_os="linux")] pub type MidiInputConnection<T> = alsa::MidiInputConnection<T>;
#[cfg(target_os="linux")] pub type MidiOutput = alsa::MidiOutput;
#[cfg(target_os="linux")] pub type MidiOutputConnection = alsa::MidiOutputConnection;

#[cfg(target_os="windows")] pub mod winmm;
#[cfg(target_os="windows")] pub type MidiInput = winmm::MidiInput;
#[cfg(target_os="windows")] pub type MidiInputConnection<T> = winmm::MidiInputConnection<T>;
#[cfg(target_os="windows")] pub type MidiOutput = winmm::MidiOutput;
#[cfg(target_os="windows")] pub type MidiOutputConnection = winmm::MidiOutputConnection;