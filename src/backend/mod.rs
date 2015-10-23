// This module is not public

// TODO: improve feature selection (make sure that there is always exactly one implementation, or enable dynamic backend selection)
// TODO: allow to disable build dependency on ALSA

#[cfg(target_os="windows")] mod winmm;
#[cfg(target_os="windows")] pub use self::winmm::*;

#[cfg(all(target_os="linux", not(feature = "jack")))] mod alsa;
#[cfg(all(target_os="linux", not(feature = "jack")))] pub use self::alsa::*;

#[cfg(all(feature = "jack", not(target_os="windows")))] mod jack;
#[cfg(all(feature = "jack", not(target_os="windows")))] pub use self::jack::*;