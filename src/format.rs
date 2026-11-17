//! Output formatting: json, yml, columns and list renderers plus the value
//! display options (byte order, number base, coil representation).

use crate::modbus::Error;

/// Overall output layout (`--fmt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutFmt {
    Json,
    Yml,
    Columns,
    List,
}

/// Byte order used to interpret 16-bit registers (`--endian`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Big,
    Little,
}

/// Number base used to display register values (`--number-fmt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumFmt {
    /// Unsigned decimal (0..65535).
    Dec,
    /// Signed decimal, two's complement (-32768..32767).
    Signed,
    Hex,
    Oct,
    Bin,
}

/// Representation of coils / discrete inputs (`--coil-fmt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoilFmt {
    Bool,
    Int,
}

impl OutFmt {
    pub const NAMES: &'static str = "json, yml, columns or list";

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "json" => OutFmt::Json,
            "yml" | "yaml" => OutFmt::Yml,
            "columns" | "column" | "cols" | "col" => OutFmt::Columns,
            "list" => OutFmt::List,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            OutFmt::Json => "json",
            OutFmt::Yml => "yml",
            OutFmt::Columns => "columns",
            OutFmt::List => "list",
        }
    }

    /// Structured formats report errors on stdout as part of the document.
    pub fn is_structured(self) -> bool {
        matches!(self, OutFmt::Json | OutFmt::Yml)
    }
}

impl Endian {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "big" | "be" | "b" => Some(Endian::Big),
            "little" | "le" | "l" => Some(Endian::Little),
            _ => None,
        }
    }

    /// Convert a register as received on the wire (big endian) to the value
    /// shown to the user.
    pub fn apply(self, v: u16) -> u16 {
        match self {
            Endian::Big => v,
            Endian::Little => v.swap_bytes(),
        }
    }
}

impl NumFmt {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "dec" | "d" | "u16" | "unsigned" => NumFmt::Dec,
            "signed" | "sdec" | "s" | "i16" => NumFmt::Signed,
            "hex" | "x" => NumFmt::Hex,
            "oct" | "o" => NumFmt::Oct,
            "bin" | "b" => NumFmt::Bin,
            _ => return None,
        })
    }

    pub fn format(self, v: u16) -> String {
        match self {
            NumFmt::Dec => v.to_string(),
            NumFmt::Signed => (v as i16).to_string(),
            NumFmt::Hex => format!("0x{v:04X}"),
            NumFmt::Oct => format!("0o{v:06o}"),
            NumFmt::Bin => format!("0b{v:016b}"),
        }
    }

    /// Whether the formatted value is a plain number in JSON / YAML
    /// (otherwise it is emitted as a string to keep its notation).
    fn is_numeric(self) -> bool {
        matches!(self, NumFmt::Dec | NumFmt::Signed)
    }
}

impl CoilFmt {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bool" | "boolean" => Some(CoilFmt::Bool),
            "int" | "num" | "number" => Some(CoilFmt::Int),
            _ => None,
        }
    }

    fn format(self, v: bool) -> &'static str {
        match (self, v) {
            (CoilFmt::Bool, true) => "true",
            (CoilFmt::Bool, false) => "false",
            (CoilFmt::Int, true) => "1",
            (CoilFmt::Int, false) => "0",
        }
    }
}

/// All display settings together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub out: OutFmt,
    pub endian: Endian,
    pub number: NumFmt,
    pub coil: CoilFmt,
}

impl Default for Format {
    fn default() -> Self {
        Format { out: OutFmt::Columns, endian: Endian::Big, number: NumFmt::Dec, coil: CoilFmt::Bool }
    }
}

/// Values read from or written to the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Values {
    Bits(Vec<bool>),
    Registers(Vec<u16>),
}

impl Values {
    pub fn len(&self) -> usize {
        match self {
            Values::Bits(v) => v.len(),
            Values::Registers(v) => v.len(),
        }
    }
}

/// Result of a successful read or write command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub command: &'static str,
    pub addr: u16,
    pub values: Values,
    /// Writes only produce output in the structured formats.
    pub write: bool,
}

/// What is known about a failed command (as much as could be parsed).
pub struct Failure<'a> {
    pub command: &'a str,
    pub addr: Option<u16>,
    pub count: Option<u16>,
    pub error: &'a Error,
}

