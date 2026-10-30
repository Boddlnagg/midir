//! Regenerates `src/backend/wms/bindings.rs`, the Rust bindings for the parts of the
//! Windows MIDI Services API (`Windows.Devices.Midi2`) that midir's `wms` backend uses.
//!
//! Only maintainers need to run this, when the backend starts using another type or when the
//! `windows-core` version changes. Everyone else builds from the checked-in file.
//!
//! Usage:
//!
//! ```text
//! cargo run --manifest-path tools/wms-bindgen/Cargo.toml -- <path to Windows.Devices.Midi2.winmd>
//! ```
//!
//! The `.winmd` file is in the Windows MIDI Services SDK NuGet package
//! (`ref/native/Windows.Devices.Midi2.winmd`) on the Windows MIDI Services releases page:
//! <https://aka.ms/midirepo>. Use the package whose version the README names.

use std::path::PathBuf;

/// Types the backend uses directly. Their dependencies are included automatically.
const TYPES: &[&str] = &[
    "Windows.Devices.Midi2.MidiApi",
    "Windows.Devices.Midi2.MidiClock",
    "Windows.Devices.Midi2.MidiSession",
    "Windows.Devices.Midi2.MidiEndpointConnection",
    "Windows.Devices.Midi2.MidiMessageReceivedEventArgs",
    "Windows.Devices.Midi2.IMidiMessageReceivedEventSource",
    "Windows.Devices.Midi2.MidiSendMessageResults",
    "Windows.Devices.Midi2.MidiGroup",
    "Windows.Devices.Midi2.Enumeration.Midi1PortFlow",
    "Windows.Devices.Midi2.Enumeration.Legacy.MidiLegacyPortDeviceInformation",
    // Used by the backend's tests to create a loopback endpoint pair.
    "Windows.Devices.Midi2.Transports.Loopback.MidiLoopbackManager",
    "Windows.Devices.Midi2.Transports.Loopback.MidiLoopbackCreationConfig",
    "Windows.Devices.Midi2.Transports.Loopback.MidiLoopbackEndpointDefinition",
    "Windows.Devices.Midi2.Transports.Loopback.MidiLoopbackRemovalConfig",
];

fn main() {
    let winmd = std::env::args()
        .nth(1)
        .expect("pass the path to Windows.Devices.Midi2.winmd");
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("src")
        .join("backend")
        .join("wms")
        .join("bindings.rs");

    let mut args: Vec<String> = vec![
        "--in".into(),
        "default".into(),
        "--in".into(),
        winmd,
        "--out".into(),
        out.to_string_lossy().into_owned(),
        "--filter".into(),
    ];
    args.extend(TYPES.iter().map(|t| t.to_string()));

    let warnings = windows_bindgen::bindgen(args);
    print!("{warnings}");
    println!("Wrote {}", out.display());
}
