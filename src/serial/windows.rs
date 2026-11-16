//! Windows serial port via the Win32 communications API (kernel32).

use super::SerialSettings;
use crate::cli::Parity;
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;

#[repr(C)]
#[derive(Default)]
struct Dcb {
    dcb_length: u32,
    baud_rate: u32,
    flags: u32,
    w_reserved: u16,
    xon_lim: u16,
    xoff_lim: u16,
    byte_size: u8,
    parity: u8,
    stop_bits: u8,
    xon_char: i8,
    xoff_char: i8,
    error_char: i8,
    eof_char: i8,
    evt_char: i8,
    w_reserved1: u16,
}

#[repr(C)]
struct CommTimeouts {
    read_interval_timeout: u32,
    read_total_timeout_multiplier: u32,
    read_total_timeout_constant: u32,
    write_total_timeout_multiplier: u32,
    write_total_timeout_constant: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCommState(h: *mut c_void, dcb: *mut Dcb) -> i32;
    fn SetCommState(h: *mut c_void, dcb: *const Dcb) -> i32;
    fn SetCommTimeouts(h: *mut c_void, t: *const CommTimeouts) -> i32;
    fn PurgeComm(h: *mut c_void, flags: u32) -> i32;
}

// DCB bit fields
const F_BINARY: u32 = 1 << 0;
const F_PARITY: u32 = 1 << 1;
const DTR_CONTROL_ENABLE: u32 = 1 << 4;
const RTS_CONTROL_ENABLE: u32 = 1 << 12;

const PURGE_RXABORT: u32 = 0x0002;
const PURGE_RXCLEAR: u32 = 0x0008;

/// Read poll interval: a read returns 0 bytes after this many ms without data.
const POLL_MS: u32 = 50;

pub struct SerialPort {
    file: File,
}

fn check(ret: i32) -> io::Result<()> {
    if ret == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl SerialPort {
    pub fn open(name: &str, s: &SerialSettings) -> io::Result<Self> {
        // COM10 and above (and all ports, to be safe) need the device namespace prefix.
        let path = if name.starts_with(r"\\") { name.to_string() } else { format!(r"\\.\{name}") };
        let file = OpenOptions::new().read(true).write(true).share_mode(0).open(&path)?;
        let h = file.as_raw_handle();

        let mut dcb = Dcb { dcb_length: std::mem::size_of::<Dcb>() as u32, ..Default::default() };
        unsafe {
            check(GetCommState(h, &mut dcb))?;
        }
        dcb.baud_rate = s.baud;
        dcb.byte_size = s.data_bits;
        dcb.parity = match s.parity {
            Parity::None => 0,
            Parity::Odd => 1,
            Parity::Even => 2,
        };
        dcb.stop_bits = if s.stop_bits == 2 { 2 } else { 0 };
        dcb.flags = F_BINARY | DTR_CONTROL_ENABLE | RTS_CONTROL_ENABLE;
        if s.parity != Parity::None {
            dcb.flags |= F_PARITY;
        }

        let timeouts = CommTimeouts {
            // MAXDWORD/MAXDWORD/constant: return immediately with available bytes,
            // otherwise wait up to `constant` ms for the first byte.
            read_interval_timeout: u32::MAX,
            read_total_timeout_multiplier: u32::MAX,
            read_total_timeout_constant: POLL_MS,
            write_total_timeout_multiplier: 0,
            write_total_timeout_constant: 2000,
        };
        unsafe {
            check(SetCommState(h, &dcb)).map_err(|e| {
                io::Error::new(e.kind(), format!("unable to apply serial settings: {e}"))
            })?;
            check(SetCommTimeouts(h, &timeouts))?;
        }
        Ok(SerialPort { file })
    }

    pub fn clear_input(&mut self) -> io::Result<()> {
        unsafe { check(PurgeComm(self.file.as_raw_handle(), PURGE_RXABORT | PURGE_RXCLEAR)) }
    }
}

impl Read for SerialPort {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Write for SerialPort {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
