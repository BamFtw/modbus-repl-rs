//! Minimal command line argument parser (no external crates).

use crate::format::{CoilFmt, Endian, NumFmt, OutFmt};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Tcp,
    Udp,
    Rtu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub proto: Proto,
    pub unit: u8,
    pub timeout: Duration,
    pub command: Option<String>,
    // RTU (serial) settings
    pub device: Option<String>,
    pub baud: u32,
    pub parity: Parity,
    pub data_bits: u8,
    pub stop_bits: u8,
    // Output formatting; `fmt` defaults to columns (interactive) or json (headless).
    pub fmt: Option<OutFmt>,
    pub endian: Endian,
    pub number: NumFmt,
    pub coil: CoilFmt,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            host: "localhost".into(),
            port: 502,
            proto: Proto::Tcp,
            unit: 1,
            timeout: Duration::from_millis(1500),
            command: None,
            device: None,
            baud: 9600,
            parity: Parity::None,
            data_bits: 8,
            stop_bits: 1,
            fmt: None,
            endian: Endian::Big,
            number: NumFmt::Dec,
            coil: CoilFmt::Bool,
        }
    }
}

pub enum Action {
    Run(Config),
    Help,
    Version,
}

pub const USAGE: &str = "\
modbus-repl - interactive / headless Modbus client (TCP, UDP, RTU)

USAGE:
    modbus-repl [OPTIONS]

GENERAL OPTIONS:
    -P, --proto <tcp|udp|rtu>   Protocol to use                         [default: tcp]
    -u, --unit <ID>             Modbus unit / slave id (0-255)          [default: 1]
    -t, --timeout <MS>          Response timeout in milliseconds        [default: 1500]
    -c, --command <CMD>         Run command(s) (separated by ;), print the results and exit
        --help                  Print this help
    -V, --version               Print version

OUTPUT OPTIONS:
    -f, --fmt <FORMAT>          json, yml, columns or list   [default: columns, json with --command]
    -e, --endian <big|little>   Byte order of 16-bit registers          [default: big]
    -n, --number-fmt <FMT>      dec, signed, hex, oct or bin            [default: dec]
        --coil-fmt <bool|int>   Coil / discrete input values            [default: bool]

TCP / UDP OPTIONS:
    -h, --host <HOST>           Address of the Modbus server            [default: localhost]
    -p, --port <PORT>           Port of the Modbus server               [default: 502]

RTU (SERIAL) OPTIONS:
    -d, --device <PORT>         Serial port, e.g. COM3 or /dev/ttyUSB0
    -b, --baud <RATE>           Baud rate                               [default: 9600]
        --parity <none|even|odd>                                        [default: none]
        --data-bits <7|8>                                               [default: 8]
        --stop-bits <1|2>                                               [default: 1]

COMMANDS:
    rc A N          read N coils starting at address A
    rd A N          read N discrete inputs starting at address A
    ri A N          read N input registers starting at address A
    rh A N          read N holding registers starting at address A
    wc A C1 .. Cn   write coils (1 or 0) starting at address A
    wh A R1 .. Rn   write 16-bit holding registers starting at address A
    fmt [FORMAT]    switch output format (json, yml, columns, list), or show it
    help            show command help (interactive mode)
    exit | quit     leave interactive mode

    Numbers may be given in decimal, hexadecimal (0x..) or binary (0b..).

EXAMPLES:
    modbus-repl -h 192.168.1.10
    modbus-repl -P udp -h 10.0.0.5 -p 5020 -c \"rh 0 10\"
    modbus-repl -P rtu -d COM3 -b 19200 --parity even -u 17
    modbus-repl -P rtu -d /dev/ttyUSB0 -c \"wh 100 0x1234 42; rh 100 2\"
";

/// Parse an unsigned integer in decimal, `0x` hex or `0b` binary notation.
pub fn parse_uint(s: &str) -> Option<u64> {
    let s = s.trim();
    let (digits, radix) = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (h, 16)
    } else if let Some(b) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        (b, 2)
    } else {
        (s, 10)
    };
    if digits.is_empty() {
        return None;
    }
    u64::from_str_radix(&digits.replace('_', ""), radix).ok()
}

