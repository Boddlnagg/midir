# midir [![crates.io](https://img.shields.io/crates/v/midir.svg)](https://crates.io/crates/midir)

Cross-platform, realtime MIDI processing in Rust.

## Features
**midir** is inspired by [RtMidi](https://github.com/thestk/rtmidi) and supports the same features*, including virtual ports (except on Windows) and full SysEx support – but with a rust-y API!

<sup>* With the exception of message queues, but these can be implemented on top of callbacks using e.g. Rust's channels.</sup>

**midir** currently supports the following platforms/backends:
- [x] ALSA (Linux)
- [x] WinMM (Windows)
- [x] CoreMIDI (macOS, iOS)
- [x] WinRT (Windows 8+), enable the `winrt` feature
- [x] Windows MIDI Services (Windows 11 25H2+, 64-bit), enable the `winmidi2` feature
- [x] Jack (Linux, macOS), enable the `jack` feature
- [x] Web MIDI (Chrome, Opera, perhaps others browsers)
- [x] Android (API 29+, NDK AMidi + JNI)

A higher-level API for parsing and assembling MIDI messages might be added in the future.

## Documentation & Example
API docs can be found at [docs.rs](https://docs.rs/crate/midir/). You can find some examples in the [`examples`](examples/) directory. Or simply run `cargo run --example test_play` after cloning this repository.

### Android
- Requires Android API 29+ and the Android NDK (r20b+).
- Build (example, to remove before merging):
  - Install: `cargo install cargo-ndk`
  - Targets: `rustup target add aarch64-linux-android`
  - Build: `cargo ndk -t arm64-v8a -o ./app/src/main/jniLibs build --release`
- Permissions/features:
  - Manifest should declare `<uses-feature android:name="android.software.midi" android:required="false" />` (not needed for USB/BLE MIDI).

### Windows MIDI Services (Preview)
The `winmidi2` feature uses [Windows MIDI Services](https://aka.ms/midi) (`Windows.Devices.Midi2`) instead of WinMM. midir's API stays the same. You see the same MIDI 1.0 ports that WinMM apps see, with the same names and in the same order. The one exception is the Microsoft GS Wavetable Synth, which only WinMM lists, so an output port's position in the list can be one lower than with the WinMM backend. Don't save a port's position to find it again later, with any backend: positions change whenever a device is plugged in or removed. Save the port's id or name instead. Messages from MIDI 2.0 devices are converted to MIDI 1.0, and timestamps are in microseconds, like the other backends.

What it needs:
- **Windows 11 25H2 or later, 64-bit (x64 or Arm64).** On older versions of Windows, `MidiInput::new` and `MidiOutput::new` return an error.
- **Windows MIDI Services in its normal mode.** If the PC is set to Legacy API mode, `MidiInput::new` and `MidiOutput::new` return an error.
- **The API DLL, until it's part of Windows.** That's planned for 2027. Until then, your app has to ship `Windows.Devices.Midi2.dll` and `Windows.Devices.Midi2.pri` in the same folder as its `.exe`. They're in the `runtimes\win-x64\native` and `runtimes\win-arm64\native` folders of the `Windows.Devices.Midi2` NuGet package on the [Windows MIDI Services releases page](https://github.com/microsoft/MIDI/releases). When Windows has its own copy, Windows uses that one instead of yours, so your app keeps working. Shipping these files is your choice as the app developer, and the release notes list the terms that come with it.

The `winmidi2` feature can't be combined with `winrt`. Virtual ports aren't supported yet.

#### Ports change while your app runs

midir can't tell your app when a port is added, removed or renamed. That's a gap in midir today, with every backend. Most of this was already true with WinMM, but it happens much more with Windows MIDI Services:
- **People can rename ports.** A port's name can change while your app is running.
- **Ports come and go.** Bluetooth LE MIDI, Network MIDI 2.0 and other newer transports add and remove ports often. Windows keeps the WinMM port numbers in one unbroken run, so when a port goes away, the numbers after it move down. A device that's removed and comes back often gets a different number.
- **Don't check for new ports in a tight loop.** Each full check is hard on Windows' Plug and Play system. If it runs on your UI thread and the PC has more than a few ports, your app can stop responding, and Windows will mark it as not responding.

Windows MIDI Services has a port watcher (`MidiLegacyPortDeviceWatcher`) for exactly these reasons. The `winmidi2` backend uses one behind the scenes. It keeps its own port list up to date, so `ports()`, `port_count()` and `port_name()` read that list instead of asking Plug and Play each time. The first of these calls in a process waits for the watcher's first full list. Still, call them when you need them (for example when the user opens a port menu), not on a timer.

To regenerate the bindings in `src/backend/winmidi2/bindings.rs`, see [`tools/winmidi2-bindgen`](tools/winmidi2-bindgen/src/main.rs).
