//! Conversion between MIDI 1.0 byte messages and Universal MIDI Packets (UMP).
//!
//! Windows MIDI Services speaks UMP only. midir's public API is MIDI 1.0 bytes, so every
//! message is converted on the way out and on the way back in. This module has no Windows
//! dependencies, so its tests can run anywhere.
//!
//! Translation from MIDI 2.0 Channel Voice messages to MIDI 1.0 follows the Default
//! Translation in Appendix D of the UMP and MIDI 2.0 Protocol specification (M2-104-UM).

/// One UMP of up to two 32-bit words. Messages this module produces are never longer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ump {
    pub words: [u32; 2],
    pub len: u8,
}

impl Ump {
    fn one(w0: u32) -> Self {
        Ump {
            words: [w0, 0],
            len: 1,
        }
    }

    fn two(w0: u32, w1: u32) -> Self {
        Ump {
            words: [w0, w1],
            len: 2,
        }
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.words[..self.len as usize]
    }
}

/// Number of 32-bit words in a UMP, from its message type (the top 4 bits of the first word).
pub fn ump_word_count(first_word: u32) -> usize {
    match first_word >> 28 {
        0x0 | 0x1 | 0x2 | 0x6 | 0x7 => 1,
        0x3 | 0x4 | 0x8 | 0x9 | 0xA => 2,
        0xB | 0xC => 3,
        _ => 4,
    }
}

const MT_SYSTEM: u32 = 0x1;
const MT_MIDI1_CHANNEL_VOICE: u32 = 0x2;
const MT_SYSEX7: u32 = 0x3;
const MT_MIDI2_CHANNEL_VOICE: u32 = 0x4;

const SYSEX7_COMPLETE: u32 = 0x0;
const SYSEX7_START: u32 = 0x1;
const SYSEX7_CONTINUE: u32 = 0x2;
const SYSEX7_END: u32 = 0x3;

const SYSEX_START: u8 = 0xF0;
const SYSEX_END: u8 = 0xF7;

/// Number of data bytes that follow a MIDI 1.0 status byte, or `None` for status bytes
/// that cannot be sent on their own (SysEx start and end, and the undefined ones).
fn data_byte_count(status: u8) -> Option<usize> {
    match status {
        0x80..=0xBF | 0xE0..=0xEF => Some(2),
        0xC0..=0xDF => Some(1),
        0xF1 | 0xF3 => Some(1),
        0xF2 => Some(2),
        0xF6 | 0xF8 | 0xFA | 0xFB | 0xFC | 0xFE | 0xFF => Some(0),
        _ => None,
    }
}

fn is_realtime(byte: u8) -> bool {
    matches!(byte, 0xF8 | 0xFA | 0xFB | 0xFC | 0xFE | 0xFF)
}

/// Converts MIDI 1.0 bytes, as passed to `MidiOutputConnection::send`, into UMPs for one group.
///
/// State is kept between calls, so a SysEx message may be split over several `send` calls,
/// and running status carries over.
pub struct BytesToUmp {
    group: u32,
    running_status: Option<u8>,
    sysex: Option<SysexOut>,
}

struct SysexOut {
    pending: [u8; 6],
    pending_len: usize,
    started: bool,
}

impl BytesToUmp {
    pub fn new(group: u8) -> Self {
        BytesToUmp {
            group: (group & 0x0F) as u32,
            running_status: None,
            sysex: None,
        }
    }

