extern crate midir;

use std::thread::sleep_ms;

use midir::{MidiApi, MidiOutApi};
use midir::alsa::MidiOutAlsa;

fn main() {
    let mut midi = MidiOutAlsa::new("My Test").unwrap();
    let count = midi.get_port_count();
    println!("Number of output devices: {}", count);
    for i in 0..count {
        println!("{}: {}", i, midi.get_port_name(i).unwrap());
    }
    println!("Opening port");
    midi.open_port(2, "RtMidi").unwrap();
    println!("Port open");
    
    for i in 0..25 {
        midi.send_message(&[144, 60, 1]);
        sleep_ms(200);
        midi.send_message(&[144, 60, 0]);
        sleep_ms(100);
    }
    
    println!("Closing port");
    midi.close_port();
    println!("Port closed");
}