#![allow(dead_code)]

use std::mem;
use std::ptr;
use std::thread::{Builder, JoinHandle};
use std::io::{stderr, Write};

use alsa_sys::{
    snd_seq_t,
    snd_seq_addr_t,
    snd_seq_delete_port, // TODO: wrap
    snd_seq_subscribe_port,
    snd_seq_unsubscribe_port,
    snd_seq_event_t,
    snd_seq_event_input_pending,
    snd_seq_drain_output,
    snd_seq_alloc_named_queue,
    snd_seq_set_queue_tempo,
    snd_seq_free_queue, // TODO: wrap
    snd_seq_control_queue
};

use alsa_sys::{
    SND_SEQ_EVENT_PORT_SUBSCRIBED,
    SND_SEQ_EVENT_PORT_UNSUBSCRIBED,
    SND_SEQ_EVENT_QFRAME,
    SND_SEQ_EVENT_TICK,
    SND_SEQ_EVENT_CLOCK,
    SND_SEQ_EVENT_SENSING,
    SND_SEQ_EVENT_SYSEX,
    SND_SEQ_EVENT_STOP,
    SND_SEQ_EVENT_START
};

use super::{MidiMessage, Ignore};
use super::{InitError, PortInfoError, ConnectError, ConnectErrorKind, SendError};
use super::traits::*;

// Include ALSA wrappers
mod wrappers;
use self::wrappers::{
    Sequencer,
    SequencerOpenMode,
    QueueTempo,
    PortSubscription,
    EventDecoder,
    EventEncoder,
    Event,
    pollfd,
    POLLIN,
    poll,
    get_port_info,
    get_port_name,
    SND_SEQ_PORT_TYPE_MIDI_GENERIC,
    SND_SEQ_PORT_TYPE_APPLICATION
};
use self::wrappers::PortInfo as APortInfo;

#[inline(always)]
unsafe fn snd_seq_stop_queue(seq: *mut snd_seq_t, q: i32, ev: *mut snd_seq_event_t) {
    snd_seq_control_queue(seq, q, SND_SEQ_EVENT_STOP as i32, 0, ev);
}

#[inline(always)]
unsafe fn snd_seq_start_queue(seq: *mut snd_seq_t, q: i32, ev: *mut snd_seq_event_t) {
    snd_seq_control_queue(seq, q, SND_SEQ_EVENT_START as i32, 0, ev);
}

const SND_SEQ_PORT_CAP_READ: u32 = 1<<0;
const SND_SEQ_PORT_CAP_WRITE: u32 = 1<<1;
const SND_SEQ_PORT_CAP_SUBS_READ: u32 = 1<<5;
const SND_SEQ_PORT_CAP_SUBS_WRITE: u32 = 1<<6;

const INITIAL_CODER_BUFFER_SIZE: usize = 32;

pub struct MidiInput {
    ignore_flags: Ignore,
    seq: Option<Sequencer>,
}

pub struct MidiInputConnection<T: 'static> {
    subscription: Option<PortSubscription>,
    thread: Option<JoinHandle<(HandlerData<T>, T)>>,
    vport: i32, // TODO: probably port numbers are only u8, therefore could use Option<u8>
    trigger_send_fd: i32,
}

struct HandlerData<T: 'static> {
    ignore_flags: Ignore,
    seq: Sequencer,
    trigger_rcv_fd: i32,
    callback: Box<FnMut(f64, &[u8], &mut T)+Send>,
    queue_id: i32, // an input queue is needed to get timestamped events
}

