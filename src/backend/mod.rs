// This module is not public

// TODO: improve feature selection (make sure that there is always exactly one implementation, or enable dynamic backend selection)
// TODO: allow to disable build dependency on ALSA

#[cfg(all(target_os = "windows", feature = "wms", feature = "winrt"))]
compile_error!("The `wms` and `winrt` features cannot be used together. Pick one Windows backend.");

#[cfg(all(
    target_os = "windows",
    feature = "wms",
    not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "arm64ec"
    ))
))]
compile_error!("The `wms` feature needs 64-bit Windows (x86_64, aarch64 or arm64ec).");

#[cfg(all(target_os = "windows", not(feature = "winrt"), not(feature = "wms")))]
mod winmm;
#[cfg(all(target_os = "windows", not(feature = "winrt"), not(feature = "wms")))]
pub use self::winmm::*;

#[cfg(all(target_os = "windows", feature = "winrt", not(feature = "wms")))]
mod winrt;
#[cfg(all(target_os = "windows", feature = "winrt", not(feature = "wms")))]
pub use self::winrt::*;

#[cfg(all(target_os = "windows", feature = "wms", not(feature = "winrt")))]
mod wms;
#[cfg(all(target_os = "windows", feature = "wms", not(feature = "winrt")))]
pub use self::wms::*;

#[cfg(all(target_os = "macos", not(feature = "jack")))]
mod coremidi;
#[cfg(all(target_os = "macos", not(feature = "jack")))]
pub use self::coremidi::*;

#[cfg(all(target_os = "ios", not(feature = "jack")))]
mod coremidi;
#[cfg(all(target_os = "ios", not(feature = "jack")))]
pub use self::coremidi::*;

#[cfg(all(target_os = "linux", not(feature = "jack")))]
mod alsa;
#[cfg(all(target_os = "linux", not(feature = "jack")))]
pub use self::alsa::*;

#[cfg(all(feature = "jack", not(target_os = "windows")))]
mod jack;
#[cfg(all(feature = "jack", not(target_os = "windows")))]
pub use self::jack::*;

#[cfg(target_arch = "wasm32")]
mod webmidi;
#[cfg(target_arch = "wasm32")]
pub use self::webmidi::*;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use self::android::*;
