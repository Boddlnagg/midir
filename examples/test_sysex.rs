extern crate midir;

fn main() {
    match example::run() {
        Ok(_) => (),
        Err(err) => println!("Error: {}", err.description())
    }
}

#[cfg(not(windows))] // virtual ports are not supported on Windows
mod example {

use std::thread::sleep;
use std::time::Duration;
use std::error::Error;

use midir::{MidiInput, MidiOutput, Ignore};
use midir::os::unix::VirtualInput;

const LARGE_SYSEX_SIZE: usize = 5572; // This is the maximum that worked for me

pub fn run() -> Result<(), Box<Error>> {
    let mut midi_in = MidiInput::new("My Test Input")?;
    midi_in.ignore(Ignore::None);
    let midi_out = MidiOutput::new("My Test Output")?;
    
    let previous_count = midi_out.port_count();
    
    println!("Creating virtual input port ...");
    let conn_in = midi_in.create_virtual("midir-test", |stamp, message, _| {
        println!("{}: {:?} (len = {})", stamp, message, message.len());
    }, ())?;
    
    assert_eq!(midi_out.port_count(), previous_count + 1);
    
    println!("Connecting to port '{}' ...", midi_out.port_name(previous_count).unwrap());
    let mut conn_out = midi_out.connect(previous_count, "midir-test")?;
    println!("Starting to send messages ...");
    //sleep(Duration::from_millis(2000));
    println!("Sending NoteOn message");
    conn_out.send(&[144, 60, 1])?;
    sleep(Duration::from_millis(200));
    println!("Sending small SysEx message ...");
    conn_out.send(&[0xF0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xF7])?;
    sleep(Duration::from_millis(200));
    println!("Sending large SysEx message ...");
    let mut v = Vec::with_capacity(LARGE_SYSEX_SIZE);
    v.push(0xF0u8);
    for _ in 1..LARGE_SYSEX_SIZE-1 {
        v.push(0u8);
    }
    v.push(0xF7u8);
    assert_eq!(v.len(), LARGE_SYSEX_SIZE);
    conn_out.send(&v)?;
    sleep(Duration::from_millis(200));
    println!("Sending large SysEx message (chunked)...");
    for ch in v.chunks(4) {
        conn_out.send(ch)?;
    }
    sleep(Duration::from_millis(200));
    println!("Sending small SysEx message ...");
    conn_out.send(&[0xF0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xF7])?;
    sleep(Duration::from_millis(200));
    println!("Closing output ...");
    conn_out.close();
    println!("Closing virtual input ...");
    conn_in.close().0;
    Ok(())
}
}

 // needed to compile successfully
#[cfg(windows)] mod example {
    use std::error::Error;
    pub fn run() -> Result<(), Box<Error>> { Ok(()) }
}
