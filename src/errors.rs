use std::error::Error;
use std::fmt;

const PORT_OUT_OF_RANGE_MSG: &'static str = "provided port number was out of range";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitError;

impl Error for InitError {
    fn description(&self) -> &str {
        "MIDI support could not be initialized"
    }
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.description().fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortInfoError {
    PortNumberOutOfRange,
}

impl Error for PortInfoError {
    fn description(&self) -> &str {
        match *self {
            PortInfoError::PortNumberOutOfRange => PORT_OUT_OF_RANGE_MSG,
        }
    }
}

impl fmt::Display for PortInfoError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.description().fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectErrorKind {
    PortNumberOutOfRange,
    Other(&'static str)
}

impl Error for ConnectErrorKind {
    fn description(&self) -> &str {
        match *self {
            ConnectErrorKind::PortNumberOutOfRange => PORT_OUT_OF_RANGE_MSG,
            ConnectErrorKind::Other(msg) => msg
        }
    }
}

impl fmt::Display for ConnectErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.description().fmt(f)
    }
}

pub struct ConnectError<T> {
    kind: ConnectErrorKind,
    inner: T
}

impl<T> ConnectError<T> {
    pub fn new(kind: ConnectErrorKind, inner: T) -> ConnectError<T> {
        ConnectError { kind: kind, inner: inner }
    }
    
    /// Helper method to create ConnectErrorKind::Other.
    pub fn other(msg: &'static str, inner: T) -> ConnectError<T> {
        Self::new(ConnectErrorKind::Other(msg), inner)
    }
    
    pub fn kind(&self) -> ConnectErrorKind {
        self.kind
    }
    
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> fmt::Debug for ConnectError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.kind.fmt(f)
    }
}

impl<T> fmt::Display for ConnectError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.kind.fmt(f)
    }
}

// This is currently not possible in stable Rust, but instead we can directly
// implement a conversion to Box<Error> by boxing just the error kind.

//impl<T: Reflect> Error for ConnectError<T>

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    InvalidData(&'static str),
    Other(&'static str)
}

impl Error for SendError {
    fn description(&self) -> &str {
        match *self {
            SendError::InvalidData(msg) => msg,
            SendError::Other(msg) => msg
        }
    }
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.description().fmt(f)
    }
}