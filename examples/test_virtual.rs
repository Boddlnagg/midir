extern crate midir;

use std::thread::sleep;
use std::time::Duration;
use std::error::Error;

use midir::{MidiInput, MidiOutput, Ignore};
use midir::os::unix::{VirtualInput, VirtualOutput};

fn main() {
    match run() {
        Ok(_) => (),
        Err(err) => println!("Error: {}", err.description())
    }
}

fn run() -> Result<(), Box<Error>> {
    let mut midi_in = try!(MidiInput::new("My Test Input"));
    midi_in.ignore(Ignore::None);
    let midi_out = try!(MidiOutput::new("My Test Output"));
    
    let previous_count = midi_out.port_count();
    
    println!("Creating virtual input port ...");
    let conn_in = try!(midi_in.create_virtual("midir-test", |stamp, message, _| {
        println!("{}: {:?} (len = {})", stamp, message, message.len());
    }, ()));
    
    assert_eq!(midi_out.port_count(), previous_count + 1);
    
    println!("Connecting to port '{}' ...", midi_out.port_name(previous_count).unwrap());
    let mut conn_out = try!(midi_out.connect(previous_count, "midir-test"));
    println!("Starting to send messages ...");
    try!(conn_out.send(&[144, 60, 1]));
    sleep(Duration::from_millis(200));
    try!(conn_out.send(&[144, 60, 0]));
    sleep(Duration::from_millis(200));
    println!("Closing output ...");
    let midi_out = conn_out.close();
    println!("Closing virtual input ...");
    let midi_in = conn_in.close().0;
    assert_eq!(midi_out.port_count(), previous_count);
    
    let previous_count = midi_in.port_count();
    
    println!("\nCreating virtual output port ...");
    let mut conn_out = try!(midi_out.create_virtual("midir-test"));
    assert_eq!(midi_in.port_count(), previous_count + 1);
    
    println!("Connecting to port '{}' ...", midi_in.port_name(previous_count).unwrap());
    let conn_in = try!(midi_in.connect(previous_count, "midir-test", |stamp, message, _| {
        println!("{}: {:?} (len = {})", stamp, message, message.len());
    }, ()));
    println!("Starting to send messages ...");
    try!(conn_out.send(&[144, 60, 1]));
    sleep(Duration::from_millis(200));
    try!(conn_out.send(&[144, 60, 0]));
    sleep(Duration::from_millis(200));
    println!("Closing input ...");
    let midi_in = conn_in.close().0;
    println!("Closing virtual output ...");
    conn_out.close();
    assert_eq!(midi_in.port_count(), previous_count);
    Ok(())
}