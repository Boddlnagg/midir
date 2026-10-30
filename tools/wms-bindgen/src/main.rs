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

/// Types the backend uses. windows-bindgen does not add the types that a method takes or returns:
/// it skips the method instead and says so, so list every type a used method needs.
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
    "Windows.Devices.Midi2.Transports.Loopback.MidiLoopbackCreationResponse",
    "Windows.Devices.Midi2.Transports.Loopback.MidiLoopbackRemovalResponse",
    "Windows.Devices.Midi2.Transports.Loopback.MidiLoopbackEntry",
    "Windows.Devices.Midi2.Transports.Loopback.MidiLoopbackEndpointEntry",
    // Generic, so not pulled in by the types that use it.
    "Windows.Foundation.TypedEventHandler",
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

    let generated = std::fs::read_to_string(&out).expect("read the generated bindings");
    let (fixed, count) = fix_noexcept_statics(&generated);
    std::fs::write(&out, fixed).expect("write the bindings");
    println!("Made {count} static methods return windows_core::Result");

    match std::process::Command::new("rustfmt")
        .args(["--edition", "2021"])
        .arg(&out)
        .status()
    {
        Ok(status) if status.success() => {}
        other => println!("warning: rustfmt did not run cleanly: {other:?}"),
    }
    println!("Wrote {}", out.display());
}

/// windows-bindgen 0.65 writes broken code for static methods marked `[NoException]`: the
/// method returns the plain value, but the factory cache it calls through returns
/// `windows_core::Result`. The API marks many static methods this way. Getting the activation
/// factory can still fail (for example when the API isn't installed), so make these methods
/// return `windows_core::Result` like every other static method.
fn fix_noexcept_statics(source: &str) -> (String, usize) {
    let lines: Vec<&str> = source.lines().collect();
    let mut out: Vec<Option<String>> = lines.iter().map(|l| Some(l.to_string())).collect();
    let mut count = 0;

    for j in 0..lines.len() {
        let call = lines[j].trim();
        if !(call.starts_with("Self::I") && call.ends_with("(|this| unsafe {")) {
            continue;
        }
        let Some(i) = (0..j)
            .rev()
            .find(|&k| lines[k].trim_start().starts_with("pub fn "))
        else {
            continue;
        };
        let signature = lines[i..j]
            .iter()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join(" ");
        let Some(signature) = result_signature(&signature) else {
            continue;
        };
        let indent = &lines[j][..lines[j].len() - lines[j].trim_start().len()];
        let close = format!("{indent}}})");
        let Some(k) = (j + 1..lines.len()).find(|&k| lines[k] == close) else {
            panic!("no end found for the static method at line {}", i + 1);
        };

        out[i] = Some(signature);
        for line in &mut out[i + 1..j] {
            *line = None;
        }
        out[j] = Some(format!("{} Ok({{", lines[j]));
        out[k] = Some(format!("{indent}}}) }})"));
        count += 1;
    }

    let mut text = out.into_iter().flatten().collect::<Vec<_>>().join("\n");
    text.push('\n');
    (text, count)
}

/// Returns the signature with its return type wrapped in `windows_core::Result`, or `None` if it
/// already returns one.
fn result_signature(signature: &str) -> Option<String> {
    let open = signature.find('(')?;
    let mut depth = 0;
    let mut close = None;
    for (n, c) in signature[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + n);
                    break;
                }
            }
            _ => {}
        }
    }
    let (head, rest) = signature.split_at(close? + 1);
    let rest = rest.trim_start();
    let tail_start = rest
        .find("where ")
        .or_else(|| rest.rfind('{'))
        .unwrap_or(rest.len());
    let (returns, tail) = rest.split_at(tail_start);
    let returns = returns.trim();
    let returns = match returns.strip_prefix("->") {
        Some(r) => r.trim(),
        None if returns.is_empty() => "()",
        None => panic!("unexpected signature: {signature}"),
    };
    if returns.starts_with("windows_core::Result<") {
        return None;
    }
    Some(format!("{head} -> windows_core::Result<{returns}> {tail}"))
}
