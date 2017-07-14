extern crate midir;

use std::thread::sleep;
use std::time::Duration;
use std::io::{stdin, stdout, Write};
use std::error::Error;

use midir::{MidiInput, MidiOutput, Ignore};

fn main() {
    match run() {
        Ok(_) => (),
        Err(err) => println!("Error: {}", err.description())
    }
}

fn run() -> Result<(), Box<Error>> {
    let mut input = String::new();
    
    let mut midi_in = try!(MidiInput::new("My Test Input"));
    midi_in.ignore(Ignore::None);
    let midi_out = try!(MidiOutput::new("My Test Output"));
    
    println!("Available input ports:");
    for i in 0..midi_in.port_count() {
        println!("{}: {}", i, midi_in.port_name(i).unwrap());
    }
    print!("Please select input port: ");
    try!(stdout().flush());
    try!(stdin().read_line(&mut input));
    let in_port: usize = try!(input.trim().parse());
    
    println!("\nAvailable output ports:");
    for i in 0..midi_out.port_count() {
        println!("{}: {}", i, midi_out.port_name(i).unwrap());
    }
    print!("Please select output port: ");
    try!(stdout().flush());
    input.clear();
    try!(stdin().read_line(&mut input));
    let out_port: usize = try!(input.trim().parse());
    
    println!("\nOpening connections");
    let conn_in = try!(midi_in.connect(in_port, "midir-test", |stamp, message, _| {
        println!("{}: {:?} (len = {})", stamp, message, message.len());
    }, ()).map_err(|e| e.kind()));
    
    let mut conn_out = try!(midi_out.connect(out_port, "midir-test").map_err(|e| e.kind()));
    
    println!("Connections open, enter `q` to exit ...");
    
    loop {
        input.clear();
        try!(stdin().read_line(&mut input));
        if input.trim() == "q" {
            break;
        } else {
            try!(conn_out.send(&[144, 60, 1]));
            sleep(Duration::from_millis(200));
            try!(conn_out.send(&[144, 60, 0]));
        }
    }
    println!("Closing connections");
    // This is optional, the connections would automatically be closed as soon as they go out of scope
    conn_in.close();
    conn_out.close();
    println!("Connections closed");
    Ok(())
}