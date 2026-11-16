//! Linux serial port via termios (declared by hand, works with glibc and musl).
//!
//! Constants below are the Linux "asm-generic" values used by x86, x86_64,
//! ARM, AArch64 and RISC-V.

use super::SerialSettings;
use crate::cli::Parity;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; 32],
    c_ispeed: u32,
    c_ospeed: u32,
    // Safety margin in case libc's struct is larger than expected.
    _reserved: [u8; 64],
}

extern "C" {
    fn tcgetattr(fd: i32, t: *mut Termios) -> i32;
    fn tcsetattr(fd: i32, optional_actions: i32, t: *const Termios) -> i32;
    fn cfmakeraw(t: *mut Termios);
    fn cfsetispeed(t: *mut Termios, speed: u32) -> i32;
    fn cfsetospeed(t: *mut Termios, speed: u32) -> i32;
    fn tcflush(fd: i32, queue: i32) -> i32;
    fn tcdrain(fd: i32) -> i32;
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
}

const O_NOCTTY: i32 = 0o400;
const O_NONBLOCK: i32 = 0o4000;
const F_GETFL: i32 = 3;
const F_SETFL: i32 = 4;

const TCSANOW: i32 = 0;
const TCIFLUSH: i32 = 0;

const CSIZE: u32 = 0o60;
const CS5: u32 = 0o0;
const CS6: u32 = 0o20;
const CS7: u32 = 0o40;
const CS8: u32 = 0o60;
const CSTOPB: u32 = 0o100;
const CREAD: u32 = 0o200;
const PARENB: u32 = 0o400;
const PARODD: u32 = 0o1000;
const CLOCAL: u32 = 0o4000;
const CRTSCTS: u32 = 0o20000000000;
const IXON: u32 = 0o2000;
const IXOFF: u32 = 0o10000;
const IXANY: u32 = 0o4000;
const INPCK: u32 = 0o20;

const VTIME: usize = 5;
const VMIN: usize = 6;

fn baud_constant(baud: u32) -> Option<u32> {
    Some(match baud {
        300 => 0o7,
        600 => 0o10,
        1200 => 0o11,
        2400 => 0o13,
        4800 => 0o14,
        9600 => 0o15,
        19200 => 0o16,
        38400 => 0o17,
        57600 => 0o10001,
        115200 => 0o10002,
        230400 => 0o10003,
        460800 => 0o10004,
        500000 => 0o10005,
        576000 => 0o10006,
        921600 => 0o10007,
        1000000 => 0o10010,
        1152000 => 0o10011,
        1500000 => 0o10012,
        2000000 => 0o10013,
        _ => return None,
    })
}

fn check(ret: i32) -> io::Result<i32> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub struct SerialPort {
    file: File,
}

impl SerialPort {
    pub fn open(name: &str, s: &SerialSettings) -> io::Result<Self> {
        let speed = baud_constant(s.baud).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported baud rate {}", s.baud))
        })?;

        // O_NONBLOCK so open() doesn't wait for carrier detect; cleared below.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NOCTTY | O_NONBLOCK)
            .open(name)?;
        let fd = file.as_raw_fd();

        unsafe {
            let mut t: Termios = std::mem::zeroed();
            check(tcgetattr(fd, &mut t))?;
            cfmakeraw(&mut t);
            t.c_iflag &= !(IXON | IXOFF | IXANY | INPCK);
            t.c_cflag &= !(CSIZE | CSTOPB | PARENB | PARODD | CRTSCTS);
            t.c_cflag |= CREAD | CLOCAL;
            t.c_cflag |= match s.data_bits {
                5 => CS5,
                6 => CS6,
                7 => CS7,
                _ => CS8,
            };
            if s.stop_bits == 2 {
                t.c_cflag |= CSTOPB;
            }
            match s.parity {
                Parity::None => {}
                Parity::Even => {
                    t.c_cflag |= PARENB;
                    t.c_iflag |= INPCK;
                }
                Parity::Odd => {
                    t.c_cflag |= PARENB | PARODD;
                    t.c_iflag |= INPCK;
                }
            }
            // Non-blocking-ish reads: return after 0.1 s without data.
            t.c_cc[VMIN] = 0;
            t.c_cc[VTIME] = 1;
            check(cfsetispeed(&mut t, speed))?;
            check(cfsetospeed(&mut t, speed))?;
            check(tcsetattr(fd, TCSANOW, &t))?;

            let flags = check(fcntl(fd, F_GETFL))?;
            check(fcntl(fd, F_SETFL, flags & !O_NONBLOCK))?;
        }
        Ok(SerialPort { file })
    }

    pub fn clear_input(&mut self) -> io::Result<()> {
        unsafe { check(tcflush(self.file.as_raw_fd(), TCIFLUSH)).map(|_| ()) }
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
        // Wait until all output has physically been transmitted.
        unsafe { check(tcdrain(self.file.as_raw_fd())).map(|_| ()) }
    }
}
