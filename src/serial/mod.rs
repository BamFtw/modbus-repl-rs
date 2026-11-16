//! Serial port access implemented directly on top of the OS APIs
//! (Win32 comm API on Windows, termios on Linux) - no external crates.

use crate::cli::Parity;

#[derive(Debug, Clone)]
pub struct SerialSettings {
    pub baud: u32,
    pub parity: Parity,
    pub data_bits: u8,
    pub stop_bits: u8,
}

impl SerialSettings {
    /// Number of bits on the wire per character (start + data + parity + stop).
    pub fn bits_per_char(&self) -> u32 {
        1 + self.data_bits as u32 + (self.parity != Parity::None) as u32 + self.stop_bits as u32
    }
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::SerialPort;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::SerialPort;

#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported;
#[cfg(not(any(windows, target_os = "linux")))]
pub use unsupported::SerialPort;
