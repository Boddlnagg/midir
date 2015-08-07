#![allow(dead_code)]

use std::mem;
use std::ptr;
use std::thread::{Builder, JoinHandle};
use std::io::{stderr, Write};
use std::sync::{Arc, RwLock, Mutex};

use super::Error::*;
use super::{Result, MidiApi, MidiInApi, MidiQueue, MidiMessage};

use alsa_sys::{
  snd_seq_t,
  snd_seq_query_next_client,
  snd_seq_query_next_port,
  snd_seq_get_any_client_info,
  snd_seq_addr_t,
  snd_seq_delete_port, // TODO: wrap
  snd_seq_subscribe_port,
  snd_seq_unsubscribe_port,
  snd_seq_event_t,
  snd_seq_event_input_pending,
  snd_midi_event_decode,
  snd_seq_drain_output,
  snd_seq_alloc_named_queue,
  snd_seq_queue_tempo_t,
  snd_seq_queue_tempo_malloc,
  snd_seq_queue_tempo_free,
  snd_seq_queue_tempo_set_tempo,
  snd_seq_queue_tempo_set_ppq,
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

#[inline(always)]
unsafe fn snd_seq_stop_queue(seq: *mut snd_seq_t, q: i32, ev: *mut snd_seq_event_t) {
   snd_seq_control_queue(seq, q, SND_SEQ_EVENT_STOP as i32, 0, ev);
}

#[inline(always)]
unsafe fn snd_seq_start_queue(seq: *mut snd_seq_t, q: i32, ev: *mut snd_seq_event_t) {
   snd_seq_control_queue(seq, q, SND_SEQ_EVENT_START as i32, 0, ev);
}

// Include ALSA wrappers
mod wrappers;
use self::wrappers::{Sequencer, SequencerOpenMode};
use self::wrappers::{ClientInfo, PortInfo, PortSubscription, EventDecoder};
use self::wrappers::{pollfd, POLLIN, poll};

const SND_SEQ_OPEN_OUTPUT: i32 = 1;
const SND_SEQ_OPEN_INPUT: i32 = 2;
const SND_SEQ_OPEN_DUPLEX: i32 = SND_SEQ_OPEN_OUTPUT|SND_SEQ_OPEN_INPUT;
const SND_SEQ_NONBLOCK: i32 = 0x0001;

const SND_SEQ_PORT_TYPE_MIDI_GENERIC: u32 = 1<<1;
const SND_SEQ_PORT_TYPE_SYNTH: u32 = 1<<10;
const SND_SEQ_PORT_TYPE_APPLICATION: u32 = 1<<20;
const SND_SEQ_PORT_CAP_READ: u32 = 1<<0;
const SND_SEQ_PORT_CAP_WRITE: u32 = 1<<1;
const SND_SEQ_PORT_CAP_SYNC_READ: u32 = 1<<2;
const SND_SEQ_PORT_CAP_SYNC_WRITE: u32 = 1<<3;
const SND_SEQ_PORT_CAP_DUPLEX: u32 = 1<<4;
const SND_SEQ_PORT_CAP_SUBS_READ: u32 = 1<<5;
const SND_SEQ_PORT_CAP_SUBS_WRITE: u32 = 1<<6;
const SND_SEQ_PORT_CAP_NO_EXPORT: u32 = 1<<7;

#[derive(Debug)]
struct AlsaMidiInData {
    queue: Arc<Mutex<MidiQueue>>,
    message: MidiMessage,
    ignore_flags: Arc<RwLock<u8>>,
    do_input: Arc<RwLock<bool>>,
    first_message: bool,
    using_callback: bool,
    continue_sysex: bool,
  	last_time: u64,
    // TODO: turn into read-only pointers?
  	seq: Arc<RwLock<Sequencer>>,
  	trigger_fds: Arc<RwLock<[i32; 2]>>,
}

// TODO: is this safe?
unsafe impl Send for AlsaMidiInData {}

impl AlsaMidiInData {
	fn new(queue: Arc<Mutex<MidiQueue>>, do_input: Arc<RwLock<bool>>, seq: Arc<RwLock<Sequencer>>, trigger_fds: Arc<RwLock<[i32; 2]>>, ignore_flags: Arc<RwLock<u8>>) -> AlsaMidiInData {
		AlsaMidiInData {
			queue: queue,
			message: MidiMessage::new(),
			ignore_flags: ignore_flags,
			do_input: do_input,
			first_message: true,
			using_callback: false,
			continue_sysex: false,
			// default values:
  		last_time: 0,
			seq: seq,
			trigger_fds: trigger_fds
		}
	}
}

struct AlsaMidiData {
  seq: Arc<RwLock<Sequencer>>,
  vport: i32,
  subscription: Option<PortSubscription>,
  thread: Option<JoinHandle<()>>,
  queue_id: i32, // an input queue is needed to get timestamped events
  trigger_fds: Arc<RwLock<[i32; 2]>>,
  do_input: Arc<RwLock<bool>>,
  ignore_flags: Arc<RwLock<u8>>
}

fn alsa_midi_handler(mut data: AlsaMidiInData) {
  let mut time: u64;
  let mut last_time: u64;
  let mut continue_sysex: bool = false;
  
  let init_buffer_size = 32;
  
  let mut buffer = unsafe {
    let mut vec = Vec::with_capacity(init_buffer_size);
    vec.set_len(init_buffer_size);
    vec.into_boxed_slice()
  };
  
  let mut coder = EventDecoder::new(false);
  
  let mut poll_fds: Box<[pollfd]>;
  unsafe {
    let poll_fd_count = (data.seq.read().unwrap().poll_descriptors_count(POLLIN) + 1) as usize;
    let mut vec = Vec::with_capacity(poll_fd_count);
    vec.set_len(poll_fd_count);
    poll_fds = vec.into_boxed_slice();
  }
  data.seq.read().unwrap().poll_descriptors(&mut poll_fds[1..], POLLIN); 
  poll_fds[0].fd = data.trigger_fds.read().unwrap()[0];
  poll_fds[0].events = POLLIN;
  
  while *data.do_input.read().unwrap() {

    if unsafe { snd_seq_event_input_pending(data.seq.write().unwrap().as_mut_ptr(), 1) } == 0 {
      // No data pending
      if poll(&mut poll_fds, -1) >= 0 {
        if poll_fds[0].revents & POLLIN != 0 {
          let mut dummy: bool = unsafe { mem::uninitialized() };
          let _res = unsafe { ::libc::read(poll_fds[0].fd, mem::transmute(&mut dummy), mem::size_of::<bool>() as ::libc::size_t) };
        }
      }
      continue;
    }

    // If here, there should be data.
    let mut ev = match data.seq.write().unwrap().event_input() {
      Ok(e) => e,
      Err(e) if e == -::libc::consts::os::posix88::ENOSPC => {
        write!(stderr(), "\nMidiInAlsa::alsaMidiHandler: MIDI input buffer overrun!\n\n");
        continue;
      },
      Err(_) => {
        write!(stderr(), "\nMidiInAlsa::alsaMidiHandler: unknown MIDI input error!\n");
        //perror("System reports");
        continue;
      }
    };
    
    let mut message = MidiMessage::new();

    // This is a bit weird, but we now have to decode an ALSA MIDI
    // event (back) into MIDI bytes.  We'll ignore non-MIDI types.
    if !continue_sysex { message.bytes.clear() }
    
    let ignore_flags: u8 = *data.ignore_flags.read().unwrap();
    let do_decode = match ev.as_ref()._type as u32 {
      SND_SEQ_EVENT_PORT_SUBSCRIBED => {
        if cfg!(debug) { println!("MidiInAlsa::alsaMidiHandler: port connection made!") };
        false
      },
      SND_SEQ_EVENT_PORT_UNSUBSCRIBED => {
        if cfg!(debug) {
          writeln!(stderr(), "MidiInAlsa::alsaMidiHandler: port connection has closed!");
          let connect = unsafe { &*ev.as_ref().data.connect() };
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
        ignore_flags & 0x02 == 0
      },
      SND_SEQ_EVENT_TICK => { // 0xF9 ... MIDI timing tick
        ignore_flags & 0x02 == 0
      },
      SND_SEQ_EVENT_CLOCK => { // 0xF8 ... MIDI timing (clock) tick
        ignore_flags & 0x02 == 0
      },
      SND_SEQ_EVENT_SENSING => { // Active sensing
        ignore_flags & 0x04 == 0
      },
      SND_SEQ_EVENT_SYSEX => {
        if ignore_flags & 0x01 != 0 { false }
        else {
          let data_len = unsafe { (*ev.as_ref().data.ext()).len } as usize;
          let buffer_len = buffer.len();
          if data_len > buffer_len {
            buffer = unsafe {
              let mut vec = Vec::with_capacity(data_len);
              vec.set_len(data_len);
              vec.into_boxed_slice()
            };
            if buffer.as_ptr().is_null() {
              *data.do_input.write().unwrap() = false;
              write!(stderr(), "\nMidiInAlsa::alsaMidiHandler: error resizing buffer memory!\n\n");
              false
            } else { true }
          } else { true }
        }
      }
      _ => true
    };

    if do_decode {
      let nbytes = unsafe { snd_midi_event_decode(coder.as_ptr(), buffer.as_mut_ptr(), buffer.len() as i64, ev.as_ref()) } as usize;
      
      if nbytes > 0 {
        // The ALSA sequencer has a maximum buffer size for MIDI sysex
        // events of 256 bytes.  If a device sends sysex messages larger
        // than this, they are segmented into 256 byte chunks.  So,
        // we'll watch for this and concatenate sysex chunks into a
        // single sysex message if necessary.
        if !continue_sysex {
          message.bytes.clear();
        } 
        message.bytes.push_all(&buffer[0..nbytes]);
        
        continue_sysex = ( ev.as_ref()._type as u32 == SND_SEQ_EVENT_SYSEX ) && ( *message.bytes.last().unwrap() != 0xF7 );
        if !continue_sysex {
          // Calculate the time stamp:
          message.timestamp = 0.0;

          // Method 1: Use the system time.
          //(void)gettimeofday(&tv, (struct timezone *)NULL);
          //time = (tv.tv_sec * 1000000) + tv.tv_usec;

          // Method 2: Use the ALSA sequencer event time data.
          // (thanks to Pedro Lopez-Cabanillas!).
          let alsa_time = unsafe { &*ev.as_ref().time.time() };
          time = ( alsa_time.tv_sec as u64 * 1_000_000 ) + ( alsa_time.tv_nsec as u64/1_000 );
          last_time = time;
          time -= data.last_time;
          data.last_time = last_time;
          if data.first_message == true {
            data.first_message = false;
          } else { 
            message.timestamp = time as f64 * 0.000001;
          }
        } else {
          // TODO: this doesn't make sense
          if cfg!(debug) {
            write!(stderr(), "\nMidiInAlsa::alsaMidiHandler: event parsing error or not a MIDI event!\n\n");
          }
        }
      }
    }

    drop(ev);
    if message.bytes.len() == 0 || continue_sysex { continue; }

    // TODO!
    /*if data.usingCallback {
      
      //RtMidiIn::RtMidiCallback callback = (RtMidiIn::RtMidiCallback) data->userCallback;
      //callback( message.timeStamp, &message.bytes, data->userData );
    }
    else*/ {
      // As long as we haven't reached our queue size limit, push the message.
      let mut queue = data.queue.lock().unwrap();
      if queue.size < queue.ring.len() {
        queue.ring[queue.back as usize] = message;
        queue.back += 1;
        if queue.back == queue.ring.len() {
          queue.back = 0;
        }
        queue.size += 1;
      }
      else {
        write!(stderr(), "\nMidiInAlsa: message queue limit reached!!\n\n");
      }
    }
  }
}

#[inline(always)]
unsafe fn port_type(pinfo: &PortInfo, bits: u32) -> bool {
  (pinfo.get_capability() & bits) == bits
}

/// This function is used to count or get the pinfo structure for a given port number.
/// TODO: introduce iterator
unsafe fn port_info(seq: *const snd_seq_t, pinfo: &mut PortInfo, typ: u32, port_number: i32) -> Option<i32> {
  let mut client;
  let mut count: i32 = 0;
  let mut cinfo = ClientInfo::allocate();
  let seq = seq as *mut _;

  cinfo.set_client(-1);
  while snd_seq_query_next_client(seq, cinfo.as_ptr()) >= 0 {
    client = cinfo.get_client();
    if client == 0 { continue; }
    // Reset query info
    pinfo.set_client(client);
    pinfo.set_port(-1);
    while snd_seq_query_next_port(seq, pinfo.as_ptr()) >= 0 {
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

pub struct MidiInAlsa {
  api_data: Box<AlsaMidiData>, // TODO: should this really be a Box?
  connected: bool,
  input_data: Option<AlsaMidiInData>,
  queue: Arc<Mutex<MidiQueue>>,
}

impl MidiInAlsa {
  // TODO: first initialize MessageQueue (backend agnostic), then pass initialized queue
  unsafe fn initialize(client_name: &str, queue_size_limit: usize) -> Result<MidiInAlsa> {
    // Set up the ALSA sequencer client.
    let mut seq = match Sequencer::open(SequencerOpenMode::Duplex, true) {
      Ok(s) => s,
      Err(_) => {
        let error_string = "MidiInAlsa::initialize: error creating ALSA sequencer client object.";
        return Err(DriverError(error_string));
      }
    };
    
    seq.set_client_name(client_name);
    
    let mut trigger_fds = [-1, -1];
    
    
    if ::libc::pipe(trigger_fds.as_mut_ptr()) == -1  {
      let error_string = "MidiInAlsa::initialize: error creating pipe objects.";
      return Err(DriverError(error_string));
    }
    
    let mut queue_id = 0;  
    // Create the input queue
    if !cfg!(feature = "avoid_timestamping") {
      queue_id = snd_seq_alloc_named_queue(seq.as_mut_ptr(), mem::transmute(b"RtMidi Queue"));
      // Set arbitrary tempo (mm=100) and resolution (240)
      let mut qtempo: *mut snd_seq_queue_tempo_t = mem::uninitialized();
      snd_seq_queue_tempo_malloc(&mut qtempo);
      snd_seq_queue_tempo_set_tempo(qtempo, 600000);
      snd_seq_queue_tempo_set_ppq(qtempo, 240);
      snd_seq_set_queue_tempo(seq.as_mut_ptr(), queue_id, qtempo);
      snd_seq_queue_tempo_free(qtempo);
      snd_seq_drain_output(seq.as_mut_ptr());
    }
    
    // Save our api-specific connection information.
    let data = Box::new(AlsaMidiData {
      seq: Arc::new(RwLock::new(seq)),
      vport: -1,
      subscription: None,
      thread: None,
      trigger_fds: Arc::new(RwLock::new(trigger_fds)),
      queue_id: queue_id,
      do_input: Arc::new(RwLock::new(false)),
      ignore_flags: Arc::new(RwLock::new(7))
    });
    
    let queue = Arc::new(Mutex::new(MidiQueue::new(queue_size_limit)));
    let input_data = Some(AlsaMidiInData::new(
      queue.clone(),
      data.do_input.clone(),
      data.seq.clone(),
      data.trigger_fds.clone(),
      data.ignore_flags.clone()
    ));
    
    Ok(MidiInAlsa {
      api_data: data,
      connected: false,
      input_data: input_data,
      queue: queue
    })
  }
}

impl Drop for MidiInAlsa {
  fn drop(&mut self) {
    // Close a connection if it exists.
    self.close_port();
    let data = &*self.api_data;
  
    // Cleanup.
    unsafe {
      ::libc::close(data.trigger_fds.read().unwrap()[0]);
      ::libc::close(data.trigger_fds.read().unwrap()[1] );
    }
    let mut seq = data.seq.write().unwrap();
    if data.vport >= 0 {
      unsafe { snd_seq_delete_port(seq.as_mut_ptr(), data.vport ) };
    }
    if !cfg!(feature = "avoid_timestamping") {
      unsafe { snd_seq_free_queue(seq.as_mut_ptr(), data.queue_id ) };
    }
  }
}

impl MidiApi for MidiInAlsa {
	fn get_port_count(&self) -> u32 {
    let mut pinfo = PortInfo::allocate();
    
    unsafe {
      port_info(self.api_data.seq.read().unwrap().as_ptr(), &mut pinfo, SND_SEQ_PORT_CAP_READ|SND_SEQ_PORT_CAP_SUBS_READ, -1).unwrap() as u32
    }
  }
  
	fn get_port_name(&self, port_number: u32 /*= 0*/) -> Result<String> {
    let mut cinfo = ClientInfo::allocate();
    let mut pinfo = PortInfo::allocate();
    
    let data = &self.api_data; 
    unsafe {
      use std::fmt::Write;
      
      if port_info(data.seq.read().unwrap().as_ptr(), &mut pinfo, SND_SEQ_PORT_CAP_READ|SND_SEQ_PORT_CAP_SUBS_READ, port_number as i32).is_some() {
        let cnum: i32 = pinfo.get_client();
        snd_seq_get_any_client_info(data.seq.write().unwrap().as_mut_ptr(), cnum, cinfo.as_ptr());
        let mut os = String::new();
        write!(&mut os, "{} {}:{}", 
          cinfo.get_name(),
          pinfo.get_client(), // These lines added to make sure devices are listed
          pinfo.get_port()   // with full portnames added to ensure individual device names
        );
        Ok(os)
      } else {
        // If we get here, we didn't find a match.
        // TODO: get rid of "Warning", use better name 
        let error_string = "MidiInAlsa::getPortName: error looking for port name!";
        Err(Warning(error_string))
      }
    }
  }
  
  fn open_port(&mut self, port_number: u32 /*= 0*/, port_name: &str /*= "RtMidi"*/) -> Result<()> {
    if self.connected {
      let error_string = "MidiInAlsa::openPort: a valid connection already exists!";
      return Err(Warning(error_string));
    }
  
    // this is inefficient, since we request the port infos below anyway
    /*let nsrc = self.get_port_count();
    if nsrc < 1 {
      let error_string = "MidiInAlsa::openPort: no MIDI input sources found!";
      return Err(NoDevicesFound(error_string));
    }*/
    
    let mut src_pinfo = PortInfo::allocate();
    let data = &mut *self.api_data;
    
    if unsafe { port_info(data.seq.read().unwrap().as_ptr(), &mut src_pinfo, SND_SEQ_PORT_CAP_READ|SND_SEQ_PORT_CAP_SUBS_READ, port_number as i32) }.is_none() {
      use std::fmt::Write; 
      let mut error_string = String::new();
      write!(error_string, "MidiInAlsa::openPort: the 'portNumber' argument ({}) is invalid.", port_number); 
      return Err(InvalidParameter(error_string));
    }
    
    let sender = snd_seq_addr_t {
      client: src_pinfo.get_client() as u8,
      port: src_pinfo.get_port() as u8
    };
    
    let mut pinfo = PortInfo::allocate();
    
    if data.vport < 0 {
      pinfo.set_client(0);
      pinfo.set_port(0);
      pinfo.set_capability(SND_SEQ_PORT_CAP_WRITE | SND_SEQ_PORT_CAP_SUBS_WRITE);
      pinfo.set_type(SND_SEQ_PORT_TYPE_MIDI_GENERIC | SND_SEQ_PORT_TYPE_APPLICATION);
      pinfo.set_midi_channels(16);
      
      if !cfg!(feature = "avoid_timestamping") {
        pinfo.set_timestamping(true);
        pinfo.set_timestamp_real(true);
        pinfo.set_timestamp_queue(data.queue_id);
      }
      
      pinfo.set_name(port_name);
      data.vport = match data.seq.write().unwrap().create_port(&mut pinfo) {
        Ok(_) => pinfo.get_port(),
        Err(_) => {
          let error_string = "MidiInAlsa::openPort: ALSA error creating input port.";
          return Err(DriverError(error_string));
        }
      }
    }
    
    let receiver = snd_seq_addr_t {
      client: data.seq.read().unwrap().get_client_id() as u8,
      port: data.vport as u8
    };
  
  
    if data.subscription.is_none() {
      // Make subscription
      let mut sub = PortSubscription::allocate();
      sub.set_sender(&sender);
      sub.set_dest(&receiver);
      if unsafe { snd_seq_subscribe_port(data.seq.write().unwrap().as_mut_ptr(), sub.as_ptr()) } != 0 {
        let error_string = "MidiInAlsa::openPort: ALSA error making port connection.";
        return Err(DriverError(error_string));
      }
      data.subscription = Some(sub);
    }
  
    if *data.do_input.read().unwrap() == false {
      // Start the input queue
      if !cfg!(feature = "avoid_timestamping") {
        let mut seq = data.seq.write().unwrap();
        unsafe {
          snd_seq_start_queue(seq.as_mut_ptr(), data.queue_id, ptr::null_mut());
          snd_seq_drain_output(seq.as_mut_ptr());
        }
      }
  
      // Start our MIDI input thread.
      *data.do_input.write().unwrap() = true;
      
      let input_data = self.input_data.take().unwrap();
      
      let threadbuilder = Builder::new();
      //pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_JOINABLE);
      //pthread_attr_setschedpolicy(&attr, SCHED_OTHER);*/
      data.thread = match threadbuilder.spawn(move || {
        alsa_midi_handler(input_data);
      }) {
        Ok(handle) => Some(handle),
        Err(_) => {
          unsafe { snd_seq_unsubscribe_port(data.seq.write().unwrap().as_mut_ptr(), data.subscription.as_mut().unwrap().as_ptr()) };
          data.subscription = None;
          *data.do_input.write().unwrap() = false;
          let error_string = "MidiInAlsa::openPort: error starting MIDI input thread!";
          return Err(ThreadError(error_string));
        }
      }
    }
  
    self.connected = true;
    Ok(())
  }
  
	//fn open_virtual_port(port_name: &str/*= "RtMidi"*/);

  fn close_port(&mut self) {
    let mut data = &mut *self.api_data;
    
    if self.connected {
      let mut seq = data.seq.write().unwrap();
      if data.subscription.is_some() {
        // TODO: find out why snd_seq_unsubscribe_port takes a long time if there was not yet any input message
        unsafe { snd_seq_unsubscribe_port(seq.as_mut_ptr(), data.subscription.as_mut().unwrap().as_ptr()) };
        data.subscription = None;
      }
      // Stop the input queue
      if !cfg!(feature = "avoid_timestamping") {
        unsafe {
          snd_seq_stop_queue(seq.as_mut_ptr(), data.queue_id, ptr::null_mut());
          snd_seq_drain_output(seq.as_mut_ptr());
        }
      }
      self.connected = false;
    }
    
    let tmp_do_input;
    // Stop thread to avoid triggering the callback, while the port is intended to be closed
    {
      let mut do_input = data.do_input.write().unwrap();
      tmp_do_input = *do_input;
      if *do_input {
        *do_input = false;
        let _res = unsafe { ::libc::write(data.trigger_fds.read().unwrap()[1], mem::transmute(&*do_input), mem::size_of::<bool>() as ::libc::size_t) };  
      }
    } 
    
      // workaround for missing non-lexical borrow
    if tmp_do_input {
      //if ( !pthread_equal(data.thread, data.dummy_thread_id) )
      //  pthread_join( data.thread, NULL );
      data.thread.take().unwrap().join().unwrap();
    }
  }
  
	fn is_port_open(&self) -> bool {
    self.connected
  }
}

impl MidiInApi for MidiInAlsa {
  fn new(client_name: &str /*= "RtMidi Input Client"*/, queue_size_limit: usize /*= 100*/) -> Result<MidiInAlsa> {
    unsafe { MidiInAlsa::initialize(client_name, queue_size_limit) }
  }
	//fn set_callback<T>(callback: MidiCallback, &mut T);
	//fn cancel_callback();
	fn ignore_types(&mut self, sysex: bool /*= true*/, time: bool /*= true*/, active_sense: bool /*= true*/) {
    let mut flags = self.api_data.ignore_flags.write().unwrap();
    *flags = 0;
    if sysex { *flags = 0x01 };
    if time { *flags |= 0x02 };
    if active_sense { *flags |= 0x04 };
  }

  fn get_message(&mut self, message: &mut Vec<u8>) -> f64 {
    message.clear();

    // TODO
    /*if ( inputData_.usingCallback ) {
      errorString_ = "RtMidiIn::getNextMessage: a user callback is currently set for this port.";
      error( RtMidiError::WARNING, errorString_ );
      return 0.0;
    }*/
    
    let mut queue = self.queue.lock().unwrap();
  
    if queue.size == 0 { return 0.0; }
  
    // Copy queued message to the vector pointer argument and then "pop" it.
    message.push_all(&queue.ring[queue.front].bytes[..]);
    let delta_time = queue.ring[queue.front].timestamp;
    queue.size -= 1;
    queue.front += 1;
    if queue.front == queue.ring.len() {
      queue.front = 0;
    }
  
    delta_time
  }
}