    /// Converts `bytes` and appends the resulting UMPs to `out`. On error, the encoder is
    /// reset, and `out` holds the UMPs for everything before the bad byte.
    pub fn encode(&mut self, bytes: &[u8], out: &mut Vec<Ump>) -> Result<(), &'static str> {
        let result = self.encode_inner(bytes, out);
        if result.is_err() {
            self.running_status = None;
            self.sysex = None;
        }
        result
    }

    fn encode_inner(&mut self, bytes: &[u8], out: &mut Vec<Ump>) -> Result<(), &'static str> {
        let mut i = 0;
        while i < bytes.len() {
            let byte = bytes[i];

            if self.sysex.is_some() {
                if byte < 0x80 {
                    self.push_sysex_byte(byte, out);
                    i += 1;
                    continue;
                }
                if is_realtime(byte) {
                    out.push(self.system_message(byte, 0, 0));
                    i += 1;
                    continue;
                }
                if byte == SYSEX_END {
                    self.finish_sysex(out);
                    i += 1;
                    continue;
                }
                return Err("SysEx message was interrupted by a status byte other than 0xF7");
            }

            if byte == SYSEX_START {
                self.running_status = None;
                self.sysex = Some(SysexOut {
                    pending: [0; 6],
                    pending_len: 0,
                    started: false,
                });
                i += 1;
                continue;
            }

            let (status, data_start) = if byte >= 0x80 {
                (byte, i + 1)
            } else {
                match self.running_status {
                    Some(status) => (status, i),
                    None => return Err("data byte without a status byte"),
                }
            };

            let count = match data_byte_count(status) {
                Some(count) => count,
                None => return Err("status byte that cannot be sent on its own"),
            };
            if data_start + count > bytes.len() {
                return Err("message is missing data bytes");
            }
            let data = &bytes[data_start..data_start + count];
            if data.iter().any(|&b| b >= 0x80) {
                return Err("message is missing data bytes");
            }
            let d1 = data.first().copied().unwrap_or(0);
            let d2 = data.get(1).copied().unwrap_or(0);

            if status < 0xF0 {
                self.running_status = Some(status);
                out.push(self.channel_voice_message(status, d1, d2));
            } else {
                if !is_realtime(status) {
                    // System Common messages cancel running status.
                    self.running_status = None;
                }
                out.push(self.system_message(status, d1, d2));
            }
            i = data_start + count;
        }
        Ok(())
    }

    fn channel_voice_message(&self, status: u8, d1: u8, d2: u8) -> Ump {
        Ump::one(
            (MT_MIDI1_CHANNEL_VOICE << 28)
                | (self.group << 24)
                | ((status as u32) << 16)
                | ((d1 as u32) << 8)
                | d2 as u32,
        )
    }

    fn system_message(&self, status: u8, d1: u8, d2: u8) -> Ump {
        Ump::one(
            (MT_SYSTEM << 28)
                | (self.group << 24)
                | ((status as u32) << 16)
                | ((d1 as u32) << 8)
                | d2 as u32,
        )
    }

    fn push_sysex_byte(&mut self, byte: u8, out: &mut Vec<Ump>) {
        let group = self.group;
        let sysex = self.sysex.as_mut().expect("SysEx in progress");
        if sysex.pending_len == 6 {
            // A seventh byte means the six held back are not the last ones.
            let status = if sysex.started {
                SYSEX7_CONTINUE
            } else {
                SYSEX7_START
            };
            out.push(sysex7_packet(group, status, &sysex.pending));
            sysex.started = true;
            sysex.pending_len = 0;
        }
        sysex.pending[sysex.pending_len] = byte;
        sysex.pending_len += 1;
    }

    fn finish_sysex(&mut self, out: &mut Vec<Ump>) {
        let sysex = self.sysex.take().expect("SysEx in progress");
        let status = if sysex.started {
            SYSEX7_END
        } else {
            SYSEX7_COMPLETE
        };
        out.push(sysex7_packet(
            self.group,
            status,
            &sysex.pending[..sysex.pending_len],
        ));
    }
}

fn sysex7_packet(group: u32, status: u32, data: &[u8]) -> Ump {
    debug_assert!(data.len() <= 6);
    let mut bytes = [0u8; 6];
    bytes[..data.len()].copy_from_slice(data);
    let w0 = (MT_SYSEX7 << 28)
        | (group << 24)
        | (status << 20)
        | ((data.len() as u32) << 16)
        | ((bytes[0] as u32) << 8)
        | bytes[1] as u32;
    let w1 = ((bytes[2] as u32) << 24)
        | ((bytes[3] as u32) << 16)
        | ((bytes[4] as u32) << 8)
        | bytes[5] as u32;
    Ump::two(w0, w1)
}

/// Converts UMPs received from an endpoint into MIDI 1.0 byte messages for one group.
///
/// Each call to `emit` gets one complete MIDI 1.0 message. A SysEx message is delivered whole,
/// from 0xF0 to 0xF7, after its last packet arrives.
pub struct UmpToBytes {
    group: u32,
    sysex: Option<Vec<u8>>,
    scratch: Vec<u8>,
}

