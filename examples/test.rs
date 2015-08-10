extern crate midir;

use std::thread::sleep_ms;
use std::io::{stdin, stdout, Write};

use midir::{MidiInput, MidiOutput, Ignore};

// TODO: better error handling using try! macro for all possible failures and printing actual error message
fn main() {
    let mut input = String::new();
    
    let mut midi_in = MidiInput::new("My Test Input");
    midi_in.ignore(Ignore::None);
    let mut midi_out = MidiOutput::new("My Test Output");
    
    println!("Available input ports:");
    for i in 0..midi_in.port_count() {
        println!("{}: {}", i, midi_in.port_name(i).unwrap());
    }
    print!("Please select input port: ");
    stdout().flush();
    stdin().read_line(&mut input);
    let in_port: u32 = input.trim().parse().unwrap();
    
    println!("\nAvailable output ports:");
    for i in 0..midi_out.port_count() {
        println!("{}: {}", i, midi_out.port_name(i).unwrap());
    }
    print!("Please select output port: ");
    stdout().flush();
    input.clear();
    stdin().read_line(&mut input);
    let out_port: u32 = input.trim().parse().unwrap();
    
    let mut midi_in = Some(midi_in);
    let mut midi_out = Some(midi_out);
    
    // This shows how to reuse input and output objects
    for _ in 0..2 {
        println!("\nOpening connections");
        let conn_in = match midi_in.unwrap().connect(in_port, "midir-test", |stamp, message, _| {
            println!("{}: {:?} (len = {})", stamp, message, message.len());
        }, ()) {
            Ok(c) => c,
            Err(err) => {
                println!("Error opening input connection.");
                return;
            }
        };
        
        let mut conn_out = match midi_out.unwrap().connect(out_port, "midir-test") {
            Ok(c) => c,
            Err(err) => {
                println!("Error opening output connection.");
                return;
            }
        };
        
        println!("Connections open, enter `q` to exit ...");
        
        loop {
            input.clear();
            stdin().read_line(&mut input).unwrap();
            if (input.trim() == "q") {
                break;
            } else {
                conn_out.send_message(&[144, 60, 1]);
                sleep_ms(200);
                conn_out.send_message(&[144, 60, 0]);
            }
        }
        println!("Closing connections");
        midi_in = Some(conn_in.close().0);
        midi_out = Some(conn_out.close());
        println!("Connections closed");
    }
}