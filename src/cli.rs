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
    /// Script file to run (`-` = stdin).
    pub script: Option<String>,
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
            script: None,
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
    -s, --script <PATH>         Run the commands of a script file (- for stdin) and exit
        --config <NAME>         Load defaults from a stored configuration (see CONFIGURATIONS)
        --help                  Print this help
    -V, --version               Print version

OUTPUT OPTIONS:
    -f, --fmt <FORMAT>          json, yml, columns or list
                                [default: columns, json with --command / --script]
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
    wait MS         pause for MS milliseconds
    help            show command help (interactive mode)
    exit | quit     leave interactive mode

    Numbers may be given in decimal, hexadecimal (0x..) or binary (0b..).

SCRIPTS:
    A script file holds the same commands, one or more per line separated
    by ';'. Text after '#' is a comment. All commands are checked before
    the first one runs; on a syntax error nothing is executed.

CONFIGURATIONS:
    Stored configurations hold default values for the options above. The
    configuration 'default' is used when '--config' is not given. Options
    given on the command line always override the stored values.

    modbus-repl config start                  edit the default configuration
    modbus-repl config list                   list stored configurations
    modbus-repl config add NAME               create a configuration (dialog)
    modbus-repl config edit NAME              change a configuration (dialog)
    modbus-repl config remove NAME            delete a configuration
    modbus-repl config export NAME PATH       save a configuration to a file
    modbus-repl config import PATH NAME       store a file as a configuration

EXAMPLES:
    modbus-repl -h 192.168.1.10
    modbus-repl -P udp -h 10.0.0.5 -p 5020 -c \"rh 0 10\"
    modbus-repl -P rtu -d COM3 -b 19200 --parity even -u 17
    modbus-repl -P rtu -d /dev/ttyUSB0 -c \"wh 100 0x1234 42; rh 100 2\"
    modbus-repl --config ts2ra -u 10
    modbus-repl -h 10.0.0.5 -s setup.mbs
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

fn num<T: TryFrom<u64>>(key: &str, v: &str) -> Result<T, String> {
    parse_uint(v)
        .and_then(|n| T::try_from(n).ok())
        .ok_or_else(|| format!("invalid value '{v}' for '{key}'"))
}

/// Map a command line option (`-h`, `--host`, ...) to its configuration key.
fn config_key(arg: &str) -> Option<&'static str> {
    Some(match arg {
        "--host" | "-h" => "host",
        "--port" | "-p" => "port",
        "--proto" | "-P" => "proto",
        "--unit" | "-u" | "--slave" => "unit",
        "--timeout" | "-t" => "timeout",
        "--device" | "-d" => "device",
        "--baud" | "-b" => "baud",
        "--parity" => "parity",
        "--data-bits" => "data-bits",
        "--stop-bits" => "stop-bits",
        "--fmt" | "-f" => "fmt",
        "--endian" | "-e" => "endian",
        "--number-fmt" | "-n" => "number-fmt",
        "--coil-fmt" => "coil-fmt",
        _ => return None,
    })
}

/// Set one option (identified by its configuration key) on `cfg`.
pub fn apply(cfg: &mut Config, key: &str, v: &str) -> Result<(), String> {
    match key {
        "host" => {
            if v.is_empty() {
                return Err("'host' must not be empty".into());
            }
            cfg.host = v.to_string();
        }
        "port" => {
            cfg.port = num(key, v)?;
            if cfg.port == 0 {
                return Err("'port' must not be 0".into());
            }
        }
        "proto" => {
            cfg.proto = match v.to_ascii_lowercase().as_str() {
                "tcp" => Proto::Tcp,
                "udp" => Proto::Udp,
                "rtu" | "serial" => Proto::Rtu,
                other => return Err(format!("unknown protocol '{other}', use tcp, udp or rtu")),
            }
        }
        "unit" => cfg.unit = num(key, v)?,
        "timeout" => {
            let ms: u64 = num(key, v)?;
            cfg.timeout = Duration::from_millis(ms.max(1));
        }
        "device" => {
            if v.is_empty() {
                return Err("'device' must not be empty".into());
            }
            cfg.device = Some(v.to_string());
        }
        "baud" => {
            cfg.baud = num(key, v)?;
            if cfg.baud == 0 {
                return Err("'baud' must not be 0".into());
            }
        }
        "parity" => {
            cfg.parity = match v.to_ascii_lowercase().as_str() {
                "none" | "n" => Parity::None,
                "even" | "e" => Parity::Even,
                "odd" | "o" => Parity::Odd,
                other => return Err(format!("invalid parity '{other}', use none, even or odd")),
            }
        }
        "data-bits" => {
            cfg.data_bits = num(key, v)?;
            if !(5..=8).contains(&cfg.data_bits) {
                return Err("'data-bits' must be between 5 and 8".into());
            }
        }
        "stop-bits" => {
            cfg.stop_bits = num(key, v)?;
            if !(1..=2).contains(&cfg.stop_bits) {
                return Err("'stop-bits' must be 1 or 2".into());
            }
        }
        "fmt" => {
            cfg.fmt = Some(OutFmt::parse(v).ok_or_else(|| format!("unknown format '{v}', use {}", OutFmt::NAMES))?)
        }
        "endian" => cfg.endian = Endian::parse(v).ok_or_else(|| format!("invalid endian '{v}', use big or little"))?,
        "number-fmt" => {
            cfg.number = NumFmt::parse(v)
                .ok_or_else(|| format!("invalid number format '{v}', use dec, signed, hex, oct or bin"))?
        }
        "coil-fmt" => cfg.coil = CoilFmt::parse(v).ok_or_else(|| format!("invalid coil format '{v}', use bool or int"))?,
        _ => return Err(format!("unknown option '{key}'")),
    }
    Ok(())
}