impl MidiInput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        let mut seq = match Sequencer::open(SequencerOpenMode::Duplex, true) {
            Ok(s) => s,
            Err(_) => { return Err(InitError); }
        };
        
        seq.set_client_name(client_name);
        
        Ok(MidiInput {
            ignore_flags: Ignore::None,
            seq: Some(seq),
        })
    }
    
    pub fn ignore(&mut self, flags: Ignore) {
        self.ignore_flags = flags;
    }
    
    pub fn port_count(&self) -> u32 {
        unsafe {
            let mut pinfo = APortInfo::allocate();
            get_port_info(self.seq.as_ref().unwrap(), &mut pinfo, SND_SEQ_PORT_CAP_READ|SND_SEQ_PORT_CAP_SUBS_READ, -1).unwrap() as u32
        }
    }
    
    pub fn port_name(&self, port_number: u32) -> Result<String, PortInfoError> {
        match get_port_name(&self.seq.as_ref().unwrap(), SND_SEQ_PORT_CAP_READ|SND_SEQ_PORT_CAP_SUBS_READ, port_number as i32) {
            Ok(s) => Ok(s),
            Err(()) => Err(PortInfoError::PortNumberOutOfRange)
        }
    }
    
    fn init_queue(&mut self) -> i32 {
        let seq = self.seq.as_mut().unwrap();
        let mut queue_id = 0;
        // Create the input queue
        if !cfg!(feature = "avoid_timestamping") {
            queue_id = unsafe { snd_seq_alloc_named_queue(seq.as_mut_ptr(), b"midir queue\0".as_ptr() as *const i8) };
            // Set arbitrary tempo (mm=100) and resolution (240)
            let mut qtempo = unsafe { QueueTempo::allocate() };
            qtempo.set_tempo(600_000);
            qtempo.set_ppq(240);
            unsafe {
                snd_seq_set_queue_tempo(seq.as_mut_ptr(), queue_id, qtempo.as_ptr());
                snd_seq_drain_output(seq.as_mut_ptr());
            }
        }
        
        queue_id
    }
    
    fn init_trigger(&mut self) -> Result<[i32; 2], ()> {
        let mut trigger_fds = [-1, -1];
        
        if unsafe { ::libc::pipe(trigger_fds.as_mut_ptr()) } == -1 {
            Err(())
        } else {
            Ok(trigger_fds)
        }
    }
    
    fn create_port(&mut self, port_name: &str, queue_id: i32) -> Result<i32, ()> {
        let mut pinfo = unsafe { APortInfo::allocate() };
        pinfo.set_client(0);
        pinfo.set_port(0);
        pinfo.set_capability(SND_SEQ_PORT_CAP_WRITE | SND_SEQ_PORT_CAP_SUBS_WRITE);
        pinfo.set_type(SND_SEQ_PORT_TYPE_MIDI_GENERIC | SND_SEQ_PORT_TYPE_APPLICATION);
        pinfo.set_midi_channels(16);
        
        if !cfg!(feature = "avoid_timestamping") {
            pinfo.set_timestamping(true);
            pinfo.set_timestamp_real(true);
            pinfo.set_timestamp_queue(queue_id);
        }
        
        pinfo.set_name(port_name);
        match self.seq.as_mut().unwrap().create_port(&mut pinfo) {
            Ok(_) => Ok(pinfo.get_port()),
            Err(_) => Err(())
        }
    }
    
    fn start_input_queue(&mut self, queue_id: i32) {
        if !cfg!(feature = "avoid_timestamping") {
            unsafe {
                let seq = self.seq.as_mut().unwrap();
                snd_seq_start_queue(seq.as_mut_ptr(), queue_id, ptr::null_mut());
                snd_seq_drain_output(seq.as_mut_ptr());
            }
        }
    }
    
    pub fn connect<F, T: Send>(
        mut self, port_number: u32, port_name: &str, callback: F, data: T
    ) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>
        where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        
        let trigger_fds = match self.init_trigger() {
            Ok(fds) => fds,
            Err(()) => { return Err(ConnectError::other("could not create communication pipe for ALSA handler", self)); }
        };
        
        let queue_id = self.init_queue();
        
        let subscription;
        
        let mut src_pinfo = unsafe { APortInfo::allocate() };
        
        if get_port_info(self.seq.as_mut().unwrap(), &mut src_pinfo, SND_SEQ_PORT_CAP_READ | SND_SEQ_PORT_CAP_SUBS_READ, port_number as i32).is_none() {
            return Err(ConnectError::new(ConnectErrorKind::PortNumberOutOfRange, self));
        }
        
        let sender = snd_seq_addr_t {
            client: src_pinfo.get_client() as u8,
            port: src_pinfo.get_port() as u8
        };
        
        let vport = match self.create_port(port_name, queue_id) {
            Ok(vp) => vp,
            Err(_) => {
                return Err(ConnectError::other("could not create ALSA input port", self));
            }
        };
        
        let receiver = snd_seq_addr_t {
            client: self.seq.as_mut().unwrap().get_client_id() as u8,
            port: vport as u8
        };
        
        // Make subscription
        let mut sub = unsafe { PortSubscription::allocate() };
        sub.set_sender(&sender);
        sub.set_dest(&receiver);
        if unsafe { snd_seq_subscribe_port(self.seq.as_mut().unwrap().as_mut_ptr(), sub.as_ptr()) } != 0 {
            return Err(ConnectError::other("could not create ALSA input subscription", self));
        }
        subscription = sub;
        
        // Start the input queue
        self.start_input_queue(queue_id);

        // Start our MIDI input thread.
        let handler_data = HandlerData {
            ignore_flags: self.ignore_flags,
            seq: self.seq.take().unwrap(),
            trigger_rcv_fd: trigger_fds[0],
            callback: Box::new(callback),
            queue_id: queue_id
        };
        
        let threadbuilder = Builder::new();
        let thread = match threadbuilder.spawn(move || {
            let mut d = data;
            let h = handle_input(handler_data, &mut d);
            (h, d) // return both the handler data and the user data 
        }) {
            Ok(handle) => handle,
            Err(_) => {
                //unsafe { snd_seq_unsubscribe_port(self.seq.as_mut_ptr(), sub.as_ptr()) };
                return Err(ConnectError::other("could not start ALSA input handler thread", self));
            }
        };

        Ok(MidiInputConnection {
            subscription: Some(subscription),
            thread: Some(thread),
            vport: vport,
            trigger_send_fd: trigger_fds[1]
        })
    }
}

