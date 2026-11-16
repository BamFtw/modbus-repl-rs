//! Modbus transports: TCP (MBAP), UDP (MBAP over datagrams) and RTU (serial + CRC16).

use crate::modbus::{self, Error, Result, Transport};
use crate::serial::SerialPort;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

const MAX_ADU: usize = 260;

fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>> {
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs()?.collect();
    if addrs.is_empty() {
        return Err(Error::Usage(format!("could not resolve host '{host}'")));
    }
    Ok(addrs)
}

/// Build a Modbus application header (MBAP) frame.
fn mbap_frame(tid: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(7 + pdu.len());
    f.extend_from_slice(&tid.to_be_bytes());
    f.extend_from_slice(&[0, 0]); // protocol id
    f.extend_from_slice(&((pdu.len() + 1) as u16).to_be_bytes());
    f.push(unit);
    f.extend_from_slice(pdu);
    f
}

/// Validate an MBAP header, returning (transaction id, remaining length incl. unit id).
fn parse_mbap_header(h: &[u8]) -> Result<(u16, usize)> {
    let tid = u16::from_be_bytes([h[0], h[1]]);
    let proto = u16::from_be_bytes([h[2], h[3]]);
    let len = u16::from_be_bytes([h[4], h[5]]) as usize;
    if proto != 0 {
        return Err(Error::Protocol(format!("invalid protocol id {proto}")));
    }
    if !(2..=254).contains(&len) {
        return Err(Error::Protocol(format!("invalid MBAP length {len}")));
    }
    Ok((tid, len))
}

// ---------------------------------------------------------------- TCP

pub struct TcpTransport {
    addrs: Vec<SocketAddr>,
    stream: Option<TcpStream>,
    timeout: Duration,
    tid: u16,
}

impl TcpTransport {
    pub fn connect(host: &str, port: u16, timeout: Duration) -> Result<Self> {
        let mut t = TcpTransport { addrs: resolve(host, port)?, stream: None, timeout, tid: 0 };
        t.ensure_connected()?;
        Ok(t)
    }

    fn ensure_connected(&mut self) -> Result<&mut TcpStream> {
        if self.stream.is_none() {
            let mut last_err = None;
            for addr in &self.addrs {
                match TcpStream::connect_timeout(addr, self.timeout) {
                    Ok(s) => {
                        s.set_nodelay(true)?;
                        s.set_read_timeout(Some(self.timeout))?;
                        s.set_write_timeout(Some(self.timeout))?;
                        self.stream = Some(s);
                        break;
                    }
                    Err(e) => last_err = Some((addr, e)),
                }
            }
            if self.stream.is_none() {
                let (addr, e) = last_err.expect("at least one address");
                return Err(Error::Io(std::io::Error::new(e.kind(), format!("cannot connect to {addr}: {e}"))));
            }
        }
        Ok(self.stream.as_mut().unwrap())
    }

    fn exchange(&mut self, unit: u8, pdu: &[u8]) -> Result<Vec<u8>> {
        self.tid = self.tid.wrapping_add(1);
        let tid = self.tid;
        let frame = mbap_frame(tid, unit, pdu);
        let s = self.ensure_connected()?;
        s.write_all(&frame)?;
        loop {
            let mut header = [0u8; 7];
            s.read_exact(&mut header)?;
            let (rtid, len) = parse_mbap_header(&header)?;
            let mut body = vec![0u8; len - 1];
            s.read_exact(&mut body)?;
            if rtid == tid {
                return Ok(body);
            }
            // Stale response from an earlier, timed out request: discard it.
        }
    }
}

impl Transport for TcpTransport {
    fn transact(&mut self, unit: u8, pdu: &[u8]) -> Result<Vec<u8>> {
        let r = self.exchange(unit, pdu);
        if matches!(r, Err(Error::Io(_)) | Err(Error::Timeout) | Err(Error::Protocol(_))) {
            // Stream state is unknown: drop it, reconnect on the next request.
            self.stream = None;
        }
        r
    }
}

// ---------------------------------------------------------------- UDP

pub struct UdpTransport {
    socket: UdpSocket,
    timeout: Duration,
    tid: u16,
}

impl UdpTransport {
    pub fn open(host: &str, port: u16, timeout: Duration) -> Result<Self> {
        let addrs = resolve(host, port)?;
        let addr = addrs.iter().find(|a| a.is_ipv4()).unwrap_or(&addrs[0]);
        let bind = if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
        let socket = UdpSocket::bind(bind)?;
        socket.connect(addr)?;
        Ok(UdpTransport { socket, timeout, tid: 0 })
    }
}

impl Transport for UdpTransport {
    fn transact(&mut self, unit: u8, pdu: &[u8]) -> Result<Vec<u8>> {
        self.tid = self.tid.wrapping_add(1);
        let tid = self.tid;
        self.socket.send(&mbap_frame(tid, unit, pdu))?;
        let deadline = Instant::now() + self.timeout;
        let mut buf = [0u8; MAX_ADU + 16];
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            self.socket.set_read_timeout(Some(remaining))?;
            let n = match self.socket.recv(&mut buf) {
                Ok(n) => n,
                // Windows reports ICMP "port unreachable" from a previous send as a recv error.
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                    return Err(Error::Protocol("connection refused (no server on that port?)".into()))
                }
                Err(e) => return Err(e.into()),
            };
            if n < 8 {
                continue;
            }
            let (rtid, len) = parse_mbap_header(&buf[..7])?;
            if rtid != tid {
                continue; // late answer to an earlier request
            }
            if 6 + len > n {
                return Err(Error::Protocol("truncated datagram".into()));
            }
            return Ok(buf[7..6 + len].to_vec());
        }
    }
}

