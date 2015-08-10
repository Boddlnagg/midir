#midir [![crates.io](https://img.shields.io/crates/v/midir.svg)](https://crates.io/crates/midir)
Cross-platform, realtime MIDI processing in Rust.

##Features
**midir** is inspired by [RtMidi](https://github.com/thestk/rtmidi) and supports the same features*, including virtual ports (except on Windows) and full SysEx support – but with a rust-y API!

<sup>* With the exception of message queues, but these can be implemented on top of callbacks using e.g. Rust's `VecDeque` – alternatively you could use channels.</sup>

**midir** currently supports the following platforms/backends: 
- [x] ALSA (Linux)
- [ ] WinMM (Windows), blocked on a [winapi-rs PR](https://github.com/retep998/winapi-rs/pull/176)
- [ ] CoreMIDI (OS X, iOS), see [this issue](https://github.com/Boddlnagg/midir/issues/1)
- [ ] Jack (Linux, OS X), see [this issue](https://github.com/Boddlnagg/midir/issues/2)

A higher-level API for parsing and assembling MIDI messages might be added in the future.

Does it work on stable Rust? Currently not, but it shouldn't be too hard to [change that](https://github.com/Boddlnagg/midir/issues/3).

## Example
Have a look at [`examples/test.rs`](examples/test.rs) and run it directly using `cargo run --example test`.