impl PortInfo for MidiInput {
    fn new(client_name: &str) -> Result<Self, super::InitError> {
        Self::new(client_name)
    }
    
    fn port_count(&self) -> u32 {
        self.port_count()
    }
    
    fn port_name(&self, port_number: u32) -> Result<String, PortInfoError> {
        self.port_name(port_number)
    }
}

impl<T: Send> InputConnect<T> for MidiInput {
    type Connection = MidiInputConnection<T>; 
    
    fn connect<F>(
        self, port_number: u32, port_name: &str, callback: F, data: T
    ) -> Result<Self::Connection, ConnectError<Self>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        self.connect(port_number, port_name, callback, data)
    }
}

impl<T: Send> ::os::nix::VirtualInput<T> for MidiInput {
    type Connection = MidiInputConnection<T>;
    
    fn create_virtual<F>(
        mut self, port_name: &str, callback: F, data: T
    ) -> Result<Self::Connection, ConnectError<Self>>
    where F: FnMut(f64, &[u8], &mut T) + Send + 'static {
        let trigger_fds = match self.init_trigger() {
            Ok(fds) => fds,
            Err(()) => { return Err(ConnectError::other("could not create communication pipe for ALSA handler", self)); }
        };
        
        let queue_id = self.init_queue();
        
        let vport = match self.create_port(port_name, queue_id) {
            Ok(vp) => vp,
            Err(_) => {
                return Err(ConnectError::other("could not create ALSA input port", self));
            }
        };
        
        // Start the input queue
        self.start_input_queue(queue_id);
        
        // Start our MIDI input thread.
        let handler_data = HandlerData {
            ignore_flags: self.ignore_flags,
            seq: self.seq.take().unwrap(),
            trigger_rcv_fd: trigger_fds[0],
            callback: Box::new(callback),
            queue_id: queue_id
        };
        
