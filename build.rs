fn main() {
    #[cfg(windows)]
    windows::build!(
        windows::foundation::*,
        windows::devices::midi::*,
        windows::devices::enumeration::DeviceInformation,
        windows::storage::streams::{Buffer, DataWriter},
        windows::win32::multimedia::{midiInAddBuffer, midiInClose, midiInGetDevCapsW, midiInGetNumDevs,
            midiInOpen, midiInPrepareHeader, midiInReset, midiInStart,
            midiInStop, midiInUnprepareHeader, midiOutClose,
            midiOutGetDevCapsW, midiOutGetNumDevs, midiOutLongMsg, midiOutOpen,
            midiOutPrepareHeader, midiOutReset, midiOutShortMsg,
            midiOutUnprepareHeader, midiInMessage, midiOutMessage,
            HMIDIIN, HMIDIOUT, MIDIHDR, MIDIINCAPSW, MIDIOUTCAPSW},
    );
  }