/// Numeric code reported as `error_code` in json / yml output:
/// 0 = success, 1..255 = Modbus exception code, negative = client side error.
pub fn error_code(e: &Error) -> i32 {
    match e {
        Error::Exception { code, .. } => *code as i32,
        Error::Io(_) => -1,
        Error::Timeout => -2,
        Error::Protocol(_) => -3,
        Error::Usage(_) => -4,
    }
}

fn printable(b: u8) -> char {
    if (33..=126).contains(&b) {
        b as char
    } else {
        '.'
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// YAML scalar for a string: plain when unambiguous, double quoted otherwise.
fn yml_str(s: &str) -> String {
    let plain = s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
        );
    if plain {
        s.to_string()
    } else {
        json_str(s)
    }
}

/// A field value of a structured document.
enum Field {
    Int(i64),
    Str(String),
    /// Already rendered scalars, emitted as a flow sequence.
    List(Vec<String>),
}

impl Format {
    /// Values as scalars suitable for JSON / YAML (strings are quoted).
    fn scalars(&self, values: &Values) -> Vec<String> {
        match values {
            Values::Bits(v) => v.iter().map(|&b| self.coil.format(b).to_string()).collect(),
            Values::Registers(v) => v
                .iter()
                .map(|&r| {
                    let s = self.number.format(self.endian.apply(r));
                    if self.number.is_numeric() {
                        s
                    } else {
                        json_str(&s)
                    }
                })
                .collect(),
        }
    }

    fn document(&self, fields: &[(&str, Field)]) -> String {
        match self.out {
            OutFmt::Json => {
                let body: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| {
                        let v = match v {
                            Field::Int(n) => n.to_string(),
                            Field::Str(s) => json_str(s),
                            Field::List(l) => format!("[{}]", l.join(", ")),
                        };
                        format!("\t\"{k}\": {v}")
                    })
                    .collect();
                format!("{{\n{}\n}}\n", body.join(",\n"))
            }
            _ => fields
                .iter()
                .map(|(k, v)| {
                    let v = match v {
                        Field::Int(n) => n.to_string(),
                        Field::Str(s) => yml_str(s),
                        Field::List(l) => format!("[{}]", l.join(", ")),
                    };
                    format!("{k}: {v}\n")
                })
                .collect(),
        }
    }

    /// Render a successful command. Returns an empty string when the format
    /// has nothing to show (writes in columns / list mode).
    pub fn response(&self, r: &Response) -> String {
        match self.out {
            OutFmt::Json | OutFmt::Yml => self.document(&[
                ("error_code", Field::Int(0)),
                ("command", Field::Str(r.command.to_string())),
                ("address", Field::Int(r.addr as i64)),
                ("count", Field::Int(r.values.len() as i64)),
                ("values", Field::List(self.scalars(&r.values))),
            ]),
            _ if r.write => String::new(),
            OutFmt::List => {
                let items: Vec<String> = match &r.values {
                    Values::Bits(v) => v.iter().map(|&b| self.coil.format(b).to_string()).collect(),
                    Values::Registers(v) => v.iter().map(|&x| self.number.format(self.endian.apply(x))).collect(),
                };
                format!("{}\n", items.join(", "))
            }
            OutFmt::Columns => self.columns(r),
        }
    }

    fn columns(&self, r: &Response) -> String {
        let addr = |i: usize| r.addr as usize + i;
        let mut out = String::new();
        match &r.values {
            Values::Bits(v) => {
                out += &format!(" {:>5} | value\n", "addr");
                for (i, &b) in v.iter().enumerate() {
                    out += &format!(" {:>5} | {}\n", addr(i), self.coil.format(b));
                }
            }
            Values::Registers(v) => {
                let rows: Vec<(String, String, String, String)> = v
                    .iter()
                    .map(|&x| {
                        let shown = self.endian.apply(x);
                        let [hi, lo] = shown.to_be_bytes();
                        (
                            format!("0x{shown:04X}"),
                            self.number.format(x),
                            self.number.format(x.swap_bytes()),
                            format!("{}{}", printable(hi), printable(lo)),
                        )
                    })
                    .collect();
                let w = rows.iter().map(|r| r.1.len().max(r.2.len())).max().unwrap_or(0).max(5);
                out += &format!(" {:>5} | {:<6} | {:>w$} | {:>w$} | char\n", "addr", "hex", "BE", "LE");
                for (i, (hex, be, le, ch)) in rows.iter().enumerate() {
                    out += &format!(" {:>5} | {hex} | {be:>w$} | {le:>w$} | {ch}\n", addr(i));
                }
            }
        }
        out
    }

    /// Render a failed command. Structured formats return a document for
    /// stdout; the others return `None` (the caller prints to stderr).
    pub fn failure(&self, f: &Failure) -> Option<String> {
        if !self.out.is_structured() {
            return None;
        }
        let mut fields = vec![
            ("error_code", Field::Int(error_code(f.error) as i64)),
            ("command", Field::Str(f.command.to_string())),
        ];
        if let Some(a) = f.addr {
            fields.push(("address", Field::Int(a as i64)));
        }
        if let Some(c) = f.count {
            fields.push(("count", Field::Int(c as i64)));
        }
        fields.push(("error", Field::Str(f.error.to_string())));
        Some(self.document(&fields))
    }

    /// Separator printed between consecutive documents of a multi-command run.
    pub fn separator(&self) -> &'static str {
        match self.out {
            OutFmt::Yml => "---\n",
            _ => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(out: OutFmt) -> Format {
        Format { out, ..Format::default() }
    }

    fn ri() -> Response {
        Response { command: "ri", addr: 30000, values: Values::Registers(vec![65535, 65535, 0]), write: false }
    }

    #[test]
    fn json() {
        assert_eq!(
            fmt(OutFmt::Json).response(&ri()),
            "{\n\t\"error_code\": 0,\n\t\"command\": \"ri\",\n\t\"address\": 30000,\n\t\"count\": 3,\n\t\"values\": [65535, 65535, 0]\n}\n"
        );
        let f = Format { number: NumFmt::Hex, ..fmt(OutFmt::Json) };
        assert!(f.response(&ri()).contains("\"values\": [\"0xFFFF\", \"0xFFFF\", \"0x0000\"]"));
    }

    #[test]
    fn yml() {
        assert_eq!(
            fmt(OutFmt::Yml).response(&ri()),
            "error_code: 0\ncommand: ri\naddress: 30000\ncount: 3\nvalues: [65535, 65535, 0]\n"
        );
    }

    #[test]
    fn list() {
        assert_eq!(fmt(OutFmt::List).response(&ri()), "65535, 65535, 0\n");
        let r = Response { command: "rh", addr: 0, values: Values::Registers(vec![1, 0xFFFF]), write: false };
        let f = Format { endian: Endian::Little, number: NumFmt::Signed, ..fmt(OutFmt::List) };
        assert_eq!(f.response(&r), "256, -1\n");
        let b = Response { command: "rc", addr: 0, values: Values::Bits(vec![true, false]), write: false };
        assert_eq!(fmt(OutFmt::List).response(&b), "true, false\n");
        assert_eq!(Format { coil: CoilFmt::Int, ..fmt(OutFmt::List) }.response(&b), "1, 0\n");
    }

    #[test]
    fn columns() {
        let r = Response { command: "ri", addr: 30020, values: Values::Registers(vec![0x0001, 0x0A26]), write: false };
        assert_eq!(
            fmt(OutFmt::Columns).response(&r),
            "  addr | hex    |    BE |    LE | char\n 30020 | 0x0001 |     1 |   256 | ..\n 30021 | 0x0A26 |  2598 |  9738 | .&\n"
        );
        let b = Response { command: "rc", addr: 5, values: Values::Bits(vec![true]), write: false };
        assert_eq!(fmt(OutFmt::Columns).response(&b), "  addr | value\n     5 | true\n");
    }

    #[test]
    fn writes_silent_unless_structured() {
        let w = Response { command: "wh", addr: 1, values: Values::Registers(vec![7]), write: true };
        assert_eq!(fmt(OutFmt::Columns).response(&w), "");
        assert_eq!(fmt(OutFmt::List).response(&w), "");
        assert!(fmt(OutFmt::Json).response(&w).contains("\"values\": [7]"));
    }

    #[test]
    fn failures() {
        let e = Error::Exception { function: 4, code: 2 };
        let f = Failure { command: "ri", addr: Some(1), count: Some(2), error: &e };
        let s = fmt(OutFmt::Json).failure(&f).unwrap();
        assert!(s.starts_with("{\n\t\"error_code\": 2,"));
        assert!(s.contains("\"error\": \"modbus exception 0x02"));
        assert!(fmt(OutFmt::Columns).failure(&f).is_none());
        let e = Error::Usage("bad \"x\"".into());
        let f = Failure { command: "1x", addr: None, count: None, error: &e };
        assert_eq!(fmt(OutFmt::Yml).failure(&f).unwrap(), "error_code: -4\ncommand: \"1x\"\nerror: \"bad \\\"x\\\"\"\n");
    }
}
