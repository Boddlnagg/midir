// This module is not public

// TODO: improve feature selection (make sure that there is always exactly one implementation, or enable dynamic backend selection)
// TODO: allow to disable build dependency on ALSA

#[cfg(all(target_os="windows", not(any(feature = "winrt", feature = "winjack"))))] mod winmm;
#[cfg(all(target_os="windows", not(any(feature = "winrt", feature = "winjack"))))] pub use self::winmm::*;

#[cfg(all(target_os="windows", feature = "winrt", not(feature = "winjack")))] mod winrt;
#[cfg(all(target_os="windows", feature = "winrt", not(feature = "winjack")))] pub use self::winrt::*;

#[cfg(all(target_os="macos", not(feature = "jack")))] mod coremidi;
#[cfg(all(target_os="macos", not(feature = "jack")))] pub use self::coremidi::*;

#[cfg(all(target_os="linux", not(feature = "jack")))] mod alsa;
#[cfg(all(target_os="linux", not(feature = "jack")))] pub use self::alsa::*;

#[cfg(all(feature = "jack", any(unix, feature = "winjack")))] mod jack;
#[cfg(all(feature = "jack", any(unix, feature = "winjack")))] pub use self::jack::*;

#[cfg(target_arch="wasm32")] mod webmidi;
#[cfg(target_arch="wasm32")] pub use self::webmidi::*;
