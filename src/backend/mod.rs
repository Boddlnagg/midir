// This module is not public

// TODO: improve feature selection (make sure that there is always exactly one implementation, or enable dynamic backend selection)
// TODO: allow to disable build dependency on ALSA

#[cfg(all(target_os = "windows", not(feature = "winrt")))]
mod winmm;
#[cfg(all(target_os = "windows", not(feature = "winrt")))]
pub use self::winmm::*;

#[cfg(all(target_os = "windows", feature = "winrt"))]
mod winrt;
#[cfg(all(target_os = "windows", feature = "winrt"))]
pub use self::winrt::*;

#[cfg(all(target_os = "macos", not(feature = "jack")))]
mod coremidi;
#[cfg(all(target_os = "macos", not(feature = "jack")))]
pub use self::coremidi::*;

#[cfg(all(target_os = "ios", not(feature = "jack")))]
mod coremidi;
#[cfg(all(target_os = "ios", not(feature = "jack")))]
pub use self::coremidi::*;

// linux: ALSA always compiles in. With the `jack` feature, JACK becomes an
// additional runtime-selectable backend instead of replacing ALSA (both can
// be present on a real system at once). See `linux::Backend`.
#[cfg(target_os = "linux")]
mod alsa;
#[cfg(all(target_os = "linux", not(feature = "jack")))]
pub use self::alsa::*;

#[cfg(all(feature = "jack", not(target_os = "windows")))]
mod jack;
#[cfg(all(feature = "jack", not(any(target_os = "windows", target_os = "linux"))))]
pub use self::jack::*;

#[cfg(all(target_os = "linux", feature = "jack"))]
mod linux;
#[cfg(all(target_os = "linux", feature = "jack"))]
pub use self::linux::*;

#[cfg(target_arch = "wasm32")]
mod webmidi;
#[cfg(target_arch = "wasm32")]
pub use self::webmidi::*;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use self::android::*;