        let threadbuilder = Builder::new();
        let thread = match threadbuilder.spawn(move || {
            let mut d = data;
            let h = handle_input(handler_data, &mut d);
            (h, d) // return both the handler data and the user data 
        }) {
            Ok(handle) => handle,
            Err(_) => {
                //unsafe { snd_seq_unsubscribe_port(self.seq.as_mut_ptr(), sub.as_ptr()) };
                return Err(ConnectError::other("could not start ALSA input handler thread", self));
            }
        };

        Ok(MidiInputConnection {
            subscription: None,
            thread: Some(thread),
            vport: vport,
            trigger_send_fd: trigger_fds[1]
        })
    }
}

impl<T> MidiInputConnection<T> {
    pub fn close(mut self) -> (MidiInput, T) {
        let (handler_data, user_data) = self.close_internal();
        
        (MidiInput {
            ignore_flags: handler_data.ignore_flags,
            seq: Some(handler_data.seq),
        }, user_data)
    }
    
    /// This must only be called if the handler thread has not yet been shut down
    fn close_internal(&mut self) -> (HandlerData<T>, T) {
        // Request the thread to stop.
        let _res = unsafe { ::libc::write(self.trigger_send_fd, mem::transmute(&false), mem::size_of::<bool>() as ::libc::size_t) };
        
        let thread = self.thread.take().unwrap(); 
        // Join the thread to get the handler_data back
        let (mut handler_data, user_data) = thread.join().unwrap(); // TODO: don't use unwrap here
        
        // TODO: find out why snd_seq_unsubscribe_port takes a long time if there was not yet any input message
        match self.subscription.as_mut() {
            Some(sub) => unsafe { snd_seq_unsubscribe_port(handler_data.seq.as_mut_ptr(), sub.as_ptr()); },
            None => ()
        }
        
        // Close the trigger fds (TODO: make sure that these are closed even in the presence of panic in thread)
        unsafe {
            ::libc::close(handler_data.trigger_rcv_fd);
            ::libc::close(self.trigger_send_fd);
        }
        
        // Stop and free the input queue
        if !cfg!(feature = "avoid_timestamping") {
            unsafe {
                snd_seq_stop_queue(handler_data.seq.as_mut_ptr(), handler_data.queue_id, ptr::null_mut());
                snd_seq_drain_output(handler_data.seq.as_mut_ptr());
                snd_seq_free_queue(handler_data.seq.as_mut_ptr(), handler_data.queue_id);
            }
        }
        
        // Delete the port
        unsafe { snd_seq_delete_port(handler_data.seq.as_mut_ptr(), self.vport) };
        
        (handler_data, user_data)
    }
}


impl<T> Drop for MidiInputConnection<T> {
    fn drop(&mut self) {
        // Use `self.thread` as a flag whether the connection has already been dropped
        if self.thread.is_some() {
            self.close_internal();
        }
    }
}

impl<T> InputConnection<T> for MidiInputConnection<T> {
    type Input = MidiInput;
    
    fn close(self) -> (Self::Input, T) {
        self.close()
    }
}

pub struct MidiOutput {
    seq: Option<Sequencer>, // TODO: if `Sequencer` is marked as non-zero, this should just be pointer-sized 
}

pub struct MidiOutputConnection {
    seq: Option<Sequencer>,
    vport: i32,
    coder: EventEncoder,
    subscription: Option<PortSubscription>
}

