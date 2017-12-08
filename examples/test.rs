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
    
    let mut midi_in = MidiInput::new("My Test Input")?;
    midi_in.ignore(Ignore::None);
    let midi_out = MidiOutput::new("My Test Output")?;
    
    println!("Available input ports:");
    for i in 0..midi_in.port_count() {
        println!("{}: {}", i, midi_in.port_name(i).unwrap());
    }
    print!("Please select input port: ");
    stdout().flush()?;
    stdin().read_line(&mut input)?;
    let in_port: usize = input.trim().parse()?;
    
    println!("\nAvailable output ports:");
    for i in 0..midi_out.port_count() {
        println!("{}: {}", i, midi_out.port_name(i).unwrap());
    }
    print!("Please select output port: ");
    stdout().flush()?;
    input.clear();
    stdin().read_line(&mut input)?;
    let out_port: usize = input.trim().parse()?;
    
    println!("\nOpening connections");
    let conn_in = midi_in.connect(in_port, "midir-test", |stamp, message, _| {
        println!("{}: {:?} (len = {})", stamp, message, message.len());
    }, ())?;
    
    let mut conn_out = midi_out.connect(out_port, "midir-test")?;
    
    println!("Connections open, enter `q` to exit ...");
    
    loop {
        input.clear();
        stdin().read_line(&mut input)?;
        if input.trim() == "q" {
            break;
        } else {
            conn_out.send(&[144, 60, 1])?;
            sleep(Duration::from_millis(200));
            conn_out.send(&[144, 60, 0])?;
        }
    }
    println!("Closing connections");
    // This is optional, the connections would automatically be closed as soon as they go out of scope
    conn_in.close();
    conn_out.close();
    println!("Connections closed");
    Ok(())
}