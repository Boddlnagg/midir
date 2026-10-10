//! Linux only. ALSA is always compiled in; with the `jack` feature, JACK
//! becomes an *additional* backend instead of replacing ALSA, since both can
//! be present on a real system at once (e.g. `a2jmidid` bridges ALSA seq
//! clients into JACK). `Backend` lets a caller pick which one to open at
//! runtime. See https://github.com/Boddlnagg/midir/issues/4.

use super::alsa;
use super::jack;
use crate::errors::{ConnectError, ConnectErrorKind};
use crate::{Ignore, InitError, PortInfoError, SendError};

/// Which native Linux MIDI implementation a `MidiInput`/`MidiOutput` talks
/// to. Both are always compiled in on Linux when this crate is built with
/// the `jack` feature; nothing here disables ALSA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    #[default]
    Alsa,
    Jack,
}

#[derive(Clone, PartialEq)]
pub enum MidiInputPort {
    Alsa(alsa::MidiInputPort),
    Jack(jack::MidiInputPort),
}

impl MidiInputPort {
    pub fn id(&self) -> String {
        match self {
            MidiInputPort::Alsa(p) => p.id(),
            MidiInputPort::Jack(p) => p.id(),
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum MidiOutputPort {
    Alsa(alsa::MidiOutputPort),
    Jack(jack::MidiOutputPort),
}

impl MidiOutputPort {
    pub fn id(&self) -> String {
        match self {
            MidiOutputPort::Alsa(p) => p.id(),
            MidiOutputPort::Jack(p) => p.id(),
        }
    }
}

pub enum MidiInput {
    Alsa(alsa::MidiInput),
    Jack(jack::MidiInput),
}

impl MidiInput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        Self::new_with_backend(client_name, Backend::default())
    }

    pub fn new_with_backend(client_name: &str, backend: Backend) -> Result<Self, InitError> {
        Ok(match backend {
            Backend::Alsa => MidiInput::Alsa(alsa::MidiInput::new(client_name)?),
            Backend::Jack => MidiInput::Jack(jack::MidiInput::new(client_name)?),
        })
    }

    pub fn ignore(&mut self, flags: Ignore) {
        match self {
            MidiInput::Alsa(imp) => imp.ignore(flags),
            MidiInput::Jack(imp) => imp.ignore(flags),
        }
    }

    pub(crate) fn ports_internal(&self) -> Vec<crate::common::MidiInputPort> {
        match self {
            MidiInput::Alsa(imp) => imp
                .ports_raw()
                .into_iter()
                .map(|imp| crate::common::MidiInputPort {
                    imp: MidiInputPort::Alsa(imp),
                })
                .collect(),
            MidiInput::Jack(imp) => imp
                .ports_raw()
                .into_iter()
                .map(|imp| crate::common::MidiInputPort {
                    imp: MidiInputPort::Jack(imp),
                })
                .collect(),
        }
    }

    pub fn port_count(&self) -> usize {
        match self {
            MidiInput::Alsa(imp) => imp.port_count(),
            MidiInput::Jack(imp) => imp.port_count(),
        }
    }

    pub fn port_name(&self, port: &MidiInputPort) -> Result<String, PortInfoError> {
        match (self, port) {
            (MidiInput::Alsa(imp), MidiInputPort::Alsa(port)) => imp.port_name(port),
            (MidiInput::Jack(imp), MidiInputPort::Jack(port)) => imp.port_name(port),
            _ => Err(PortInfoError::InvalidPort),
        }
    }

    pub fn connect<F, T: Send>(
        self,
        port: &MidiInputPort,
        port_name: &str,
        callback: F,
        data: T,
    ) -> Result<MidiInputConnection<T>, ConnectError<Self>>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        match (self, port) {
            (MidiInput::Alsa(imp), MidiInputPort::Alsa(port)) => imp
                .connect(port, port_name, callback, data)
                .map(|conn| MidiInputConnection::Alsa(conn))
                .map_err(|err| {
                    let kind = err.kind();
                    ConnectError::new(kind, MidiInput::Alsa(err.into_inner()))
                }),
            (MidiInput::Jack(imp), MidiInputPort::Jack(port)) => imp
                .connect(port, port_name, callback, data)
                .map(|conn| MidiInputConnection::Jack(conn))
                .map_err(|err| {
                    let kind = err.kind();
                    ConnectError::new(kind, MidiInput::Jack(err.into_inner()))
                }),
            (this, _) => Err(ConnectError::new(ConnectErrorKind::InvalidPort, this)),
        }
    }

    pub fn create_virtual<F, T: Send>(
        self,
        port_name: &str,
        callback: F,
        data: T,
    ) -> Result<MidiInputConnection<T>, ConnectError<Self>>
    where
        F: FnMut(u64, &[u8], &mut T) + Send + 'static,
    {
        match self {
            MidiInput::Alsa(imp) => imp
                .create_virtual(port_name, callback, data)
                .map(|conn| MidiInputConnection::Alsa(conn))
                .map_err(|err| {
                    let kind = err.kind();
                    ConnectError::new(kind, MidiInput::Alsa(err.into_inner()))
                }),
            MidiInput::Jack(imp) => imp
                .create_virtual(port_name, callback, data)
                .map(|conn| MidiInputConnection::Jack(conn))
                .map_err(|err| {
                    let kind = err.kind();
                    ConnectError::new(kind, MidiInput::Jack(err.into_inner()))
                }),
        }
    }
}

