use std::{mem, str};
use std::ffi::{CString, CStr};
use std::ops::{Deref, DerefMut};
use alsa_sys::{
    snd_seq_t,
    snd_seq_open,
    snd_seq_close,
    snd_seq_create_port,
    snd_seq_create_simple_port,
    snd_seq_set_client_name,
    snd_seq_get_any_client_info,
    snd_seq_poll_descriptors_count,
    snd_seq_client_id,
    snd_seq_client_info_t,
    snd_seq_client_info_malloc,
    snd_seq_client_info_free,
    snd_seq_client_info_set_client,
    snd_seq_client_info_get_client,
    snd_seq_client_info_get_name,
    snd_seq_port_info_t,
    snd_seq_port_info_malloc,
    snd_seq_port_info_free,
    snd_seq_port_info_get_client,
    snd_seq_port_info_set_client,
    snd_seq_port_info_get_port,
    snd_seq_port_info_set_port,
    snd_seq_port_info_get_capability,
    snd_seq_port_info_set_capability,
    snd_seq_port_info_set_name,
    snd_seq_port_info_get_type,
    snd_seq_port_info_set_type,
    snd_seq_port_info_set_midi_channels,
    snd_seq_port_info_set_timestamping,
    snd_seq_port_info_set_timestamp_real,    
    snd_seq_port_info_set_timestamp_queue,
    snd_seq_port_subscribe_t,
    snd_seq_port_subscribe_malloc,
    snd_seq_port_subscribe_free,
    snd_seq_port_subscribe_set_sender,
    snd_seq_port_subscribe_set_dest,
    snd_seq_port_subscribe_set_time_update,
    snd_seq_port_subscribe_set_time_real,
    snd_seq_addr_t,
    snd_midi_event_t,
    snd_midi_event_no_status,
    snd_midi_event_new,
    snd_midi_event_free,
    snd_midi_event_decode,
    snd_midi_event_encode,
    snd_midi_event_resize_buffer,
    snd_seq_event_t,
    snd_seq_free_event,
    snd_seq_event_input,
    snd_seq_event_output,
    snd_seq_drain_output,
    snd_seq_queue_tempo_t,
    snd_seq_queue_tempo_malloc,
    snd_seq_queue_tempo_free,
    snd_seq_queue_tempo_set_tempo,
    snd_seq_queue_tempo_set_ppq,
    snd_seq_timestamp_t,
    snd_seq_query_next_client,
    snd_seq_query_next_port,
};
use libc::c_char;

const SND_SEQ_OPEN_OUTPUT: i32 = 1;
const SND_SEQ_OPEN_INPUT: i32 = 2;
const SND_SEQ_OPEN_DUPLEX: i32 = SND_SEQ_OPEN_OUTPUT|SND_SEQ_OPEN_INPUT;
const SND_SEQ_NONBLOCK: i32 = 0x0001;
const SND_SEQ_ADDRESS_SUBSCRIBERS: u8 = 254;
const SND_SEQ_ADDRESS_UNKNOWN: u8 = 253;
const SND_SEQ_QUEUE_DIRECT: u8 = 253;

// TODO: use bitflags! macro
pub const SND_SEQ_PORT_TYPE_MIDI_GENERIC: u32 = 1<<1;
pub const SND_SEQ_PORT_TYPE_SYNTH: u32 = 1<<10;
pub const SND_SEQ_PORT_TYPE_APPLICATION: u32 = 1<<20;

// TODO: try to make sure that wrapped pointers are directly initialized,
//       then mark them as non-zero and use std::ptr::Unique where appropriate 

// Define some bindings and types which are not available from alsa-sys or libc
extern {
  fn snd_seq_poll_descriptors(seq: *mut snd_seq_t,
    pfds: *mut ::libc::pollfd,
    space: u32,
    events: i16 
    ) -> i32;
}

pub fn poll(fds: &mut [::libc::pollfd], timeout: i32) -> i32 {
    unsafe { ::libc::poll(fds.as_mut_ptr(), fds.len() as ::libc::nfds_t, timeout) }
}

const DEFAULT_SEQ: &'static [u8] = b"default\0";

