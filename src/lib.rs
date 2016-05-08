#![cfg_attr(windows, feature(alloc, heap_api))]

#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate enum_primitive;

#[cfg(target_os="linux")]
extern crate libc;
#[cfg(all(target_os="linux", not(feature = "jack")))]
extern crate alsa_sys;
#[cfg(all(feature = "jack", not(target_os = "windows")))]
extern crate jack_sys;

#[cfg(target_os="windows")] extern crate winapi;
#[cfg(target_os="windows")] extern crate winmm as winmm_sys;
#[cfg(target_os="windows")] extern crate alloc;

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

impl std::ops::BitOr for Ignore {
    type Output = Ignore;
    #[inline(always)]
    fn bitor(self, rhs: Self) -> Self::Output {
        // this is safe because all combinations also exist as variants
        unsafe { std::mem::transmute(self as u8 | rhs as u8) }
    }
}

impl Ignore {
    #[inline(always)]
    pub fn contains(self, other: Ignore) -> bool {
        self as u8 & other as u8 != 0 
    }
}

// A MIDI structure used internally by the class to store incoming
// messages. Each message represents one and only one MIDI message.
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShortMessage {
	pub status: u8,
	pub data1: u8,
	pub data2: u8,
}
impl ShortMessage {
	pub fn to_u32(&self) -> u32 {
		((((self.data2 as u32) << 16) & 0xFF0000) |
		  (((self.data1 as u32) << 8) & 0xFF00) |
		  ((self.status as u32) & 0xFF)) as u32
	}
}

use std::fmt;
impl fmt::Display for ShortMessage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(0x{:X}, {}, {}, {})", self.status & 0xF0, (self.status & 0x0F) + 1, self.data1, self.data2)
    }
}

enum_from_primitive! {
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    // voice
    NoteOff = 0x80,
    NoteOn = 0x90,
    PolyphonicAftertouch = 0xA0,
    ControlChange = 0xB0,
    ProgramChange = 0xC0,
    ChannelAftertouch = 0xD0,
    PitchBend = 0xE0,

    // sysex
    SysExStart = 0xF0,
    MIDITimeCodeQtrFrame = 0xF1,
    SongPositionPointer = 0xF2,
    SongSelect = 0xF3,
    TuneRequest = 0xF6, // F4 anf 5 are reserved and unused
    SysExEnd = 0xF7,
    TimingClock = 0xF8,
    Start = 0xFA,
    Continue = 0xFB,
    Stop = 0xFC,
    ActiveSensing = 0xFE, // FD also res/unused
    SystemReset = 0xFF,
}
}

pub mod os; // include platform-specific behaviour

mod errors;
pub use errors::*;

mod common;
pub use common::*;

mod backend;
