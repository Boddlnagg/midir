extern crate console_error_panic_hook;
extern crate js_sys;
extern crate midir;
extern crate wasm_bindgen;
extern crate web_sys;

use js_sys::Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::console;

use std::error::Error;
use std::sync::{
    Arc,
    Mutex,
};

use midir::{
    Ignore,
    MidiInput,
};

pub fn log(s: String) {
    console::log(&Array::of1(&s.into()));
}

macro_rules! println {
    ()              => (log("".to_owned()));
    ($($arg:tt)*)   => (log(format!($($arg)*)));
}

#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));

    let token_outer = Arc::new(Mutex::new(None));
    let token = token_outer.clone();
    let closure: Closure<dyn FnMut()> = Closure::wrap(Box::new(move || {
        if run().unwrap() == true {
            if let Some(token) = *token.lock().unwrap() {
                web_sys::window().unwrap().clear_interval_with_handle(token);
            }
        }
    }));
    *token_outer.lock().unwrap() = web_sys::window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            200,
        )
        .ok();
    closure.forget();
}

fn run() -> Result<bool, Box<dyn Error>> {
    let window = web_sys::window().expect("no global `window` exists");

    let mut midi_in = MidiInput::new("midir reading input")?;
    midi_in.ignore(Ignore::None);

    // Get an input port
    let ports = midi_in.ports();
    let in_port = match &ports[..] {
        [] => {
            println!("No ports available yet, will try again");
            return Ok(false);
        }
        [ref port] => {
            println!(
                "Choosing the only available input port: {}",
                midi_in.port_name(port).unwrap()
            );
            port
        }
        _ => {
            let mut msg = "Choose an available input port:\n".to_string();
            for (i, port) in ports.iter().enumerate() {
                msg.push_str(format!("{}: {}\n", i, midi_in.port_name(port).unwrap()).as_str());
            }
            loop {
                if let Ok(Some(port_str)) = window.prompt_with_message_and_default(&msg, "0") {
                    if let Ok(port_int) = port_str.parse::<usize>() {
                        if let Some(port) = &ports.get(port_int) {
                            break port.clone();
                        }
                    }
                }
            }
        }
    };

    println!("Opening connection");
    let in_port_name = midi_in.port_name(in_port)?;

    // _conn_in needs to be a named parameter, because it needs to be kept alive until the end of the scope
    let _conn_in = midi_in.connect(
        in_port,
        "midir-read-input",
        move |stamp, message, _| {
            println!("{}: {:?} (len = {})", stamp, message, message.len());
        },
        (),
    )?;

    println!("Connection open, reading input from '{}'", in_port_name);
    Box::leak(Box::new(_conn_in));
    Ok(true)
}
