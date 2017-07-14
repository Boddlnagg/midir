extern crate midir;

use std::thread::sleep;
use std::time::Duration;
use std::io::{stdin, stdout, Write};
use std::error::Error;
use std::sync::mpsc::channel;

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
    let mut midi_out = try!(MidiOutput::new("My Test Output"));
    
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
    
    // This shows how to reuse input and output objects:
    // Open/close the connections twice using the same MidiInput/MidiOutput objects
    for _ in 0..2 {
        println!("\nOpening connections");
        let (sender, receiver) = channel(); // We use this as an example custom data to pass into the callback

        let conn_in = try!(midi_in.connect(in_port, "midir-test", move |stamp, message| {
            // The last of the three callback parameters is the object that we pass in as last parameter of `connect`.
            println!("{}: {:?} (len = {})", stamp, message, message.len());
            for b in message { let _ = sender.send(*b); }
        }).map_err(|e| e.kind()));
        
        // One could get the log back here out of the error
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
        midi_in = conn_in.close();
        midi_out = conn_out.close();
        println!("Connections closed");

        let all_received_bytes = receiver.try_iter().collect::<Vec<u8>>();
        println!("Received bytes: {:?}", all_received_bytes);
    }
    Ok(())
}