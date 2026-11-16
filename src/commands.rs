//! REPL command parsing, execution and output formatting.

use crate::cli::parse_uint;
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
    Help,
    Exit,
}

pub enum Output {
    None,
    Bits { addr: u16, values: Vec<bool> },
    Registers { addr: u16, values: Vec<u16> },
}

fn usage(msg: impl Into<String>) -> Error {
    Error::Usage(format!("ERROR: {}", msg.into()))
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

pub fn execute(client: &mut Client, cmd: &Command) -> Result<Output> {
    Ok(match *cmd {
        Command::Read { table, addr, count } => match table {
            Table::Coils => Output::Bits { addr, values: client.read_bits(modbus::FC_READ_COILS, addr, count)? },
            Table::Discrete => {
                Output::Bits { addr, values: client.read_bits(modbus::FC_READ_DISCRETE_INPUTS, addr, count)? }
            }
            Table::Input => Output::Registers {
                addr,
                values: client.read_registers(modbus::FC_READ_INPUT_REGISTERS, addr, count)?,
            },
            Table::Holding => Output::Registers {
                addr,
                values: client.read_registers(modbus::FC_READ_HOLDING_REGISTERS, addr, count)?,
            },
        },
        Command::WriteCoils { addr, ref values } => {
            client.write_coils(addr, values)?;
            Output::None
        }
        Command::WriteRegisters { addr, ref values } => {
            client.write_registers(addr, values)?;
            Output::None
        }
        Command::Help | Command::Exit => Output::None,
    })
}

fn printable(b: u8) -> char {
    if (33..=126).contains(&b) {
        b as char
    } else {
        '.'
    }
}

impl Output {
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        match self {
            Output::None => {}
            Output::Bits { addr, values } => {
                for (i, v) in values.iter().enumerate() {
                    let a = *addr as usize + i;
                    out += &format!("  {a:05} : {:>5}\n", *v as u8);
                }
            }
            Output::Registers { addr, values } => {
                for (i, v) in values.iter().enumerate() {
                    let a = *addr as usize + i;
                    let [hi, lo] = v.to_be_bytes();
                    out += &format!("  {a:05} : {v:>5} | 0x{v:04X} | {}{}\n", printable(hi), printable(lo));
                }
            }
        }
        out
    }

    pub fn json(&self) -> Option<String> {
        match self {
            Output::None => None,
            Output::Bits { values, .. } => {
                Some(format!("[{}]", values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")))
            }
            Output::Registers { values, .. } => {
                Some(format!("[{}]", values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")))
            }
        }
    }
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
    fn formatting() {
        let o = Output::Registers { addr: 7, values: vec![0x4142, 5] };
        assert_eq!(o.pretty(), "  00007 : 16706 | 0x4142 | AB\n  00008 :     5 | 0x0005 | ..\n");
        assert_eq!(o.json().unwrap(), "[16706,5]");
        let b = Output::Bits { addr: 0, values: vec![true, false] };
        assert_eq!(b.pretty(), "  00000 :     1\n  00001 :     0\n");
        assert_eq!(b.json().unwrap(), "[true,false]");
    }
}