impl UmpToBytes {
    pub fn new(group: u8) -> Self {
        UmpToBytes {
            group: (group & 0x0F) as u32,
            sysex: None,
            scratch: Vec::with_capacity(3),
        }
    }

    /// Decodes one UMP. `words` must hold the whole UMP. Messages for other groups, and
    /// messages with no MIDI 1.0 equivalent, are skipped.
    pub fn decode<F: FnMut(&[u8])>(&mut self, words: &[u32], mut emit: F) {
        let Some(&w0) = words.first() else {
            return;
        };
        if words.len() < ump_word_count(w0) {
            return;
        }
        let message_type = w0 >> 28;
        match message_type {
            MT_SYSTEM | MT_MIDI1_CHANNEL_VOICE | MT_SYSEX7 | MT_MIDI2_CHANNEL_VOICE => {}
            // Utility, SysEx8 / Mixed Data Set, stream and the rest either have no MIDI 1.0
            // form (M2-104-UM D.2.9) or carry no group.
            _ => return,
        }
        if (w0 >> 24) & 0x0F != self.group {
            return;
        }

        match message_type {
            MT_SYSTEM => self.decode_system(w0, &mut emit),
            MT_MIDI1_CHANNEL_VOICE => self.decode_midi1_channel_voice(w0, &mut emit),
            MT_SYSEX7 => self.decode_sysex7(w0, words[1], &mut emit),
            MT_MIDI2_CHANNEL_VOICE => self.decode_midi2_channel_voice(w0, words[1], &mut emit),
            _ => {}
        }
    }

    fn emit_bytes<F: FnMut(&[u8])>(&mut self, bytes: &[u8], emit: &mut F) {
        self.scratch.clear();
        self.scratch.extend_from_slice(bytes);
        emit(&self.scratch);
    }

    fn decode_system<F: FnMut(&[u8])>(&mut self, w0: u32, emit: &mut F) {
        let status = ((w0 >> 16) & 0xFF) as u8;
        if status < 0xF0 {
            return;
        }
        let d1 = ((w0 >> 8) & 0x7F) as u8;
        let d2 = (w0 & 0x7F) as u8;
        match data_byte_count(status) {
            Some(0) => self.emit_bytes(&[status], emit),
            Some(1) => self.emit_bytes(&[status, d1], emit),
            Some(2) => self.emit_bytes(&[status, d1, d2], emit),
            _ => {}
        }
    }

    fn decode_midi1_channel_voice<F: FnMut(&[u8])>(&mut self, w0: u32, emit: &mut F) {
        let status = ((w0 >> 16) & 0xFF) as u8;
        let d1 = ((w0 >> 8) & 0x7F) as u8;
        let d2 = (w0 & 0x7F) as u8;
        match status {
            0x80..=0xBF | 0xE0..=0xEF => self.emit_bytes(&[status, d1, d2], emit),
            0xC0..=0xDF => self.emit_bytes(&[status, d1], emit),
            _ => {}
        }
    }

    fn decode_sysex7<F: FnMut(&[u8])>(&mut self, w0: u32, w1: u32, emit: &mut F) {
        let status = (w0 >> 20) & 0x0F;
        let count = (((w0 >> 16) & 0x0F) as usize).min(6);
        let all = [
            ((w0 >> 8) & 0x7F) as u8,
            (w0 & 0x7F) as u8,
            ((w1 >> 24) & 0x7F) as u8,
            ((w1 >> 16) & 0x7F) as u8,
            ((w1 >> 8) & 0x7F) as u8,
            (w1 & 0x7F) as u8,
        ];
        let data = &all[..count];

        match status {
            SYSEX7_COMPLETE => {
                // A complete message also ends any message that was left unfinished.
                self.sysex = None;
                let mut message = Vec::with_capacity(count + 2);
                message.push(SYSEX_START);
                message.extend_from_slice(data);
                message.push(SYSEX_END);
                emit(&message);
            }
            SYSEX7_START => {
                let mut message = Vec::with_capacity(64);
                message.push(SYSEX_START);
                message.extend_from_slice(data);
                self.sysex = Some(message);
            }
            SYSEX7_CONTINUE => {
                if let Some(message) = self.sysex.as_mut() {
                    message.extend_from_slice(data);
                }
            }
            SYSEX7_END => {
                if let Some(mut message) = self.sysex.take() {
                    message.extend_from_slice(data);
                    message.push(SYSEX_END);
                    emit(&message);
                }
            }
            _ => {}
        }
    }