impl MidiOutput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        let mut seq = match Sequencer::open(SequencerOpenMode::Output, true) {
            Ok(s) => s,
            Err(_) => { return Err(InitError); }
        };
        
        // Set client name.
        seq.set_client_name(client_name);
        
        Ok(MidiOutput {
            seq: Some(seq),
        })
    }
    
    pub fn port_count(&self) -> u32 {
        unsafe {
            let mut pinfo = APortInfo::allocate();
            get_port_info(self.seq.as_ref().unwrap(), &mut pinfo, SND_SEQ_PORT_CAP_WRITE|SND_SEQ_PORT_CAP_SUBS_WRITE, -1).unwrap() as u32
        }
    }
    
    pub fn port_name(&self, port_number: u32) -> Result<String, PortInfoError> {
        match get_port_name(&self.seq.as_ref().unwrap(), SND_SEQ_PORT_CAP_WRITE|SND_SEQ_PORT_CAP_SUBS_WRITE, port_number as i32) {
            Ok(s) => Ok(s),
            Err(()) => Err(PortInfoError::PortNumberOutOfRange)
        }
    }
    
    pub fn connect(mut self, port_number: u32, port_name: &str) -> Result<MidiOutputConnection, ConnectError<MidiOutput>> {
        let mut pinfo = unsafe { APortInfo::allocate() };
        
        if get_port_info(self.seq.as_ref().unwrap(), &mut pinfo, SND_SEQ_PORT_CAP_WRITE|SND_SEQ_PORT_CAP_SUBS_WRITE, port_number as i32).is_none() {
            return Err(ConnectError::new(ConnectErrorKind::PortNumberOutOfRange, self));
        }
        
        let receiver = snd_seq_addr_t {
            client: pinfo.get_client() as u8,
            port: pinfo.get_port() as u8
        };
        
        let vport = match self.seq.as_mut().unwrap().create_simple_port(port_name,
                            SND_SEQ_PORT_CAP_READ|SND_SEQ_PORT_CAP_SUBS_READ,
                            SND_SEQ_PORT_TYPE_MIDI_GENERIC|SND_SEQ_PORT_TYPE_APPLICATION) {
            Ok(vport) => vport,
            Err(_) => {
                return Err(ConnectError::other("could not create ALSA output port", self));
            }
        };
        
        let sender = snd_seq_addr_t {
            client: self.seq.as_ref().unwrap().get_client_id() as u8,
            port: vport as u8
        };
        
        // Make subscription
        let mut sub = unsafe { PortSubscription::allocate() };
        sub.set_sender(&sender);
        sub.set_dest(&receiver);
        sub.set_time_update(true);
        sub.set_time_real(true);
        if unsafe { snd_seq_subscribe_port(self.seq.as_mut().unwrap().as_mut_ptr(), sub.as_ptr()) } != 0 {
            return Err(ConnectError::other("could not create ALSA output subscription", self));
        }
        
        Ok(MidiOutputConnection {
            seq: self.seq.take(),
            vport: vport,
            coder: EventEncoder::new(INITIAL_CODER_BUFFER_SIZE),
            subscription: Some(sub)
        })
    }
}

impl PortInfo for MidiOutput {
    fn new(client_name: &str) -> Result<Self, super::InitError> {
        Self::new(client_name)
    }
    
    fn port_count(&self) -> u32 {
        self.port_count()
    }
    
    fn port_name(&self, port_number: u32) -> Result<String, PortInfoError> {
        self.port_name(port_number)
    }
}

impl OutputConnect for MidiOutput {
    type Connection = MidiOutputConnection; 
    
     fn connect(
        self, port_number: u32, port_name: &str
    ) -> Result<Self::Connection, super::ConnectError<Self>> {
        self.connect(port_number, port_name)
    }
}

impl ::os::nix::VirtualOutput for MidiOutput {
    type Connection = MidiOutputConnection;
    
    fn create_virtual(
        mut self, port_name: &str
    ) -> Result<Self::Connection, ConnectError<Self>> {
        let vport = match self.seq.as_mut().unwrap().create_simple_port(port_name,
                            SND_SEQ_PORT_CAP_READ|SND_SEQ_PORT_CAP_SUBS_READ,
                            SND_SEQ_PORT_TYPE_MIDI_GENERIC|SND_SEQ_PORT_TYPE_APPLICATION) {
            Ok(vport) => vport,
            Err(_) => {
                return Err(ConnectError::other("could not create ALSA output port", self));
            }
        };
        
        Ok(MidiOutputConnection {
            seq: self.seq.take(),
            vport: vport,
            coder: EventEncoder::new(INITIAL_CODER_BUFFER_SIZE),
            subscription: None
        })
    }
}
        

