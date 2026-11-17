//! REPL command parsing, execution and output formatting.

use crate::cli::parse_uint;
use crate::format::{OutFmt, Response, Values};
use crate::modbus::{self, Client, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Coils,
    Discrete,
    Input,
    Holding,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Read { table: Table, addr: u16, count: u16 },
    WriteCoils { addr: u16, values: Vec<bool> },
    WriteRegisters { addr: u16, values: Vec<u16> },
    /// Switch the output format; `None` shows the current one.
    Fmt(Option<OutFmt>),
    Help,
    Exit,
}

impl Command {
    /// Short command name as typed by the user (`rh`, `wc`, ...).
    pub fn name(&self) -> &'static str {
        match self {
            Command::Read { table: Table::Coils, .. } => "rc",
            Command::Read { table: Table::Discrete, .. } => "rd",
            Command::Read { table: Table::Input, .. } => "ri",
            Command::Read { table: Table::Holding, .. } => "rh",
            Command::WriteCoils { .. } => "wc",
            Command::WriteRegisters { .. } => "wh",
            Command::Fmt(_) => "fmt",
            Command::Help => "help",
            Command::Exit => "exit",
        }
    }

    pub fn addr(&self) -> Option<u16> {
        match *self {
            Command::Read { addr, .. } | Command::WriteCoils { addr, .. } | Command::WriteRegisters { addr, .. } => {
                Some(addr)
            }
            _ => None,
        }
    }

    pub fn count(&self) -> Option<u16> {
        match self {
            Command::Read { count, .. } => Some(*count),
            Command::WriteCoils { values, .. } => Some(values.len() as u16),
            Command::WriteRegisters { values, .. } => Some(values.len() as u16),
            _ => None,
        }
    }
}

fn usage(msg: impl Into<String>) -> Error {
    Error::Usage(msg.into())
}

fn parse_addr(s: Option<&&str>) -> Result<u16> {
    let s = s.ok_or_else(|| usage("missing address"))?;
    parse_uint(s)
        .and_then(|v| u16::try_from(v).ok())
        .ok_or_else(|| usage(format!("invalid address '{s}'")))
}

fn parse_register_value(s: &str) -> Result<u16> {
    if let Some(neg) = s.strip_prefix('-') {
        // Allow signed 16-bit values, stored as two's complement.
        return parse_uint(neg)
            .filter(|&v| v <= 0x8000)
            .map(|v| (v as i32).wrapping_neg() as i16 as u16)
            .ok_or_else(|| usage(format!("invalid register value '{s}'")));
    }
    parse_uint(s)
        .and_then(|v| u16::try_from(v).ok())
        .ok_or_else(|| usage(format!("invalid register value '{s}' (must be 0..65535)")))
}

pub fn parse(line: &str) -> Result<Command> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let Some(&cmd) = parts.first() else { return Err(usage("empty command")) };
    let cmd = cmd.to_ascii_lowercase();

    match cmd.as_str() {
        "help" | "?" => return Ok(Command::Help),
        "exit" | "quit" => return Ok(Command::Exit),
        "fmt" => {
            return match parts.get(1..).unwrap_or(&[]) {
                [] => Ok(Command::Fmt(None)),
                [f] => OutFmt::parse(f)
                    .map(|f| Command::Fmt(Some(f)))
                    .ok_or_else(|| usage(format!("unknown format '{f}', use {}", OutFmt::NAMES))),
                _ => Err(usage("too many arguments, expected: fmt <format>")),
            };
        }
        _ => {}
    }

    let mut chars = cmd.chars();
    let (action, target) = (chars.next(), chars.next());
    if chars.next().is_some() || !matches!(action, Some('r') | Some('w')) {
        return Err(usage("invalid action"));
    }
    let table = match target {
        Some('c') => Table::Coils,
        Some('d') => Table::Discrete,
        Some('i') => Table::Input,
        Some('h') => Table::Holding,
        _ => return Err(usage("unknown target")),
    };

    let addr = parse_addr(parts.get(1))?;
    let args = parts.get(2..).unwrap_or(&[]);

    if action == Some('r') {
        let count = match args {
            [] => 1,
            [n] => parse_uint(n)
                .and_then(|v| u16::try_from(v).ok())
                .ok_or_else(|| usage(format!("invalid count '{n}'")))?,
            _ => return Err(usage("too many arguments, expected: <address> [count]")),
        };
        return Ok(Command::Read { table, addr, count });
    }

    if args.is_empty() {
        return Err(usage("no values given to write"));
    }
    match table {
        Table::Coils => {
            let values = args
                .iter()
                .map(|v| parse_uint(v).map(|n| n != 0).ok_or_else(|| usage(format!("invalid coil value '{v}'"))))
                .collect::<Result<Vec<_>>>()?;
            Ok(Command::WriteCoils { addr, values })
        }
        Table::Holding => {
            let values = args.iter().map(|v| parse_register_value(v)).collect::<Result<Vec<_>>>()?;
            Ok(Command::WriteRegisters { addr, values })
        }
        Table::Discrete | Table::Input => Err(usage("can't write to read only registers")),
    }
}

