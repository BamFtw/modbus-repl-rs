//! Modbus application layer: PDU encoding / decoding, independent of transport.

use std::fmt;
use std::io;

pub const FC_READ_COILS: u8 = 0x01;
pub const FC_READ_DISCRETE_INPUTS: u8 = 0x02;
pub const FC_READ_HOLDING_REGISTERS: u8 = 0x03;
pub const FC_READ_INPUT_REGISTERS: u8 = 0x04;
pub const FC_WRITE_SINGLE_COIL: u8 = 0x05;
pub const FC_WRITE_SINGLE_REGISTER: u8 = 0x06;
pub const FC_WRITE_MULTIPLE_COILS: u8 = 0x0F;
pub const FC_WRITE_MULTIPLE_REGISTERS: u8 = 0x10;

pub const MAX_READ_BITS: u16 = 2000;
pub const MAX_READ_REGISTERS: u16 = 125;
pub const MAX_WRITE_BITS: u16 = 1968;
pub const MAX_WRITE_REGISTERS: u16 = 123;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Timeout,
    Exception { function: u8, code: u8 },
    Protocol(String),
    Usage(String),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Error::Timeout,
            _ => Error::Io(e),
        }
    }
}

pub fn exception_name(code: u8) -> &'static str {
    match code {
        0x01 => "illegal function",
        0x02 => "illegal data address",
        0x03 => "illegal data value",
        0x04 => "server device failure",
        0x05 => "acknowledge",
        0x06 => "server device busy",
        0x08 => "memory parity error",
        0x0A => "gateway path unavailable",
        0x0B => "gateway target device failed to respond",
        _ => "unknown exception",
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Timeout => write!(f, "timeout"),
            Error::Exception { function, code } => write!(
                f,
                "modbus exception 0x{code:02X} ({}) for function 0x{function:02X}",
                exception_name(*code)
            ),
            Error::Protocol(m) => write!(f, "protocol error: {m}"),
            Error::Usage(m) => write!(f, "{m}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A transport sends a request PDU to a unit and returns the response PDU
/// (function code + data, without any framing such as MBAP header or CRC).
pub trait Transport {
    fn transact(&mut self, unit: u8, pdu: &[u8]) -> Result<Vec<u8>>;
}

/// Given the first bytes of a response PDU, determine the full PDU length.
/// Returns `None` when more bytes are needed to decide.
/// Used by stream framings that carry no length field (RTU).
pub fn expected_response_len(pdu: &[u8]) -> std::result::Result<Option<usize>, Error> {
    let Some(&fc) = pdu.first() else { return Ok(None) };
    if fc & 0x80 != 0 {
        return Ok(Some(2));
    }
    match fc {
        FC_READ_COILS | FC_READ_DISCRETE_INPUTS | FC_READ_HOLDING_REGISTERS | FC_READ_INPUT_REGISTERS => {
            Ok(pdu.get(1).map(|&n| 2 + n as usize))
        }
        FC_WRITE_SINGLE_COIL | FC_WRITE_SINGLE_REGISTER | FC_WRITE_MULTIPLE_COILS | FC_WRITE_MULTIPLE_REGISTERS => {
            Ok(Some(5))
        }
        _ => Err(Error::Protocol(format!("unsupported function code 0x{fc:02X} in response"))),
    }
}

fn check_range(addr: u16, count: u16, max: u16) -> Result<()> {
    if count == 0 || count > max {
        return Err(Error::Usage(format!("count must be between 1 and {max}")));
    }
    if addr as u32 + count as u32 > 0x1_0000 {
        return Err(Error::Usage("address range exceeds 65535".into()));
    }
    Ok(())
}

pub struct Client {
    transport: Box<dyn Transport>,
    pub unit: u8,
}

impl Client {
    pub fn new(transport: Box<dyn Transport>, unit: u8) -> Self {
        Client { transport, unit }
    }

    fn request(&mut self, pdu: &[u8]) -> Result<Vec<u8>> {
        let resp = self.transport.transact(self.unit, pdu)?;
        let fc = pdu[0];
        match resp.first() {
            None => Err(Error::Protocol("empty response".into())),
            Some(&r) if r == fc | 0x80 => Err(Error::Exception {
                function: fc,
                code: resp.get(1).copied().unwrap_or(0),
            }),
            Some(&r) if r != fc => Err(Error::Protocol(format!(
                "unexpected function code 0x{r:02X} in response (expected 0x{fc:02X})"
            ))),
            _ => Ok(resp),
        }
    }

    /// Build a read request and return the payload bytes after validating the byte count.
    fn read_raw(&mut self, fc: u8, addr: u16, count: u16, expected_bytes: usize) -> Result<Vec<u8>> {
        let [ah, al] = addr.to_be_bytes();
        let [ch, cl] = count.to_be_bytes();
        let resp = self.request(&[fc, ah, al, ch, cl])?;
        let n = *resp.get(1).ok_or_else(|| Error::Protocol("truncated response".into()))? as usize;
        if resp.len() != 2 + n {
            return Err(Error::Protocol("byte count does not match response length".into()));
        }
        if n != expected_bytes {
            return Err(Error::Protocol(format!("expected {expected_bytes} data bytes, got {n}")));
        }
        Ok(resp[2..].to_vec())
    }

    pub fn read_bits(&mut self, fc: u8, addr: u16, count: u16) -> Result<Vec<bool>> {
        check_range(addr, count, MAX_READ_BITS)?;
        let data = self.read_raw(fc, addr, count, (count as usize).div_ceil(8))?;
        Ok((0..count as usize).map(|i| data[i / 8] & (1 << (i % 8)) != 0).collect())
    }

    pub fn read_registers(&mut self, fc: u8, addr: u16, count: u16) -> Result<Vec<u16>> {
        check_range(addr, count, MAX_READ_REGISTERS)?;
        let data = self.read_raw(fc, addr, count, count as usize * 2)?;
        Ok(data.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect())
    }

    fn check_write_echo(resp: &[u8], addr: u16, count: u16) -> Result<()> {
        if resp.len() != 5 {
            return Err(Error::Protocol("invalid write response length".into()));
        }
        let ra = u16::from_be_bytes([resp[1], resp[2]]);
        let rc = u16::from_be_bytes([resp[3], resp[4]]);
        if ra != addr || rc != count {
            return Err(Error::Protocol(format!(
                "write response mismatch (address {ra}, count {rc}; expected {addr}, {count})"
            )));
        }
        Ok(())
    }

    pub fn write_coils(&mut self, addr: u16, values: &[bool]) -> Result<()> {
        let count = u16::try_from(values.len()).unwrap_or(u16::MAX);
        check_range(addr, count, MAX_WRITE_BITS)?;
        let mut bytes = vec![0u8; values.len().div_ceil(8)];
        for (i, _) in values.iter().enumerate().filter(|(_, v)| **v) {
            bytes[i / 8] |= 1 << (i % 8);
        }
        let mut pdu = vec![FC_WRITE_MULTIPLE_COILS];
        pdu.extend_from_slice(&addr.to_be_bytes());
        pdu.extend_from_slice(&count.to_be_bytes());
        pdu.push(bytes.len() as u8);
        pdu.extend_from_slice(&bytes);
        let resp = self.request(&pdu)?;
        Self::check_write_echo(&resp, addr, count)
    }

    pub fn write_registers(&mut self, addr: u16, values: &[u16]) -> Result<()> {
        let count = u16::try_from(values.len()).unwrap_or(u16::MAX);
        check_range(addr, count, MAX_WRITE_REGISTERS)?;
        let mut pdu = vec![FC_WRITE_MULTIPLE_REGISTERS];
        pdu.extend_from_slice(&addr.to_be_bytes());
        pdu.extend_from_slice(&count.to_be_bytes());
        pdu.push((values.len() * 2) as u8);
        for v in values {
            pdu.extend_from_slice(&v.to_be_bytes());
        }
        let resp = self.request(&pdu)?;
        Self::check_write_echo(&resp, addr, count)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Transport that records requests and replays canned responses.
    pub struct Mock {
        pub requests: Vec<(u8, Vec<u8>)>,
        pub responses: VecDeque<Vec<u8>>,
    }

    impl Transport for Mock {
        fn transact(&mut self, unit: u8, pdu: &[u8]) -> Result<Vec<u8>> {
            self.requests.push((unit, pdu.to_vec()));
            self.responses.pop_front().ok_or(Error::Timeout)
        }
    }

    fn client(responses: Vec<Vec<u8>>) -> Client {
        Client::new(Box::new(Mock { requests: vec![], responses: responses.into() }), 1)
    }

    #[test]
    fn read_registers() {
        let mut c = client(vec![vec![0x03, 0x04, 0x12, 0x34, 0x00, 0x2A]]);
        assert_eq!(c.read_registers(FC_READ_HOLDING_REGISTERS, 10, 2).unwrap(), vec![0x1234, 42]);
    }

    #[test]
    fn read_bits() {
        let mut c = client(vec![vec![0x01, 0x02, 0b1000_0101, 0b0000_0001]]);
        let bits = c.read_bits(FC_READ_COILS, 0, 9).unwrap();
        assert_eq!(bits, vec![true, false, true, false, false, false, false, true, true]);
    }

    #[test]
    fn exception() {
        let mut c = client(vec![vec![0x83, 0x02]]);
        match c.read_registers(FC_READ_HOLDING_REGISTERS, 0, 1) {
            Err(Error::Exception { function: 3, code: 2 }) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn write_coils_encoding() {
        let mut c = client(vec![vec![0x0F, 0x00, 0x13, 0x00, 0x0A]]);
        let v = [true, false, true, true, false, false, true, true, true, false];
        c.write_coils(0x13, &v).unwrap();
    }

    #[test]
    fn write_registers_echo_mismatch() {
        let mut c = client(vec![vec![0x10, 0x00, 0x01, 0x00, 0x01]]);
        assert!(matches!(c.write_registers(1, &[1, 2]), Err(Error::Protocol(_))));
    }

    #[test]
    fn range_checks() {
        let mut c = client(vec![]);
        assert!(matches!(c.read_registers(3, 0, 126), Err(Error::Usage(_))));
        assert!(matches!(c.read_registers(3, 0xFFFF, 2), Err(Error::Usage(_))));
        assert!(matches!(c.read_bits(1, 0, 0), Err(Error::Usage(_))));
    }

    #[test]
    fn expected_len() {
        assert_eq!(expected_response_len(&[]).unwrap(), None);
        assert_eq!(expected_response_len(&[0x03]).unwrap(), None);
        assert_eq!(expected_response_len(&[0x03, 4]).unwrap(), Some(6));
        assert_eq!(expected_response_len(&[0x83]).unwrap(), Some(2));
        assert_eq!(expected_response_len(&[0x10]).unwrap(), Some(5));
    }
}