/// Split `--opt=value` into its parts; other arguments are returned as is.
fn split_arg(arg: &str) -> (&str, Option<&str>) {
    match arg.split_once('=') {
        Some((n, v)) if n.starts_with("--") => (n, Some(v)),
        _ => (arg, None),
    }
}

/// Parse the command line.
///
/// `load` returns the stored settings (key, value) of the configuration named
/// by `--config`, or of the default configuration when `None` is passed (an
/// empty list if there is none). Settings given on the command line override
/// the stored ones.
pub fn parse<I, L>(args: I, load: L) -> Result<Action, String>
where
    I: IntoIterator<Item = String>,
    L: FnOnce(Option<&str>) -> Result<Vec<(String, String)>, String>,
{
    let args: Vec<String> = args.into_iter().collect();

    // First pass: find --config, so stored values can be applied before the
    // command line ones.
    let mut config_name = None;
    let mut i = 0;
    while i < args.len() {
        let (name, inline) = split_arg(&args[i]);
        if name == "--config" {
            let v = match inline {
                Some(v) => v,
                None => {
                    i += 1;
                    args.get(i).map(String::as_str).ok_or("option '--config' requires a value")?
                }
            };
            config_name = Some(v.to_string());
        } else if (config_key(name).is_some() || matches!(name, "--command" | "-c" | "--script" | "-s"))
            && inline.is_none()
        {
            i += 1; // skip the value
        }
        i += 1;
    }

    let mut cfg = Config::default();
    for (key, value) in load(config_name.as_deref())? {
        apply(&mut cfg, &key, &value).map_err(|e| match &config_name {
            Some(n) => format!("configuration '{n}': {e}"),
            None => format!("default configuration: {e}"),
        })?;
    }

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (name, inline) = split_arg(arg);
        let mut value = || -> Result<&str, String> {
            match inline {
                Some(v) => Ok(v),
                None => it.next().map(String::as_str).ok_or_else(|| format!("option '{name}' requires a value")),
            }
        };

        match name {
            "--help" | "-?" => return Ok(Action::Help),
            "--version" | "-V" => return Ok(Action::Version),
            "--config" => {
                value()?;
            }
            "--command" | "-c" => {
                let cmd = value()?;
                if cmd.trim().is_empty() {
                    return Err("'--command' must not be empty".into());
                }
                cfg.command = Some(cmd.to_string());
            }
            "--script" | "-s" => {
                let path = value()?;
                if path.is_empty() {
                    return Err("'--script' requires a file name".into());
                }
                cfg.script = Some(path.to_string());
            }
            _ => match config_key(name) {
                Some(key) => {
                    let v = value()?;
                    apply(&mut cfg, key, v).map_err(|e| format!("{name}: {e}"))?;
                }
                None => return Err(format!("unknown argument '{arg}' (see --help)")),
            },
        }
    }

    if cfg.command.is_some() && cfg.script.is_some() {
        return Err("'--command' and '--script' can't be used together".into());
    }
    if cfg.proto == Proto::Rtu && cfg.device.is_none() {
        return Err("RTU protocol requires '--device' (e.g. COM3 or /dev/ttyUSB0)".into());
    }
    Ok(Action::Run(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Action, String> {
        parse(args.iter().map(|s| s.to_string()), |_| Ok(vec![]))
    }

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn stored_config() {
        // --config selects the stored settings, command line values override them.
        let args = ["--config", "ts2ra", "-u", "10"].map(String::from);
        let Ok(Action::Run(c)) = parse(args, |name| {
            assert_eq!(name, Some("ts2ra"));
            Ok(kv(&[("proto", "rtu"), ("device", "COM3"), ("unit", "5"), ("fmt", "list")]))
        }) else {
            panic!()
        };
        assert_eq!((c.proto, c.device.as_deref(), c.unit, c.fmt), (Proto::Rtu, Some("COM3"), 10, Some(OutFmt::List)));

        // Without --config the default configuration is requested.
        let Ok(Action::Run(c)) = parse(["--port=5020".to_string()], |name| {
            assert_eq!(name, None);
            Ok(kv(&[("host", "10.0.0.1"), ("port", "1")]))
        }) else {
            panic!()
        };
        assert_eq!((c.host.as_str(), c.port), ("10.0.0.1", 5020));

        // --config=name form, and invalid stored values are reported.
        let r = parse(["--config=x".to_string()], |name| {
            assert_eq!(name, Some("x"));
            Ok(kv(&[("baud", "fast")]))
        });
        assert!(r.err().unwrap().contains("configuration 'x'"));

        // A command text that looks like an option is not mistaken for --config.
        let r = parse(["-c", "--config", "-u", "2"].map(String::from), |name| {
            assert_eq!(name, None);
            Ok(vec![])
        });
        assert!(r.is_ok());
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
    fn script_option() {
        let Ok(Action::Run(c)) = p(&["-s", "setup.mbs", "-u", "3"]) else { panic!() };
        assert_eq!((c.script.as_deref(), c.unit), (Some("setup.mbs"), 3));
        let Ok(Action::Run(c)) = p(&["--script=-"]) else { panic!() };
        assert_eq!(c.script.as_deref(), Some("-"));
        assert!(p(&["-s", "a", "-c", "rh 0"]).is_err());
        assert!(p(&["-s"]).is_err());
    }

    #[test]
    fn empty_command_rejected() {
        assert!(p(&["-c", " "]).is_err());
        assert!(p(&["-H"]).is_err());
    }
}
