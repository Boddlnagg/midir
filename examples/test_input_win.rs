extern crate midir;

use std::thread::sleep_ms;
use std::io::stdin;

use midir::{MidiApi, MidiInApi};
use midir::winmm::MidiInWinMM;

fn main() {
    let mut midi = MidiInWinMM::new("My Test", 100).unwrap();
    let count = midi.get_port_count();
    println!("Number of input devices: {}", count);
    for i in 0..count {
        println!("{}: {}", i, midi.get_port_name(i).unwrap());
    }
    println!("Opening port");
    midi.open_port(0, "RtMidi").unwrap();
    midi.ignore_types(false, false, false);
    midi.set_callback(|stamp, message| {
        println!("{}: {:?} (len = {})", stamp, message, message.len());
    });
    println!("Port open, press ENTER to exit ...");
    stdin().read_line(&mut String::new()).unwrap();
    println!("Closing port");
    midi.close_port();
    println!("Port closed");
}