/// This function is used to count or get the pinfo structure for a given port number.
/// TODO: introduce iterator?
pub fn get_port_info(seq: &Sequencer, pinfo: &mut PortInfo, typ: u32, port_number: i32) -> Option<i32> {
    let mut client;
    let mut count = 0;
    let mut cinfo = unsafe { ClientInfo::allocate() };
    
    // Get a *mut pointer out of `seq`. This should be safe, since
    // the `query` functions won't modify it, the interface just always uses *mut.
    let seq = seq.p as *mut _;

    cinfo.set_client(-1);
    while unsafe { snd_seq_query_next_client(seq, cinfo.as_ptr()) } >= 0 {
        client = cinfo.get_client();
        if client == 0 { continue; }
        // Reset query info
        pinfo.set_client(client);
        pinfo.set_port(-1);
        while unsafe { snd_seq_query_next_port(seq, pinfo.as_ptr()) } >= 0 {
            let atyp: u32 = pinfo.get_type();
            if (atyp & SND_SEQ_PORT_TYPE_MIDI_GENERIC) == 0 &&
                (atyp & SND_SEQ_PORT_TYPE_SYNTH ) == 0 { continue; }
            let caps: u32 = pinfo.get_capability();
            if (caps & typ) != typ { continue; }
            if count == port_number { return Some(1); }
            count += 1;
        }
    }

    // If a negative portNumber was used, return the port count.
    // TODO: This could be a separate function which returns a u32
    if port_number < 0 { return Some(count) };
    None
}

pub fn get_port_name(seq: &Sequencer, typ: u32, port_number: i32) -> Result<String, ()> {
    use std::fmt::Write;
    
    let mut pinfo = unsafe { PortInfo::allocate() };
    
    if get_port_info(seq, &mut pinfo, typ, port_number).is_some() {
        let cnum: i32 = pinfo.get_client();    
        let cinfo = seq.get_any_client_info(cnum);
        let mut output = String::new();
        write!(&mut output, "{} {}:{}", 
            cinfo.get_name(),
            pinfo.get_client(), // These lines added to make sure devices are listed
            pinfo.get_port()    // with full portnames added to ensure individual device names
        ).unwrap();
        Ok(output)
    } else {
        // If we get here, we didn't find a match.
        Err(())
    }
}

#[repr(i32)]
pub enum SequencerOpenMode {
    Output = SND_SEQ_OPEN_OUTPUT,
    Input = SND_SEQ_OPEN_INPUT,
    Duplex = SND_SEQ_OPEN_DUPLEX
}

pub struct Sequencer {
    p: *mut snd_seq_t
}

unsafe impl Send for Sequencer {}

impl Sequencer {
    pub fn open(mode: SequencerOpenMode, non_block: bool) -> Result<Sequencer, ()> {
        let mut seq = unsafe { mem::uninitialized() };
        let result = unsafe { snd_seq_open(
            &mut seq,
            DEFAULT_SEQ.as_ptr() as *const c_char,
            mode as i32,
            if non_block { SND_SEQ_NONBLOCK } else { 0 }
        ) };
        if result < 0 { Err(()) }
        else { Ok(Sequencer { p: seq }) }
    }
    
    pub fn get_client_id(&self) -> i32 {
        unsafe { snd_seq_client_id(self.p) }      
    }
    
    pub fn get_any_client_info(&self, cnum: i32) -> ClientInfo {
        unsafe {
            let mut cinfo = ClientInfo::allocate();
            snd_seq_get_any_client_info(self.p, cnum, cinfo.as_ptr());
            cinfo
        }
    }
    
    pub fn set_client_name(&mut self, name: &str) {
        let c_name = CString::new(name).ok().expect("client name must not contain null bytes");
        unsafe { snd_seq_set_client_name(self.p, c_name.as_ptr()) };
    }
    
    pub fn create_port(&mut self, port: &mut PortInfo) -> Result<(), i32> {
        match unsafe { snd_seq_create_port(self.p, port.p) } {
            0 => Ok(()),
            err => Err(err)
        }
    }
    
