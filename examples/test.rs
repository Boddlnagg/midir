extern crate midir;

use std::thread::sleep_ms;

use midir::{MidiApi, MidiInApi};
use midir::alsa::MidiInAlsa;

fn main() {
	let mut x = MidiInAlsa::new("My Test", 100).unwrap();
	let count = x.get_port_count();
	println!("Device count: {}", count);
	for i in 0..count {
		println!("{}: {}", i, x.get_port_name(i).unwrap());
	}
	println!("Opening port");
	x.open_port(2, "RtMidi").unwrap();
	println!("Port open");
	
	let mut message = Vec::new();
	
  	for _ in 0..500 {
	    let stamp = x.get_message(&mut message);
		if (message.len() > 0) {
			println!("{}: {:?}", stamp, message);
		}
	    // Sleep for 10 milliseconds ... platform-dependent.
	    sleep_ms(10);
  	}
	
	println!("Closing port");
	x.close_port();
	println!("Port closed");
}