fn num<T: TryFrom<u64>>(opt: &str, v: &str) -> Result<T, String> {
    parse_uint(v)
        .and_then(|n| T::try_from(n).ok())
        .ok_or_else(|| format!("invalid value '{v}' for option '{opt}'"))
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Action, String> {
    let mut cfg = Config::default();
    let mut it = args.into_iter();

    while let Some(arg) = it.next() {
        // Support both `--opt value` and `--opt=value`.
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };

        let mut value = |name: &str| -> Result<String, String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => it.next().ok_or_else(|| format!("option '{name}' requires a value")),
            }
        };

        match name.as_str() {
            "--help" | "-?" => return Ok(Action::Help),
            "--version" | "-V" => return Ok(Action::Version),
            "--host" | "-h" => cfg.host = value(&name)?,
            "--port" | "-p" => {
                let v = value(&name)?;
                cfg.port = num(&name, &v)?;
                if cfg.port == 0 {
                    return Err("invalid '--port' parameter".into());
                }
            }
            "--proto" | "-P" => {
                cfg.proto = match value(&name)?.to_ascii_lowercase().as_str() {
                    "tcp" => Proto::Tcp,
                    "udp" => Proto::Udp,
                    "rtu" | "serial" => Proto::Rtu,
                    other => return Err(format!("unknown protocol '{other}', use tcp, udp or rtu")),
                }
            }
            "--unit" | "-u" | "--slave" => cfg.unit = num(&name, &value(&name)?)?,
            "--timeout" | "-t" => {
                let ms: u64 = num(&name, &value(&name)?)?;
                cfg.timeout = Duration::from_millis(ms.max(1));
            }
            "--command" | "-c" => {
                let cmd = value(&name)?;
                if cmd.trim().is_empty() {
                    return Err("'--command' must not be empty".into());
                }
                cfg.command = Some(cmd);
            }
            "--fmt" | "-f" => {
                let v = value(&name)?;
                cfg.fmt = Some(
                    OutFmt::parse(&v).ok_or_else(|| format!("unknown format '{v}', use {}", OutFmt::NAMES))?,
                );
            }
            "--endian" | "-e" => {
                let v = value(&name)?;
                cfg.endian = Endian::parse(&v).ok_or_else(|| format!("invalid endian '{v}', use big or little"))?;
            }
            "--number-fmt" | "-n" => {
                let v = value(&name)?;
                cfg.number = NumFmt::parse(&v)
                    .ok_or_else(|| format!("invalid number format '{v}', use dec, signed, hex, oct or bin"))?;
            }
            "--coil-fmt" => {
                let v = value(&name)?;
                cfg.coil = CoilFmt::parse(&v).ok_or_else(|| format!("invalid coil format '{v}', use bool or int"))?;
            }
            "--device" | "-d" => cfg.device = Some(value(&name)?),
            "--baud" | "-b" => cfg.baud = num(&name, &value(&name)?)?,
            "--parity" => {
                cfg.parity = match value(&name)?.to_ascii_lowercase().as_str() {
                    "none" | "n" => Parity::None,
                    "even" | "e" => Parity::Even,
                    "odd" | "o" => Parity::Odd,
                    other => return Err(format!("invalid parity '{other}', use none, even or odd")),
                }
            }
            "--data-bits" => {
                cfg.data_bits = num(&name, &value(&name)?)?;
                if !(5..=8).contains(&cfg.data_bits) {
                    return Err("'--data-bits' must be between 5 and 8".into());
                }
            }
            "--stop-bits" => {
                cfg.stop_bits = num(&name, &value(&name)?)?;
                if !(1..=2).contains(&cfg.stop_bits) {
                    return Err("'--stop-bits' must be 1 or 2".into());
                }
            }
            _ => return Err(format!("unknown argument '{arg}' (see --help)")),
        }
    }

    if cfg.proto == Proto::Rtu && cfg.device.is_none() {
        return Err("RTU protocol requires '--device' (e.g. COM3 or /dev/ttyUSB0)".into());
    }
    if cfg.proto != Proto::Rtu && cfg.host.is_empty() {
        return Err("invalid '--host' parameter".into());
    }
    Ok(Action::Run(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Action, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn numbers() {
        assert_eq!(parse_uint("10"), Some(10));
        assert_eq!(parse_uint("0x1F"), Some(31));
        assert_eq!(parse_uint("0b101"), Some(5));
        assert_eq!(parse_uint("0x"), None);
        assert_eq!(parse_uint("abc"), None);
    }

    #[test]
    fn options() {
        let Ok(Action::Run(c)) = p(&["-h", "1.2.3.4", "--port=5020", "-P", "udp", "-c", "rh 0 1"]) else {
            panic!()
        };
        assert_eq!(c.host, "1.2.3.4");
        assert_eq!(c.port, 5020);
        assert_eq!(c.proto, Proto::Udp);
        assert_eq!(c.command.as_deref(), Some("rh 0 1"));
    }

    #[test]
    fn format_options() {
        let Ok(Action::Run(c)) = p(&[]) else { panic!() };
        assert_eq!((c.fmt, c.endian, c.number, c.coil), (None, Endian::Big, NumFmt::Dec, CoilFmt::Bool));
        let Ok(Action::Run(c)) = p(&["-f", "yml", "-e", "little", "-n", "hex", "--coil-fmt=int"]) else { panic!() };
        assert_eq!(
            (c.fmt, c.endian, c.number, c.coil),
            (Some(OutFmt::Yml), Endian::Little, NumFmt::Hex, CoilFmt::Int)
        );
        assert!(p(&["-f", "xml"]).is_err());
        assert!(p(&["-e", "middle"]).is_err());
        assert!(p(&["-n", "dez"]).is_err());
        assert!(p(&["--coil-fmt", "yes"]).is_err());
    }

    #[test]
    fn rtu_requires_device() {
        assert!(p(&["-P", "rtu"]).is_err());
        let Ok(Action::Run(c)) = p(&["-P", "rtu", "-d", "COM3", "--parity", "none", "-b", "19200"]) else {
            panic!()
        };
        assert_eq!(c.parity, Parity::None);
        assert_eq!(c.baud, 19200);
    }

    #[test]
    fn empty_command_rejected() {
        assert!(p(&["-c", " "]).is_err());
        assert!(p(&["-H"]).is_err());
    }
}
