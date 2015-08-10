#![feature(vec_push_all, box_raw, heap_api)]

#[cfg(target_os="linux")] extern crate libc;
#[cfg(target_os="linux")] extern crate alsa_sys;
#[cfg(target_os="windows")] extern crate winapi;
#[cfg(target_os="windows")] extern crate winmm as winmm_sys;

use std::ops::BitOr;
use std::mem;

#[derive(Debug)]
pub struct InitError;

#[derive(Debug)]
pub enum PortInfoError {
    PortNumberOutOfRange,
}

// TODO: implement (not derive) Debug, Show, ... without using inner T
// TODO: use struct(kind: ConnectErrorKind, inner: T) instead
#[derive(Debug)]
pub enum ConnectError<T> {
    PortNumberOutOfRange(T),
    Unspecified(T) // TODO: maybe add a &str description?
}

#[derive(Debug)]
pub enum SendError {
    InvalidData,
    Unspecified // TODO: maybe add a &str description?
}

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