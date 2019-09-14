extern crate midir;

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
    
    let mut midi_in = MidiInput::new("midir forwarding input")?;
    midi_in.ignore(Ignore::None);
    let midi_out = MidiOutput::new("midir forwarding output")?;
    
    println!("Available input ports:");
    let midi_in_ports = midi_in.ports();
    for (i, p) in midi_in_ports.iter().enumerate() {
        println!("{}: {}", i, midi_in.port_name(p)?);
    }
    print!("Please select input port: ");
    stdout().flush()?;
    stdin().read_line(&mut input)?;
    let in_port = midi_in_ports.get(input.trim().parse::<usize>()?)
                               .ok_or("Invalid port number")?;
    
    println!("\nAvailable output ports:");
    let midi_out_ports = midi_out.ports();
    for (i, p) in midi_out_ports.iter().enumerate() {
        println!("{}: {}", i, midi_out.port_name(p)?);
    }
    print!("Please select output port: ");
    stdout().flush()?;
    input.clear();
    stdin().read_line(&mut input)?;
    let out_port = midi_out_ports.get(input.trim().parse::<usize>()?)
                                 .ok_or("Invalid port number")?;
    
    println!("\nOpening connections");
    let in_port_name = midi_in.port_name(in_port)?;
    let out_port_name = midi_out.port_name(out_port)?;

    let mut conn_out = midi_out.connect(out_port, "midir-forward")?;

    // _conn_in needs to be a named parameter, because it needs to be kept alive until the end of the scope
    let _conn_in = midi_in.connect(in_port, "midir-forward", move |stamp, message, _| {
        conn_out.send(message).unwrap_or_else(|_| println!("Error when forwarding message ..."));
        println!("{}: {:?} (len = {})", stamp, message, message.len());
    }, ())?;
    
    println!("Connections open, forwarding from '{}' to '{}' (press enter to exit) ...", in_port_name, out_port_name);

    input.clear();
    stdin().read_line(&mut input)?; // wait for next enter key press

    println!("Closing connections");
    Ok(())
}