    fn decode_midi2_channel_voice<F: FnMut(&[u8])>(&mut self, w0: u32, w1: u32, emit: &mut F) {
        let opcode = ((w0 >> 20) & 0x0F) as u8;
        let channel = ((w0 >> 16) & 0x0F) as u8;
        let index1 = ((w0 >> 8) & 0x7F) as u8;
        let index2 = (w0 & 0x7F) as u8;
        let cc = 0xB0 | channel;

        match opcode {
            // Note Off
            0x8 => {
                let velocity = scale_down(w1 >> 16, 16, 7) as u8;
                self.emit_bytes(&[0x80 | channel, index1, velocity], emit);
            }
            // Note On. A MIDI 1.0 velocity of 0 means Note Off, so 0 becomes 1 (D.2.1).
            0x9 => {
                let velocity = (scale_down(w1 >> 16, 16, 7) as u8).max(1);
                self.emit_bytes(&[0x90 | channel, index1, velocity], emit);
            }
            // Poly Pressure
            0xA => {
                let value = scale_down(w1, 32, 7) as u8;
                self.emit_bytes(&[0xA0 | channel, index1, value], emit);
            }
            // Control Change
            0xB => {
                let value = scale_down(w1, 32, 7) as u8;
                self.emit_bytes(&[cc, index1, value], emit);
            }
            // Registered (RPN) and Assignable (NRPN) Controllers: four Control Changes (D.2.3).
            0x2 | 0x3 => {
                let (bank_cc, index_cc) = if opcode == 0x2 { (101, 100) } else { (99, 98) };
                let value = scale_down(w1, 32, 14);
                self.emit_bytes(&[cc, bank_cc, index1], emit);
                self.emit_bytes(&[cc, index_cc, index2], emit);
                self.emit_bytes(&[cc, 6, ((value >> 7) & 0x7F) as u8], emit);
                self.emit_bytes(&[cc, 38, (value & 0x7F) as u8], emit);
            }
            // Program Change, with Bank Select first when the Bank Valid bit is set (D.2.4).
            0xC => {
                if w0 & 0x1 != 0 {
                    self.emit_bytes(&[cc, 0, ((w1 >> 8) & 0x7F) as u8], emit);
                    self.emit_bytes(&[cc, 32, (w1 & 0x7F) as u8], emit);
                }
                self.emit_bytes(&[0xC0 | channel, ((w1 >> 24) & 0x7F) as u8], emit);
            }
            // Channel Pressure
            0xD => {
                let value = scale_down(w1, 32, 7) as u8;
                self.emit_bytes(&[0xD0 | channel, value], emit);
            }
            // Pitch Bend, LSB first.
            0xE => {
                let value = scale_down(w1, 32, 14);
                let lsb = (value & 0x7F) as u8;
                let msb = ((value >> 7) & 0x7F) as u8;
                self.emit_bytes(&[0xE0 | channel, lsb, msb], emit);
            }
            // Per-note controllers, relative controllers, per-note pitch bend and per-note
            // management have no MIDI 1.0 equivalent (D.2.8).
            _ => {}
        }
    }
}

