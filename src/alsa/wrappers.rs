use std::{mem, str};
use std::ffi::{CString, CStr};
use alsa_sys::{
    snd_seq_t,
    snd_seq_open,
    snd_seq_close,
    snd_seq_create_port,
    snd_seq_set_client_name,
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
    snd_seq_addr_t,
    snd_midi_event_t,
    snd_midi_event_no_status,
    snd_midi_event_new,
    snd_midi_event_free,
    snd_midi_event_decode,
    snd_seq_event_t,
    snd_seq_event_input,
    snd_seq_free_event
};


const SND_SEQ_OPEN_OUTPUT: i32 = 1;
const SND_SEQ_OPEN_INPUT: i32 = 2;
const SND_SEQ_OPEN_DUPLEX: i32 = SND_SEQ_OPEN_OUTPUT|SND_SEQ_OPEN_INPUT;
const SND_SEQ_NONBLOCK: i32 = 0x0001;

// TODO: make sure that all pointers are directly initialized, then mark all wrapped pointers as non-zero
// TODO: use std::ptr::Unique
// TODO: m2ake get/set methods unsafe or make sure that pointer has actually been initialized

// Define some bindings and types which are not available from alsa-sys or libc
extern {
  fn snd_seq_poll_descriptors(seq: *mut snd_seq_t,
    pfds: *mut pollfd,
    space: u32,
    events: i16 
	) -> i32;	
}

#[repr(C)]
pub struct pollfd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
}

pub const POLLIN: i16 = 1;

pub fn poll(fds: &mut [pollfd], timeout: i32) -> i32 {
    extern { fn poll(fds: *mut pollfd, nfds: u32, timeout: i32) -> i32; }
    unsafe { poll(fds.as_mut_ptr(), fds.len() as u32, timeout) }
}

#[repr(i32)]
pub enum SequencerOpenMode {
    Output = SND_SEQ_OPEN_OUTPUT,
    Input = SND_SEQ_OPEN_INPUT,
    Duplex = SND_SEQ_OPEN_DUPLEX
}

#[derive(Debug)]
pub struct Sequencer {
    p: *mut snd_seq_t
}

impl Sequencer {
    pub fn open(mode: SequencerOpenMode, non_block: bool) -> Result<Sequencer, ()> {
        let mut seq = unsafe { mem::uninitialized() };
        let result = unsafe { snd_seq_open(
            &mut seq,
            mem::transmute(b"default"),
            mode as i32,
            if non_block { SND_SEQ_NONBLOCK } else { 0 }
        ) };
        if result < 0 { Err(()) }
        else { Ok(Sequencer { p: seq }) }
    }
    
    pub fn get_client_id(&self) -> i32 {
        unsafe { snd_seq_client_id(self.p) }      
    }
    
    pub fn set_client_name(&mut self, name: &str) {
        let c_name = CString::new(name).ok().expect("client_name must not contain null bytes");
        unsafe { snd_seq_set_client_name(self.p, c_name.as_ptr()) };
    }
    
    pub fn create_port(&mut self, port: &mut PortInfo) -> Result<(), i32> {
        match unsafe { snd_seq_create_port(self.p, port.p) } {
            0 => Ok(()),
            err => Err(err)
        }
    }
    
    pub fn poll_descriptors_count(&self, events: i16) -> i32 {
        unsafe { snd_seq_poll_descriptors_count(self.p, events) }
    }
    
    pub fn poll_descriptors(&self, pollfds: &mut [pollfd], events: i16) {
        unsafe { snd_seq_poll_descriptors(self.p, pollfds.as_mut_ptr(), pollfds.len() as u32, POLLIN) };   
    }
    
    pub fn event_input(&mut self) -> Result<Event, i32> {
        let mut ev = unsafe { mem::uninitialized() };
        match unsafe { snd_seq_event_input(self.p, &mut ev) } {
            0 => Ok(Event { p: ev }),
            err => Err(err)
        }
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
        println!("Closing sequencer");
        unsafe { snd_seq_close(self.p) };
    }
}

pub struct ClientInfo {
    p: *mut snd_seq_client_info_t
}

impl ClientInfo {
    pub fn allocate() -> ClientInfo {
        let mut cinfo: *mut snd_seq_client_info_t = unsafe { mem::uninitialized() };
        unsafe { snd_seq_client_info_malloc(&mut cinfo) };
        // TODO: check return value?
        ClientInfo { p: cinfo }
    }
    
    pub fn as_ptr(&mut self) -> *mut snd_seq_client_info_t {
        self.p
    }
    
    pub unsafe fn set_client(&mut self, client: i32) {
        snd_seq_client_info_set_client(self.p, client);
    }
    
    pub unsafe fn get_client(&self) -> i32 {
        snd_seq_client_info_get_client(self.p)
    }
    
    pub unsafe fn get_name(&self) -> &str {
        let name_bytes = CStr::from_ptr(snd_seq_client_info_get_name(self.p)).to_bytes(); 
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

impl PortInfo {
    pub fn allocate() -> PortInfo {
        let mut pinfo: *mut snd_seq_port_info_t = unsafe { mem::uninitialized() };
        unsafe { snd_seq_port_info_malloc(&mut pinfo) };
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

impl PortSubscription {
    pub fn allocate() -> PortSubscription {
        let mut psub: *mut snd_seq_port_subscribe_t = unsafe { mem::uninitialized() };
        unsafe { snd_seq_port_subscribe_malloc(&mut psub) };
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
}

impl Drop for PortSubscription {
    fn drop(&mut self) {
        unsafe { snd_seq_port_subscribe_free(self.p) };
    }
}

pub struct EventDecoder {
    p: *mut snd_midi_event_t
}

impl EventDecoder {
    pub fn new(no_status: bool) -> EventDecoder {
        let mut coder;
        unsafe {
            coder = mem::uninitialized();
            // this could only fail with "Out of memory", which we ignore
            snd_midi_event_new(0, &mut coder);
            //snd_midi_event_init(data.coder);
            snd_midi_event_no_status(coder, no_status as i32);
        }
        EventDecoder { p: coder }
    }
    
    pub fn as_ptr(&mut self) -> *mut snd_midi_event_t {
        self.p
    }
    
    pub fn decode(&mut self, buffer: &mut [u8], ev: Event) -> usize {
        unsafe {
            snd_midi_event_decode(self.p, buffer.as_mut_ptr(), buffer.len() as i64, ev.p) as usize
        }
    }
}

impl Drop for EventDecoder {
    fn drop(&mut self) {
        unsafe { snd_midi_event_free(self.p) };
    }
}

pub struct Event {
    p: *const snd_seq_event_t
}

impl Event {
    pub fn as_ref(&mut self) -> &mut snd_seq_event_t {
        unsafe { &mut *(self.p as *mut _) }
    } 
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe { snd_seq_free_event(self.p as *mut _) };
    }
}