    pub fn create_simple_port(&mut self, port_name: &str, caps: u32, typ: u32) -> Result<i32, i32> {
        let c_name = CString::new(port_name).ok().expect("port_name must not contain null bytes");
        let result = unsafe { snd_seq_create_simple_port(self.p, c_name.as_ptr(), caps, typ) };
        if result < 0 {
            Err(result)
        } else {
            Ok(result)
        }
    }
    
    pub fn poll_descriptors_count(&self, events: i16) -> i32 {
        unsafe { snd_seq_poll_descriptors_count(self.p, events) }
    }
    
    pub fn poll_descriptors(&self, pollfds: &mut [::libc::pollfd], events: i16) {
        unsafe { snd_seq_poll_descriptors(self.p, pollfds.as_mut_ptr(), pollfds.len() as u32, events) };   
    }
    
    pub fn event_input(&mut self) -> Result<(EventBox, i32), i32> {
        let mut ev = unsafe { mem::uninitialized() };
        let result = unsafe { snd_seq_event_input(self.p, &mut ev) };
        if result < 0 {
            Err(result)
        } else {
            Ok((EventBox { p: ev }, result))
        }
    }
    
    pub fn event_output(&mut self, ev: &Event) -> Result<i32, i32> {
        let result = unsafe { snd_seq_event_output(self.p, (&ev.ev as *const _) as *mut _) };
        if result < 0 {
            Err(result)
        } else {
            Ok(result)
        }
    }
    
    pub fn drain_output(&mut self) {
        unsafe { snd_seq_drain_output(self.p) };
    }
    
    pub fn as_ptr(&self) -> *const snd_seq_t {
        self.p
    }
    
    pub fn as_mut_ptr(&mut self) -> *mut snd_seq_t {
        self.p
    }
}

impl Drop for Sequencer {
    fn drop(&mut self) {
        unsafe { snd_seq_close(self.p) };
    }
}

pub struct ClientInfo {
    p: *mut snd_seq_client_info_t
}

unsafe impl Send for ClientInfo {}

impl ClientInfo {
    pub unsafe fn allocate() -> ClientInfo {
        let mut cinfo: *mut snd_seq_client_info_t = mem::uninitialized();
        snd_seq_client_info_malloc(&mut cinfo);
        // TODO: check return value?
        ClientInfo { p: cinfo }
    }
    
    pub fn as_ptr(&mut self) -> *mut snd_seq_client_info_t {
        self.p
    }
    
    pub fn set_client(&mut self, client: i32) {
        unsafe { snd_seq_client_info_set_client(self.p, client) };
    }
    
    pub fn get_client(&self) -> i32 {
        unsafe { snd_seq_client_info_get_client(self.p) }
    }
    
    pub fn get_name(&self) -> &str {
        let name_bytes = unsafe { CStr::from_ptr(snd_seq_client_info_get_name(self.p)).to_bytes() }; 
        str::from_utf8(name_bytes).ok().expect("Error converting name to UTF8")
    }
}

impl Drop for ClientInfo {
    fn drop(&mut self) {
        unsafe { snd_seq_client_info_free(self.p) };
    }
}


pub struct PortInfo {
    p: *mut snd_seq_port_info_t
}

unsafe impl Send for PortInfo {}

impl PortInfo {
    pub unsafe fn allocate() -> PortInfo {
        let mut pinfo: *mut snd_seq_port_info_t = mem::uninitialized();
        snd_seq_port_info_malloc(&mut pinfo);
        // TODO: check return value?
        PortInfo { p: pinfo }
    }
    
    pub fn as_ptr(&mut self) -> *mut snd_seq_port_info_t {
        self.p
    }
    
    pub fn get_client(&self) -> i32 {
        unsafe { snd_seq_port_info_get_client(self.p) }
    }
    
    pub fn set_client(&mut self, client: i32) {
        unsafe { snd_seq_port_info_set_client(self.p, client) };
    }
    
    pub fn get_port(&self) -> i32 {
        unsafe { snd_seq_port_info_get_port(self.p) }
    }
    
    pub fn set_port(&mut self, port: i32) {
        unsafe { snd_seq_port_info_set_port(self.p, port) };
    }
    
    pub fn get_capability(&self) -> u32 {
        unsafe { snd_seq_port_info_get_capability(self.p) }
    }
    
    pub fn set_capability(&mut self, capability: u32) {
        unsafe { snd_seq_port_info_set_capability(self.p, capability) }
    }
    
