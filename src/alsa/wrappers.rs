use std::{mem, str};
use std::ffi::{CString, CStr};
use alsa_sys::{
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
    snd_seq_addr_t
};

// TODO: make sure that all pointers are directly initialized, then mark all wrapped pointers as non-zero
// TODO: use std::ptr::Unique
// TODO: make get/set methods unsafe or make sure that pointer has actually been initialized

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

impl PortInfo {
    pub fn allocate() -> PortInfo {
        let mut pinfo: *mut snd_seq_port_info_t = unsafe { mem::uninitialized() };
        unsafe { snd_seq_port_info_malloc(&mut pinfo) };
        // TODO: check return value?
        PortInfo { p: pinfo }
    }
    
    pub fn as_ptr(&self) -> *mut snd_seq_port_info_t {
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
    
    pub fn as_ptr(&self) -> *mut snd_seq_port_subscribe_t {
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