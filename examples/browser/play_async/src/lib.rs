use js_sys::{Array, Promise};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::console;

use std::error::Error;

use midir::MidiOutput;

pub fn log(s: String) {
    console::log(&Array::of1(&s.into()));
}

macro_rules! println {
    ()              => (log("".to_owned()));
    ($($arg:tt)*)   => (log(format!($($arg)*)));
}

#[wasm_bindgen(start)]
pub async fn start() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));

    run().await.unwrap();
}

async fn run() -> Result<(), Box<dyn Error>> {
    let window = web_sys::window().expect("no global `window` exists");

    let midi_out = MidiOutput::new_async("midir output").await?;

    // Get an input port
    let ports = midi_out.ports();
    let out_port = match &ports[..] {
        [] => {
            println!("No ports available yet");
            return Ok(());
        }
        [ref port] => {
            println!(
                "Choosing the only available output port: {}",
                midi_out.port_name(port).unwrap()
            );
            port
        }
        _ => {
            let mut msg = "Choose an available output port:\n".to_string();
            for (i, port) in ports.iter().enumerate() {
                msg.push_str(format!("{}: {}\n", i, midi_out.port_name(port).unwrap()).as_str());
            }
            loop {
                if let Ok(Some(port_str)) = window.prompt_with_message_and_default(&msg, "0") {
                    if let Ok(port_int) = port_str.parse::<usize>() {
                        if let Some(port) = ports.get(port_int) {
                            break port;
                        }
                    }
                }
            }
        }
    };

    println!("\nOpening connection");
    let mut conn_out = midi_out.connect(out_port, "midir-test")?;
    println!("Connection open. Listen!");

    const NOTE_ON_MSG: u8 = 0x90;
    const NOTE_OFF_MSG: u8 = 0x80;
    const VELOCITY: u8 = 0x64;

    for (midi_note, duration) in [
        (66, 4),
        (65, 3),
        (63, 1),
        (61, 6),
        (59, 2),
        (58, 4),
        (56, 4),
        (54, 4),
    ] {
        let _ = conn_out.send(&[NOTE_ON_MSG, midi_note, VELOCITY]);
        sleep(duration * 150).await;
        let _ = conn_out.send(&[NOTE_OFF_MSG, midi_note, VELOCITY]);
    }

    // sleep(Duration::from_millis(150));
    println!("\nClosing connection");

    // This is optional, the connection would automatically be closed as soon as it goes out of scope
    conn_out.close();
    println!("Connection closed");
    Ok(())
}

async fn sleep(ms: u64) {
    let window = web_sys::window().unwrap();
    let promise = Promise::new(&mut |resolve, _| {
        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32)
            .unwrap();
    });
    JsFuture::from(promise).await.unwrap();
}