    pub fn get_type(&self) -> u32 {
        unsafe { snd_seq_port_info_get_type(self.p) }
    }
    
    pub fn set_type(&mut self, typ: u32) {
        unsafe { snd_seq_port_info_set_type(self.p, typ) };
    }
    
    pub fn set_midi_channels(&mut self, channels: i32) {
        unsafe { snd_seq_port_info_set_midi_channels(self.p, channels) }
    }
    
    pub fn set_name(&mut self, name: &str) {
        let cname = CString::new(name).ok().expect("Error creating C string");
        unsafe { snd_seq_port_info_set_name(self.p, cname.as_ptr()) }; 
    }
    
    pub fn set_timestamping(&mut self, enable: bool) {
        unsafe { snd_seq_port_info_set_timestamping(self.p, enable as i32) }; 
    }
    
    pub fn set_timestamp_real(&mut self, enable: bool) {
        unsafe { snd_seq_port_info_set_timestamp_real(self.p, enable as i32) };
    }
    
    pub fn set_timestamp_queue(&mut self, queue: i32) {
        unsafe { snd_seq_port_info_set_timestamp_queue(self.p, queue) };
    }
}

impl Drop for PortInfo {
    fn drop(&mut self) {
        unsafe { snd_seq_port_info_free(self.p) };
    }
}

pub struct PortSubscription {
    p: *mut snd_seq_port_subscribe_t
}

unsafe impl Send for PortSubscription {}

impl PortSubscription {
    pub unsafe fn allocate() -> PortSubscription {
        let mut psub: *mut snd_seq_port_subscribe_t = mem::uninitialized();
        snd_seq_port_subscribe_malloc(&mut psub);
        // TODO: check return value?
        PortSubscription { p: psub }
    }
    
    pub fn as_ptr(&mut self) -> *mut snd_seq_port_subscribe_t {
        self.p
    }
    
    pub fn set_sender(&mut self, addr: *const snd_seq_addr_t) {
        unsafe { snd_seq_port_subscribe_set_sender(self.p, addr) }
    }
    
    pub fn set_dest(&mut self, addr: *const snd_seq_addr_t) {
        unsafe { snd_seq_port_subscribe_set_dest(self.p, addr) }
    }
    
    pub fn set_time_update(&mut self, enable: bool) {
        unsafe { snd_seq_port_subscribe_set_time_update(self.p, enable as i32) }; 
    }
    
    pub fn set_time_real(&mut self, enable: bool) {
        unsafe { snd_seq_port_subscribe_set_time_real(self.p, enable as i32) }; 
    }
}

impl Drop for PortSubscription {
    fn drop(&mut self) {
        unsafe { snd_seq_port_subscribe_free(self.p) };
    }
}

pub struct EventDecoder {
    p: *mut snd_midi_event_t
}

unsafe impl Send for EventDecoder {}

impl EventDecoder {
    pub fn new(merge_commands: bool) -> EventDecoder {
        let mut coder;
        unsafe {
            coder = mem::uninitialized();
            // this could only fail with "Out of memory", which we ignore
            snd_midi_event_new(0, &mut coder);
            //snd_midi_event_init(data.coder);
            snd_midi_event_no_status(coder, !merge_commands as i32);
        }
        EventDecoder { p: coder }
    }
    
    pub fn as_ptr(&mut self) -> *mut snd_midi_event_t {
        self.p
    }
    
    pub fn decode(&mut self, buffer: &mut [u8], ev: &mut Event) -> usize {
        unsafe {
            snd_midi_event_decode(self.p, buffer.as_mut_ptr(), buffer.len() as ::libc::c_long, &ev.ev) as usize
        }
    }
}

impl Drop for EventDecoder {
    fn drop(&mut self) {
        unsafe { snd_midi_event_free(self.p) };
    }
}

pub struct EventEncoder {
    p: *mut snd_midi_event_t,
    buffer_size: usize
}

unsafe impl Send for EventEncoder {}

