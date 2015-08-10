extern crate midir;

use std::thread::sleep_ms;
use std::io::stdin;

use midir::alsa::{MidiInput, MidiOutput};
use midir::Ignore;

fn main() {
    let mut midi_in = MidiInput::new("My Test");
    midi_in.ignore(Ignore::None);
    let mut midi_out = MidiOutput::new("My Test");
    
    println!("Input devices:");
    for i in 0..midi_in.port_count() {
        println!("{}: {}", i, midi_in.port_name(i).unwrap());
    }
    
    println!("\nOutput devices:");
    for i in 0..midi_out.port_count() {
        println!("{}: {}", i, midi_out.port_name(i).unwrap());
    }
    
    let mut midi_in = Some(midi_in);
    let mut midi_out = Some(midi_out);
    
    // This shows how to reuse input and output objects
    for _ in 0..2 {
        println!("\nOpening connections");
        let conn_in = match midi_in.unwrap().connect(2, "RtMidi", |stamp, message, _| {
            println!("{}: {:?} (len = {})", stamp, message, message.len());
        }, ()) {
            Ok(c) => c,
            Err(err) => {
                println!("Error opening input connection.");
                return;
            }
        };
        
        let mut conn_out = match midi_out.unwrap().connect(2, "RtMidi") {
            Ok(c) => c,
            Err(err) => {
                println!("Error opening output connection.");
                return;
            }
        };
        
        println!("Connections open, enter `q` to exit ...");
        let mut input = String::new();
        loop {
            stdin().read_line(&mut input).unwrap();
            if (input.trim() == "q") {
                break;
            } else {
                conn_out.send_message(&[144, 60, 1]);
                sleep_ms(200);
                conn_out.send_message(&[144, 60, 0]);
            }
            input.clear();
        }
        println!("Closing connections");
        midi_in = Some(conn_in.close().0);
        midi_out = Some(conn_out.close());
        println!("Connections closed");
    }
}