/// Downscaling by dropping low bits, as in M2-104-UM D.1.4.
fn scale_down(value: u32, source_bits: u32, destination_bits: u32) -> u32 {
    let value = if source_bits < 32 {
        value & ((1u32 << source_bits) - 1)
    } else {
        value
    };
    value >> (source_bits - destination_bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(group: u8, bytes: &[u8]) -> Vec<Vec<u32>> {
        let mut encoder = BytesToUmp::new(group);
        let mut out = Vec::new();
        encoder.encode(bytes, &mut out).unwrap();
        out.iter().map(|u| u.as_slice().to_vec()).collect()
    }

    fn decode(group: u8, packets: &[&[u32]]) -> Vec<Vec<u8>> {
        let mut decoder = UmpToBytes::new(group);
        let mut out = Vec::new();
        for words in packets {
            decoder.decode(words, |m| out.push(m.to_vec()));
        }
        out
    }

    #[test]
    fn note_on_goes_to_midi1_channel_voice() {
        assert_eq!(encode(0, &[0x93, 60, 100]), vec![vec![0x2093_3C64]]);
        assert_eq!(encode(5, &[0x80, 1, 2]), vec![vec![0x2580_0102]]);
    }

    #[test]
    fn program_change_has_one_data_byte() {
        assert_eq!(encode(0, &[0xC1, 7]), vec![vec![0x20C1_0700]]);
    }

    #[test]
    fn running_status_is_accepted() {
        assert_eq!(
            encode(0, &[0x90, 60, 100, 62, 90]),
            vec![vec![0x2090_3C64], vec![0x2090_3E5A]]
        );
    }

    #[test]
    fn system_messages_use_message_type_1() {
        assert_eq!(encode(2, &[0xF8]), vec![vec![0x12F8_0000]]);
        assert_eq!(encode(0, &[0xF2, 0x10, 0x20]), vec![vec![0x10F2_1020]]);
        assert_eq!(encode(0, &[0xF1, 0x35]), vec![vec![0x10F1_3500]]);
    }

    #[test]
    fn short_sysex_is_one_complete_packet() {
        assert_eq!(
            encode(0, &[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]),
            vec![vec![0x3004_7E7F, 0x0601_0000]]
        );
    }

    #[test]
    fn empty_sysex_is_a_complete_packet_with_no_data() {
        assert_eq!(encode(0, &[0xF0, 0xF7]), vec![vec![0x3000_0000, 0]]);
    }

    #[test]
    fn six_byte_sysex_is_one_complete_packet() {
        assert_eq!(
            encode(0, &[0xF0, 1, 2, 3, 4, 5, 6, 0xF7]),
            vec![vec![0x3006_0102, 0x0304_0506]]
        );
    }

    #[test]
    fn long_sysex_is_start_continue_end() {
        let mut bytes = vec![0xF0];
        bytes.extend(1..=14u8);
        bytes.push(0xF7);
        assert_eq!(
            encode(1, &bytes),
            vec![
                vec![0x3116_0102, 0x0304_0506],
                vec![0x3126_0708, 0x090A_0B0C],
                vec![0x3132_0D0E, 0x0000_0000],
            ]
        );
    }

    #[test]
    fn twelve_byte_sysex_ends_with_a_full_end_packet() {
        let mut bytes = vec![0xF0];
        bytes.extend(1..=12u8);
        bytes.push(0xF7);
        assert_eq!(
            encode(0, &bytes),
            vec![
                vec![0x3016_0102, 0x0304_0506],
                vec![0x3036_0708, 0x090A_0B0C],
            ]
        );
    }

    #[test]
    fn sysex_split_across_sends() {
        let mut encoder = BytesToUmp::new(0);
        let mut out = Vec::new();
        encoder.encode(&[0xF0, 1, 2, 3, 4], &mut out).unwrap();
        assert!(out.is_empty());
        encoder.encode(&[5, 6, 7], &mut out).unwrap();
        assert_eq!(out.len(), 1);
        encoder.encode(&[0xF7], &mut out).unwrap();
        let packets: Vec<_> = out.iter().map(|u| u.as_slice().to_vec()).collect();
        assert_eq!(
            packets,
            vec![vec![0x3016_0102, 0x0304_0506], vec![0x3031_0700, 0]]
        );
    }

    #[test]
    fn realtime_inside_sysex_is_sent_without_ending_it() {
        assert_eq!(
            encode(0, &[0xF0, 1, 0xF8, 2, 0xF7]),
            vec![vec![0x10F8_0000], vec![0x3002_0102, 0]]
        );
    }

    #[test]
    fn bad_input_is_an_error() {
        let mut encoder = BytesToUmp::new(0);
        let mut out = Vec::new();
        assert!(encoder.encode(&[60, 100], &mut out).is_err());
        assert!(encoder.encode(&[0x90, 60], &mut out).is_err());
        assert!(encoder.encode(&[0xF7], &mut out).is_err());
        assert!(encoder.encode(&[0xF4], &mut out).is_err());
        assert!(encoder.encode(&[0xF0, 1, 0x90], &mut out).is_err());
        // The error reset the encoder, so a new message works.
        out.clear();
        encoder.encode(&[0x90, 60, 100], &mut out).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn decode_midi1_messages() {
        assert_eq!(
            decode(
                3,
                &[
                    &[0x2393_3C64],
                    &[0x23C1_0700],
                    &[0x13F8_0000],
                    &[0x13F2_1020]
                ]
            ),
            vec![
                vec![0x93, 60, 100],
                vec![0xC1, 7],
                vec![0xF8],
                vec![0xF2, 0x10, 0x20]
            ]
        );
    }

    #[test]
    fn decode_skips_other_groups_and_untranslatable_types() {
        assert!(decode(0, &[&[0x2193_3C64]]).is_empty());
        assert!(decode(0, &[&[0x0000_0000]]).is_empty());
        assert!(decode(0, &[&[0xF000_0000, 0, 0, 0]]).is_empty());
        assert!(decode(0, &[&[0x5000_0000, 0, 0, 0]]).is_empty());
    }

    #[test]
    fn decode_sysex_round_trip() {
        let mut bytes = vec![0xF0];
        bytes.extend(0..40u8);
        bytes.push(0xF7);
        let packets = encode(4, &bytes);
        let refs: Vec<&[u32]> = packets.iter().map(|p| p.as_slice()).collect();
        assert_eq!(decode(4, &refs), vec![bytes]);
    }

    #[test]
    fn decode_sysex_continue_without_start_is_dropped() {
        assert!(decode(0, &[&[0x3022_0102, 0], &[0x3031_0300, 0]]).is_empty());
    }

    #[test]
    fn decode_midi2_note_on_velocity_zero_becomes_one() {
        assert_eq!(
            decode(0, &[&[0x4090_3C00, 0x0000_0000]]),
            vec![vec![0x90, 60, 1]]
        );
        assert_eq!(
            decode(0, &[&[0x4090_3C00, 0xFFFF_0000]]),
            vec![vec![0x90, 60, 127]]
        );
    }

    #[test]
    fn decode_midi2_note_off_keeps_velocity_zero() {
        assert_eq!(
            decode(0, &[&[0x4085_3C00, 0x0000_0000]]),
            vec![vec![0x85, 60, 0]]
        );
    }

    #[test]
    fn decode_midi2_control_change_scales_down() {
        assert_eq!(
            decode(0, &[&[0x40B2_0700, 0xFFFF_FFFF]]),
            vec![vec![0xB2, 7, 127]]
        );
        assert_eq!(
            decode(0, &[&[0x40B2_0700, 0x8000_0000]]),
            vec![vec![0xB2, 7, 64]]
        );
    }

    #[test]
    fn decode_midi2_pitch_bend_center() {
        assert_eq!(
            decode(0, &[&[0x40E0_0000, 0x8000_0000]]),
            vec![vec![0xE0, 0x00, 0x40]]
        );
    }

    #[test]
    fn decode_midi2_program_change_with_bank() {
        assert_eq!(
            decode(0, &[&[0x40C1_0001, 0x0500_0203]]),
            vec![vec![0xB1, 0, 2], vec![0xB1, 32, 3], vec![0xC1, 5]]
        );
        assert_eq!(
            decode(0, &[&[0x40C1_0000, 0x0500_0000]]),
            vec![vec![0xC1, 5]]
        );
    }

    #[test]
    fn decode_midi2_rpn_is_four_control_changes() {
        // RPN 0/0 (pitch bend sensitivity), 14-bit value 0x0600.
        let value = 0x0600u32 << 18;
        assert_eq!(
            decode(0, &[&[0x4020_0000, value]]),
            vec![
                vec![0xB0, 101, 0],
                vec![0xB0, 100, 0],
                vec![0xB0, 6, 0x0C],
                vec![0xB0, 38, 0x00]
            ]
        );
    }

    #[test]
    fn decode_midi2_nrpn_uses_cc_99_and_98() {
        let messages = decode(0, &[&[0x4030_0102, 0]]);
        assert_eq!(messages[0], vec![0xB0, 99, 1]);
        assert_eq!(messages[1], vec![0xB0, 98, 2]);
    }

    #[test]
    fn decode_midi2_per_note_messages_are_dropped() {
        assert!(decode(0, &[&[0x4000_3C01, 0]]).is_empty());
        assert!(decode(0, &[&[0x4060_3C00, 0]]).is_empty());
        assert!(decode(0, &[&[0x40F0_3C00, 0]]).is_empty());
    }
}
