//! Fallback for platforms without a serial implementation.

use super::SerialSettings;
use std::io::{self, Read, Write};

pub struct SerialPort;

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "serial ports are not supported on this platform")
}

impl SerialPort {
    pub fn open(_name: &str, _s: &SerialSettings) -> io::Result<Self> {
        Err(unsupported())
    }
    pub fn clear_input(&mut self) -> io::Result<()> {
        Err(unsupported())
    }
}

impl Read for SerialPort {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(unsupported())
    }
}

impl Write for SerialPort {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(unsupported())
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(unsupported())
    }
}