impl MidiOutputConnection {
    pub fn close(mut self) -> MidiOutput {
        self.close_internal();
        
        MidiOutput {
            seq: self.seq.take(),
        }
    }
    
    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {        
        let nbytes = message.len();
        
        if nbytes > self.coder.get_buffer_size() {
            if self.coder.resize_buffer(nbytes).is_err() {
                return Err(SendError::Other("could not resize ALSA encoding buffer"));
            }
        }
        
        let mut ev = Event::new();
        ev.set_source(self.vport as u8);
        ev.set_subs();
        ev.set_direct();
        
        if self.coder.encode(message, &mut ev).is_err() {
            return Err(SendError::InvalidData("ALSA encoder reported invalid data"));
        }
        
        // Send the event.
        if self.seq.as_mut().unwrap().event_output(&ev).is_err() {
            return Err(SendError::Other("could not send encoded ALSA message"));
        }
        
        self.seq.as_mut().unwrap().drain_output();
        Ok(())
    }
    
    fn close_internal(&mut self) {
        unsafe {
            let seq = self.seq.as_mut().unwrap();
            if self.subscription.is_some() {
                snd_seq_unsubscribe_port(seq.as_mut_ptr(), self.subscription.as_mut().unwrap().as_ptr());
            }
            snd_seq_delete_port(seq.as_mut_ptr(), self.vport);
        }
    }
}

impl Drop for MidiOutputConnection {
    fn drop(&mut self) {
        if self.seq.is_some() {
            self.close_internal();
        }
    }
}

impl OutputConnection for MidiOutputConnection {
    type Output = MidiOutput;
    
    fn close(self) -> Self::Output {
        self.close()
    }
    
    fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        self.send(message)
    }   
}