pub enum MidiInputConnection<T: 'static> {
    Alsa(alsa::MidiInputConnection<T>),
    Jack(jack::MidiInputConnection<T>),
}

impl<T> MidiInputConnection<T> {
    pub fn close(self) -> (MidiInput, T) {
        match self {
            MidiInputConnection::Alsa(conn) => {
                let (imp, data) = conn.close();
                (MidiInput::Alsa(imp), data)
            }
            MidiInputConnection::Jack(conn) => {
                let (imp, data) = conn.close();
                (MidiInput::Jack(imp), data)
            }
        }
    }
}

pub enum MidiOutput {
    Alsa(alsa::MidiOutput),
    Jack(jack::MidiOutput),
}

impl MidiOutput {
    pub fn new(client_name: &str) -> Result<Self, InitError> {
        Self::new_with_backend(client_name, Backend::default())
    }

    pub fn new_with_backend(client_name: &str, backend: Backend) -> Result<Self, InitError> {
        Ok(match backend {
            Backend::Alsa => MidiOutput::Alsa(alsa::MidiOutput::new(client_name)?),
            Backend::Jack => MidiOutput::Jack(jack::MidiOutput::new(client_name)?),
        })
    }

    pub(crate) fn ports_internal(&self) -> Vec<crate::common::MidiOutputPort> {
        match self {
            MidiOutput::Alsa(imp) => imp
                .ports_raw()
                .into_iter()
                .map(|imp| crate::common::MidiOutputPort {
                    imp: MidiOutputPort::Alsa(imp),
                })
                .collect(),
            MidiOutput::Jack(imp) => imp
                .ports_raw()
                .into_iter()
                .map(|imp| crate::common::MidiOutputPort {
                    imp: MidiOutputPort::Jack(imp),
                })
                .collect(),
        }
    }

    pub fn port_count(&self) -> usize {
        match self {
            MidiOutput::Alsa(imp) => imp.port_count(),
            MidiOutput::Jack(imp) => imp.port_count(),
        }
    }

    pub fn port_name(&self, port: &MidiOutputPort) -> Result<String, PortInfoError> {
        match (self, port) {
            (MidiOutput::Alsa(imp), MidiOutputPort::Alsa(port)) => imp.port_name(port),
            (MidiOutput::Jack(imp), MidiOutputPort::Jack(port)) => imp.port_name(port),
            _ => Err(PortInfoError::InvalidPort),
        }
    }

    pub fn connect(
        self,
        port: &MidiOutputPort,
        port_name: &str,
    ) -> Result<MidiOutputConnection, ConnectError<Self>> {
        match (self, port) {
            (MidiOutput::Alsa(imp), MidiOutputPort::Alsa(port)) => imp
                .connect(port, port_name)
                .map(MidiOutputConnection::Alsa)
                .map_err(|err| {
                    let kind = err.kind();
                    ConnectError::new(kind, MidiOutput::Alsa(err.into_inner()))
                }),
            (MidiOutput::Jack(imp), MidiOutputPort::Jack(port)) => imp
                .connect(port, port_name)
                .map(MidiOutputConnection::Jack)
                .map_err(|err| {
                    let kind = err.kind();
                    ConnectError::new(kind, MidiOutput::Jack(err.into_inner()))
                }),
            (this, _) => Err(ConnectError::new(ConnectErrorKind::InvalidPort, this)),
        }
    }

    pub fn create_virtual(
        self,
        port_name: &str,
    ) -> Result<MidiOutputConnection, ConnectError<Self>> {
        match self {
            MidiOutput::Alsa(imp) => imp
                .create_virtual(port_name)
                .map(MidiOutputConnection::Alsa)
                .map_err(|err| {
                    let kind = err.kind();
                    ConnectError::new(kind, MidiOutput::Alsa(err.into_inner()))
                }),
            MidiOutput::Jack(imp) => imp
                .create_virtual(port_name)
                .map(MidiOutputConnection::Jack)
                .map_err(|err| {
                    let kind = err.kind();
                    ConnectError::new(kind, MidiOutput::Jack(err.into_inner()))
                }),
        }
    }
}

pub enum MidiOutputConnection {
    Alsa(alsa::MidiOutputConnection),
    Jack(jack::MidiOutputConnection),
}

impl MidiOutputConnection {
    pub fn close(self) -> MidiOutput {
        match self {
            MidiOutputConnection::Alsa(conn) => MidiOutput::Alsa(conn.close()),
            MidiOutputConnection::Jack(conn) => MidiOutput::Jack(conn.close()),
        }
    }

    pub fn send(&mut self, message: &[u8]) -> Result<(), SendError> {
        match self {
            MidiOutputConnection::Alsa(conn) => conn.send(message),
            MidiOutputConnection::Jack(conn) => conn.send(message),
        }
    }
}