/// Run a read or write command against the device.
/// Must not be called with `Fmt`, `Help` or `Exit`.
pub fn execute(client: &mut Client, cmd: &Command) -> Result<Response> {
    let command = cmd.name();
    let (addr, values, write) = match *cmd {
        Command::Read { table, addr, count } => {
            let values = match table {
                Table::Coils => Values::Bits(client.read_bits(modbus::FC_READ_COILS, addr, count)?),
                Table::Discrete => Values::Bits(client.read_bits(modbus::FC_READ_DISCRETE_INPUTS, addr, count)?),
                Table::Input => Values::Registers(client.read_registers(modbus::FC_READ_INPUT_REGISTERS, addr, count)?),
                Table::Holding => {
                    Values::Registers(client.read_registers(modbus::FC_READ_HOLDING_REGISTERS, addr, count)?)
                }
            };
            (addr, values, false)
        }
        Command::WriteCoils { addr, ref values } => {
            client.write_coils(addr, values)?;
            (addr, Values::Bits(values.clone()), true)
        }
        Command::WriteRegisters { addr, ref values } => {
            client.write_registers(addr, values)?;
            (addr, Values::Registers(values.clone()), true)
        }
        Command::Fmt(_) | Command::Help | Command::Exit => {
            return Err(usage(format!("'{command}' is not a device command")));
        }
    };
    Ok(Response { command, addr, values, write })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads() {
        assert_eq!(parse("rh 10 5").unwrap(), Command::Read { table: Table::Holding, addr: 10, count: 5 });
        assert_eq!(parse("ri 0x10").unwrap(), Command::Read { table: Table::Input, addr: 16, count: 1 });
        assert!(parse("rx 0 1").is_err());
        assert!(parse("xh 0 1").is_err());
        assert!(parse("rh").is_err());
    }

    #[test]
    fn parse_writes() {
        assert_eq!(parse("wc 3 1 0 5").unwrap(), Command::WriteCoils { addr: 3, values: vec![true, false, true] });
        assert_eq!(
            parse("wh 0 0x1234 -1 65535").unwrap(),
            Command::WriteRegisters { addr: 0, values: vec![0x1234, 0xFFFF, 0xFFFF] }
        );
        assert!(parse("wh 0 65536").is_err());
        assert!(parse("wi 0 1").is_err());
        assert!(parse("wd 0 1").is_err());
        assert!(parse("wh 0").is_err());
    }

    #[test]
    fn parse_fmt() {
        assert_eq!(parse("fmt json").unwrap(), Command::Fmt(Some(OutFmt::Json)));
        assert_eq!(parse("FMT yaml").unwrap(), Command::Fmt(Some(OutFmt::Yml)));
        assert_eq!(parse("fmt").unwrap(), Command::Fmt(None));
        assert!(parse("fmt xml").is_err());
        assert!(parse("fmt json list").is_err());
    }

    #[test]
    fn names() {
        assert_eq!(parse("ri 5 2").unwrap().name(), "ri");
        let w = parse("wc 3 1 0").unwrap();
        assert_eq!((w.name(), w.addr(), w.count()), ("wc", Some(3), Some(2)));
    }
}
