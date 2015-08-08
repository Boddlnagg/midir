extern crate midir;

use std::thread::sleep_ms;
use std::io::stdin;

use midir::{MidiApi, MidiInApi, MidiOutApi};
use midir::winmm::{MidiInWinMM, MidiOutWinMM};

fn main() {
    let mut midi_in = MidiInWinMM::new("My Test", 100).unwrap();
    let mut midi_out = MidiOutWinMM::new("My Test").unwrap();
    let count_in = midi_in.get_port_count();
    println!("Number of input devices: {}", count_in);
    for i in 0..count_in {
        println!("{}: {}", i, midi_in.get_port_name(i).unwrap());
    }
    let count_out = midi_out.get_port_count();
    println!("Number of output devices: {}", count_out);
    for i in 0..count_out {
        println!("{}: {}", i, midi_out.get_port_name(i).unwrap());
    }
    println!("Opening ports");
    midi_in.open_port(0, "RtMidi").unwrap();
    midi_out.open_port(1, "RtMidi").unwrap();
    midi_in.ignore_types(false, false, false);
    midi_in.set_callback(|stamp, message| {
        println!("{}: {:?} (len = {})", stamp, message, message.len());
    });
    println!("Ports open, enter `q` to exit ...");
    let mut input = String::new();
    loop {
        stdin().read_line(&mut input).unwrap();
        if (input.trim() == "q") {
            break;
        } else {
            midi_out.send_message(&[144, 60, 1]);
            sleep_ms(200);
            midi_out.send_message(&[144, 60, 0]);
        }
        input.clear();
    }
    println!("Closing ports");
    midi_in.close_port();
    midi_out.close_port();
    println!("Ports closed");
}