// ---------------------------------------------------------------- RTU

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

pub struct RtuTransport {
    port: SerialPort,
    timeout: Duration,
    /// Silent interval (3.5 character times, min 1.75 ms) required between frames.
    frame_gap: Duration,
}

impl RtuTransport {
    pub fn new(port: SerialPort, baud: u32, bits_per_char: u32, timeout: Duration) -> Self {
        let char_us = 1_000_000u64 * bits_per_char as u64 / baud.max(1) as u64;
        let gap = if baud > 19200 { 1750 } else { char_us * 35 / 10 };
        RtuTransport { port, timeout, frame_gap: Duration::from_micros(gap) }
    }
}

impl Transport for RtuTransport {
    fn transact(&mut self, unit: u8, pdu: &[u8]) -> Result<Vec<u8>> {
        let mut frame = Vec::with_capacity(pdu.len() + 3);
        frame.push(unit);
        frame.extend_from_slice(pdu);
        frame.extend_from_slice(&crc16(&frame).to_le_bytes());

        std::thread::sleep(self.frame_gap);
        self.port.clear_input()?;
        self.port.write_all(&frame)?;
        self.port.flush()?;

        if unit == 0 {
            // Broadcast: slaves never answer. Only meaningful for writes.
            return match pdu[0] {
                modbus::FC_WRITE_SINGLE_COIL
                | modbus::FC_WRITE_SINGLE_REGISTER
                | modbus::FC_WRITE_MULTIPLE_COILS
                | modbus::FC_WRITE_MULTIPLE_REGISTERS => Ok(pdu[..5].to_vec()),
                _ => Err(Error::Usage("unit 0 (broadcast) can only be used for writes".into())),
            };
        }

        let deadline = Instant::now() + self.timeout;
        let mut buf: Vec<u8> = Vec::with_capacity(MAX_ADU);
        let mut chunk = [0u8; MAX_ADU];
        loop {
            // Frame = unit + PDU + 2 byte CRC
            if buf.len() > 1 {
                if let Some(pdu_len) = modbus::expected_response_len(&buf[1..])? {
                    let total = 1 + pdu_len + 2;
                    if buf.len() >= total {
                        let (body, crc) = buf[..total].split_at(total - 2);
                        if crc16(body).to_le_bytes() != [crc[0], crc[1]] {
                            return Err(Error::Protocol("CRC mismatch".into()));
                        }
                        if body[0] != unit {
                            return Err(Error::Protocol(format!(
                                "response from unit {} (expected {unit})",
                                body[0]
                            )));
                        }
                        return Ok(body[1..].to_vec());
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(if buf.is_empty() {
                    Error::Timeout
                } else {
                    Error::Protocol(format!("incomplete frame received ({} bytes)", buf.len()))
                });
            }
            let n = self.port.read(&mut chunk)?; // returns 0 after a short poll interval
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() > MAX_ADU {
                return Err(Error::Protocol("response too long".into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn crc() {
        // Read holding registers, unit 1, addr 0, count 10 -> CRC C5 CD
        assert_eq!(crc16(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A]).to_le_bytes(), [0xC5, 0xCD]);
    }

    #[test]
    fn mbap() {
        assert_eq!(
            mbap_frame(0x0102, 7, &[3, 0, 0, 0, 1]),
            vec![1, 2, 0, 0, 0, 6, 7, 3, 0, 0, 0, 1]
        );
    }

    /// Spin up a tiny Modbus TCP server answering one "read holding registers" request.
    #[test]
    fn tcp_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = [0u8; 12];
            s.read_exact(&mut req).unwrap();
            assert_eq!(&req[6..], &[5, 3, 0, 10, 0, 2]);
            let mut resp = req[..4].to_vec();
            resp.extend_from_slice(&[0, 7, 5, 3, 4, 0xAB, 0xCD, 0, 1]);
            s.write_all(&resp).unwrap();
        });
        let t = TcpTransport::connect("127.0.0.1", port, Duration::from_secs(2)).unwrap();
        let mut c = modbus::Client::new(Box::new(t), 5);
        assert_eq!(c.read_registers(3, 10, 2).unwrap(), vec![0xABCD, 1]);
        srv.join().unwrap();
    }

    #[test]
    fn udp_roundtrip() {
        let srv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = srv_sock.local_addr().unwrap().port();
        let srv = std::thread::spawn(move || {
            let mut buf = [0u8; 300];
            let (n, from) = srv_sock.recv_from(&mut buf).unwrap();
            assert_eq!(n, 12);
            let mut resp = buf[..4].to_vec();
            resp.extend_from_slice(&[0, 4, 1, 1, 1, 0b101]);
            srv_sock.send_to(&resp, from).unwrap();
        });
        let t = UdpTransport::open("127.0.0.1", port, Duration::from_secs(2)).unwrap();
        let mut c = modbus::Client::new(Box::new(t), 1);
        assert_eq!(c.read_bits(1, 0, 3).unwrap(), vec![true, false, true]);
        srv.join().unwrap();
    }

    #[test]
    fn udp_timeout() {
        let srv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = srv_sock.local_addr().unwrap().port();
        let t = UdpTransport::open("127.0.0.1", port, Duration::from_millis(200)).unwrap();
        let mut c = modbus::Client::new(Box::new(t), 1);
        assert!(matches!(c.read_bits(1, 0, 3), Err(Error::Timeout)));
        drop(srv_sock);
    }
}