impl EventEncoder {
    pub fn new(buffer_size: usize) -> EventEncoder {
        let mut coder;
        unsafe {
            coder = mem::uninitialized();
            // this could only fail with "Out of memory", which we ignore
            snd_midi_event_new(buffer_size as ::libc::size_t, &mut coder);
            //snd_midi_event_init(data.coder);
        }
        EventEncoder { p: coder, buffer_size: buffer_size }
    }
    
    pub fn as_ptr(&mut self) -> *mut snd_midi_event_t {
        self.p
    }
    
    pub fn get_buffer_size(&self) -> usize {
        self.buffer_size
    }
    
    pub fn resize_buffer(&mut self, new_size: usize) -> Result<(), ()> {
        let result = unsafe { snd_midi_event_resize_buffer(self.p, new_size as ::libc::size_t) };
        if result != 0 { Err(()) } else { Ok(()) }
    }
    
    pub fn encode(&mut self, message: &[u8], ev: &mut Event) -> Result<(), ()> {
        let result = unsafe { snd_midi_event_encode(self.p, message.as_ptr(), message.len() as ::libc::c_long, &mut ev.ev) as usize};
        if result < message.len() { Err(()) } else { Ok(()) }
    }
    
    pub fn decode(&mut self, buffer: &mut [u8], ev: &snd_seq_event_t) {
        unsafe { snd_midi_event_decode(self.p, buffer.as_mut_ptr(), buffer.len() as ::libc::c_long, ev) };
    }
}

impl Drop for EventEncoder {
    fn drop(&mut self) {
        unsafe { snd_midi_event_free(self.p) };
    }
}

pub struct Event {
    ev: snd_seq_event_t
}

impl Event {
    pub fn new() -> Event {
        // initialize everything with zero
        Event { ev: snd_seq_event_t {
            _type: 0,
            flags: 0,
            tag: 0,
            queue: 0,
            time: snd_seq_timestamp_t { data: [0; 2] },
            source: snd_seq_addr_t { client: 0, port: 0 },
            dest: snd_seq_addr_t  { client: 0, port: 0 },
            data: ::alsa_sys::Union_Unnamed10 { data: [0; 3] },
        }}
    }
    
    #[inline(always)]
    pub fn set_source(&mut self, p: u8) {
        self.source.port = p;
    }
    
    #[inline(always)]
    pub fn set_subs(&mut self) {
        self.dest.client = SND_SEQ_ADDRESS_SUBSCRIBERS;
        self.dest.port = SND_SEQ_ADDRESS_UNKNOWN;
    }
    
    #[inline(always)]
    pub fn set_direct(&mut self) {
        self.queue = SND_SEQ_QUEUE_DIRECT;
    }
}

impl Deref for Event {
    type Target = snd_seq_event_t;
    fn deref(&self) -> &Self::Target {
        &self.ev
    }
}

impl DerefMut for Event {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ev
    }
}

pub struct EventBox {
    p: *const snd_seq_event_t
}

unsafe impl Send for EventBox {}

impl Deref for EventBox {
    type Target = Event;
    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.p as *const Event) }
    }
}

impl DerefMut for EventBox {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *(self.p as *mut Event) }
    }
}

impl Drop for EventBox {
    fn drop(&mut self) {
        unsafe { snd_seq_free_event(self.p as *mut _) };
    }
}

pub struct QueueTempo {
    p: *mut snd_seq_queue_tempo_t
}

unsafe impl Send for QueueTempo {}

impl QueueTempo {
    pub unsafe fn allocate() -> QueueTempo {
        let mut psub: *mut snd_seq_queue_tempo_t = mem::uninitialized();
        snd_seq_queue_tempo_malloc(&mut psub);
        // TODO: check return value?
        QueueTempo { p: psub }
    }
    
    pub fn as_ptr(&mut self) -> *mut snd_seq_queue_tempo_t {
        self.p
    }
    
    pub fn set_tempo(&mut self, tempo: u32) {
        unsafe { snd_seq_queue_tempo_set_tempo(self.p, tempo) };
    }
    
    pub fn set_ppq(&mut self, ppq: i32) {
        unsafe { snd_seq_queue_tempo_set_ppq(self.p, ppq) };
    }
}

impl Drop for QueueTempo {
    fn drop(&mut self) {
        unsafe { snd_seq_queue_tempo_free(self.p) }
    }
}
