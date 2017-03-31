# midir [![crates.io](https://img.shields.io/crates/v/midir.svg)](https://crates.io/crates/midir) [![Travis Build Status](https://travis-ci.org/Boddlnagg/midir.svg?branch=master)](https://travis-ci.org/Boddlnagg/midir?branch=master) [![AppVeyor Build status](https://ci.appveyor.com/api/projects/status/atit0teb38s2am2y/branch/master?svg=true)](https://ci.appveyor.com/project/Boddlnagg/midir)

Cross-platform, realtime MIDI processing in Rust.

## Features
**midir** is inspired by [RtMidi](https://github.com/thestk/rtmidi) and supports the same features*, including virtual ports (except on Windows) and full SysEx support – but with a rust-y API!

<sup>* With the exception of message queues, but these can be implemented on top of callbacks using e.g. Rust's channels.</sup>

**midir** currently supports the following platforms/backends: 
- [x] ALSA (Linux)
- [x] WinMM (Windows)
- [ ] CoreMIDI (OS X, iOS), see [this issue](https://github.com/Boddlnagg/midir/issues/1)
- [x] Jack (Linux, OS X), use the `jack` feature

A higher-level API for parsing and assembling MIDI messages might be added in the future.

## Example
Have a look at [`examples/test.rs`](examples/test.rs) and run it directly using `cargo run --example test`.