fn handle_input<'a, T>(mut data: HandlerData<T>, user_data: &mut T) -> HandlerData<T> {
    let mut last_time: Option<u64> = None;
    let mut continue_sysex: bool = false;
    
    let mut buffer = unsafe {
        let mut vec = Vec::with_capacity(INITIAL_CODER_BUFFER_SIZE);
        vec.set_len(INITIAL_CODER_BUFFER_SIZE);
        vec.into_boxed_slice()
    };
    
    let mut coder = EventDecoder::new(false);
    
    let mut poll_fds: Box<[pollfd]>;
    unsafe {
        let poll_fd_count = (data.seq.poll_descriptors_count(POLLIN) + 1) as usize;
        let mut vec = Vec::with_capacity(poll_fd_count);
        vec.set_len(poll_fd_count);
        poll_fds = vec.into_boxed_slice();
    }
    data.seq.poll_descriptors(&mut poll_fds[1..], POLLIN); 
    poll_fds[0].fd = data.trigger_rcv_fd;
    poll_fds[0].events = POLLIN;
    
    let mut do_input = true;
    while do_input {
        if unsafe { snd_seq_event_input_pending(data.seq.as_mut_ptr(), 1) } == 0 {
            // No data pending
            if poll(&mut poll_fds, -1) >= 0 {
                // Read from our "channel" whether we should stop the thread 
                if poll_fds[0].revents & POLLIN != 0 {
                    let _res = unsafe { ::libc::read(poll_fds[0].fd, mem::transmute(&mut do_input), mem::size_of::<bool>() as ::libc::size_t) };
                }
            }
            continue;
        }

        // If here, there should be data.
        let mut ev = match data.seq.event_input() {
            Ok((ev, _)) => ev,
            Err(e) if e == -::libc::consts::os::posix88::ENOSPC => {
                let _ = writeln!(stderr(), "\nError in handle_input: ALSA MIDI input buffer overrun!\n");
                continue;
            },
            Err(_) => {
                let _ = writeln!(stderr(), "\nError in handle_input: unknown ALSA MIDI input error!\n");
                //perror("System reports");
                continue;
            }
        };
        
        let mut message = MidiMessage::new();

        // This is a bit weird, but we now have to decode an ALSA MIDI
        // event (back) into MIDI bytes. We'll ignore non-MIDI types.
        if !continue_sysex { message.bytes.clear() }
        
        let ignore_flags = data.ignore_flags;
        let do_decode = match ev._type as u32 {
            SND_SEQ_EVENT_PORT_SUBSCRIBED => {
                if cfg!(debug) { println!("Notice from handle_input: ALSA port connection made!") };
                false
            },
            SND_SEQ_EVENT_PORT_UNSUBSCRIBED => {
                if cfg!(debug) {
                    let _ = writeln!(stderr(), "Notice from handle_input: ALSA port connection has closed!");
                    let connect = unsafe { &*ev.data.connect() };
                    println!("sender = {}:{}, dest = {}:{}",
                        connect.sender.client,
                        connect.sender.port,
                        connect.dest.client,
                        connect.dest.port
                    );
                }
                false
            },
            SND_SEQ_EVENT_QFRAME => { // MIDI time code
                !ignore_flags.contains(Ignore::Time)
            },
            SND_SEQ_EVENT_TICK => { // 0xF9 ... MIDI timing tick
                !ignore_flags.contains(Ignore::Time)
            },
            SND_SEQ_EVENT_CLOCK => { // 0xF8 ... MIDI timing (clock) tick
                !ignore_flags.contains(Ignore::Time)
            },
            SND_SEQ_EVENT_SENSING => { // Active sensing
                !ignore_flags.contains(Ignore::ActiveSense)
            },
            SND_SEQ_EVENT_SYSEX => {
                if ignore_flags.contains(Ignore::Sysex) { false }
                else {
                    let data_len = unsafe { (*ev.data.ext()).len } as usize;
                    let buffer_len = buffer.len();
                    if data_len > buffer_len {
                        // Resize buffer
                        buffer = unsafe {
                            let mut vec = Vec::with_capacity(data_len);
                            vec.set_len(data_len);
                            vec.into_boxed_slice()
                        };
                        true
                    } else { true }
                }
            }
            _ => true
        };

        if do_decode {
            let nbytes = coder.decode(&mut buffer, &mut *ev);
            
            if nbytes > 0 {
                // The ALSA sequencer has a maximum buffer size for MIDI sysex
                // events of 256 bytes. If a device sends sysex messages larger
                // than this, they are segmented into 256 byte chunks.    So,
                // we'll watch for this and concatenate sysex chunks into a
                // single sysex message if necessary.
                if !continue_sysex {
                    message.bytes.clear();
                }
                
                // TODO: use push_all or whatever it gets replaced with, when stable 
                for b in &buffer[0..nbytes] {
                    message.bytes.push(*b);
                }
                
                continue_sysex = ( ev._type as u32 == SND_SEQ_EVENT_SYSEX ) && ( *message.bytes.last().unwrap() != 0xF7 );
                if !continue_sysex {
                    // Calculate the time stamp:
                    // Use the ALSA sequencer event time data.
                    // (thanks to Pedro Lopez-Cabanillas!).
                    let alsa_time = unsafe { &*ev.time.time() };
                    let timestamp = ( alsa_time.tv_sec as u64 * 1_000_000 ) + ( alsa_time.tv_nsec as u64/1_000 );
                    message.timestamp = match last_time {
                        None => 0.0,
                        Some(last) => (timestamp - last) as f64 * 0.000001
                    };
                    last_time = Some(timestamp);
                } else {
                    // TODO: this doesn't make sense
                    if cfg!(debug) {
                        let _ = writeln!(stderr(), "\nError in handle_input: event parsing error or not a MIDI event!\n");
                    }
                }
            }
        }

        drop(ev);
        if message.bytes.len() == 0 || continue_sysex { continue; }
        
        (data.callback)(message.timestamp, &message.bytes, user_data);
    }
    data // return data back to thread owner
}
