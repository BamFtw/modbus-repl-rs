//! Stored configurations: named sets of default option values.
//!
//! `modbus-repl config ...` manages them, `--config NAME` applies one, and
//! the configuration named `default` is applied when `--config` is not given.
//!
//! Each configuration is a text file `<dir>/<NAME>.conf` with one
//! `key = value` line per option (keys are the long option names without
//! `--`). Lines starting with `#` are comments.

use crate::cli::{self, Config};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub const DEFAULT_NAME: &str = "default";
const EXT: &str = "conf";

/// Options that can be stored, in file / dialog order:
/// (key, description, built-in default shown in the dialog).
const OPTIONS: &[(&str, &str, &str)] = &[
    ("proto", "protocol: tcp, udp or rtu", "tcp"),
    ("host", "server address (tcp/udp)", "localhost"),
    ("port", "server port (tcp/udp)", "502"),
    ("device", "serial port, e.g. COM3 or /dev/ttyUSB0 (rtu)", "not set"),
    ("baud", "baud rate (rtu)", "9600"),
    ("parity", "parity: none, even or odd (rtu)", "none"),
    ("data-bits", "data bits: 7 or 8 (rtu)", "8"),
    ("stop-bits", "stop bits: 1 or 2 (rtu)", "1"),
    ("unit", "unit / slave id: 0-255", "1"),
    ("timeout", "response timeout in ms", "1500"),
    ("fmt", "output format: json, yml, columns or list", "columns, json with --command"),
    ("endian", "register byte order: big or little", "big"),
    ("number-fmt", "register values: dec, signed, hex, oct or bin", "dec"),
    ("coil-fmt", "coil values: bool or int", "bool"),
];


pub const USAGE: &str = "\
USAGE:
    modbus-repl config start                  edit the default configuration
    modbus-repl config list                   list stored configurations
    modbus-repl config add NAME               create a configuration (dialog)
    modbus-repl config edit NAME              change a configuration (dialog)
    modbus-repl config remove NAME            delete a configuration
    modbus-repl config export NAME PATH       save a configuration to a file
    modbus-repl config import PATH NAME       store a file as a configuration

The configuration 'default' is used when '--config NAME' is not given.
Options given on the command line always override stored values.
The storage directory can be changed with the MODBUS_REPL_CONFIG_DIR
environment variable.
";

/// Stored settings: (key, value) pairs in `OPTIONS` order.
pub type Entries = Vec<(String, String)>;

fn get<'a>(entries: &'a Entries, key: &str) -> Option<&'a str> {
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn set(entries: &mut Entries, key: &str, value: Option<String>) {
    entries.retain(|(k, _)| k != key);
    if let Some(v) = value {
        entries.push((key.to_string(), v));
    }
    let pos = |k: &str| OPTIONS.iter().position(|o| o.0 == k).unwrap_or(usize::MAX);
    entries.sort_by_key(|(k, _)| pos(k));
}

/// Check a single value the same way the command line does.
fn validate(key: &str, value: &str) -> Result<(), String> {
    cli::apply(&mut Config::default(), key, value)
}

/// Directory holding the configuration files.
pub fn dir() -> Result<PathBuf, String> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(d) = env("MODBUS_REPL_CONFIG_DIR") {
        return Ok(d);
    }
    let base = if cfg!(windows) {
        env("APPDATA")
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    base.map(|b| b.join("modbus-repl"))
        .ok_or_else(|| "cannot determine the configuration directory, set MODBUS_REPL_CONFIG_DIR".into())
}

fn check_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(format!("invalid configuration name '{name}' (use letters, digits, '-', '_' and '.')"))
    }
}

fn path_of(name: &str) -> Result<PathBuf, String> {
    check_name(name)?;
    Ok(dir()?.join(format!("{name}.{EXT}")))
}

/// Parse and validate the contents of a configuration file.
pub fn parse_text(text: &str) -> Result<Entries, String> {
    let mut entries = Entries::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |msg: String| format!("line {}: {msg}", n + 1);
        let (key, value) = line.split_once('=').ok_or_else(|| err("expected 'key = value'".into()))?;
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        if !OPTIONS.iter().any(|o| o.0 == key) {
            return Err(err(format!("unknown option '{key}'")));
        }
        validate(&key, value).map_err(err)?;
        set(&mut entries, &key, Some(value.to_string()));
    }
    Ok(entries)
}

fn to_text(name: &str, entries: &Entries) -> String {
    let mut out = format!("# modbus-repl configuration '{name}'\n");
    for (k, v) in entries {
        out += &format!("{k} = {v}\n");
    }
    out
}

fn read_file(path: &Path) -> Result<Entries, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
    parse_text(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_file(path: &Path, name: &str, entries: &Entries) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create '{}': {e}", parent.display()))?;
    }
    fs::write(path, to_text(name, entries)).map_err(|e| format!("cannot write '{}': {e}", path.display()))
}

/// Settings for `--config NAME`, or of the default configuration when `name`
/// is `None` (empty when there is no default configuration).
pub fn load(name: Option<&str>) -> Result<Entries, String> {
    match name {
        Some(name) => {
            let path = path_of(name)?;
            if !path.exists() {
                return Err(format!("configuration '{name}' not found (see 'modbus-repl config list')"));
            }
            read_file(&path)
        }
        None => match dir() {
            Ok(d) => {
                let path = d.join(format!("{DEFAULT_NAME}.{EXT}"));
                if path.exists() {
                    read_file(&path)
                } else {
                    Ok(Entries::new())
                }
            }
            // Without a known directory there simply is no default configuration.
            Err(_) => Ok(Entries::new()),
        },
    }
}

fn list_names() -> Result<Vec<String>, String> {
    let d = dir()?;
    let rd = match fs::read_dir(&d) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("cannot read '{}': {e}", d.display())),
    };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == EXT))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
        .filter(|n| check_name(n).is_ok())
        .collect();
    names.sort();
    Ok(names)
}

/// Read one trimmed line; `None` on end of input.
fn read_line(input: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_string()))
}

fn ask_yes_no(input: &mut impl BufRead, out: &mut impl Write, question: &str, default: bool) -> io::Result<bool> {
    loop {
        write!(out, "{question} [{}] ", if default { "Y/n" } else { "y/N" })?;
        out.flush()?;
        match read_line(input)?.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None => return Ok(false),
            Some("") => return Ok(default),
            Some("y" | "yes") => return Ok(true),
            Some("n" | "no") => return Ok(false),
            _ => writeln!(out, "  please answer y or n")?,
        }
    }
}

/// Interactive editor for the settings of configuration `name`.
/// Returns the new settings, or `None` when the user cancelled.
pub fn dialog(
    input: &mut impl BufRead,
    out: &mut impl Write,
    name: &str,
    mut entries: Entries,
) -> io::Result<Option<Entries>> {
    writeln!(out, "Configuration '{name}'")?;
    writeln!(out, "Press Enter to keep the value in brackets, type '-' to use the built-in default.")?;
    writeln!(out, "Press Ctrl+C to cancel without saving.")?;
    writeln!(out)?;

    for &(key, desc, builtin) in OPTIONS {
        loop {
            let current = match get(&entries, key) {
                Some(v) => v.to_string(),
                None => format!("default: {builtin}"),
            };
            write!(out, "  {key:<11} {desc}\n  {:<11} [{current}] > ", "")?;
            out.flush()?;
            let Some(answer) = read_line(input)? else {
                writeln!(out)?;
                return Ok(None);
            };
            match answer.as_str() {
                "" => break,
                "-" => {
                    set(&mut entries, key, None);
                    break;
                }
                v => match validate(key, v) {
                    Ok(()) => {
                        set(&mut entries, key, Some(v.to_string()));
                        break;
                    }
                    Err(e) => writeln!(out, "  ERROR: {e}")?,
                },
            }
        }
    }

    writeln!(out)?;
    if entries.is_empty() {
        writeln!(out, "No values set, all options use their built-in defaults.")?;
    } else {
        writeln!(out, "Stored values:")?;
        for (k, v) in &entries {
            writeln!(out, "  {k} = {v}")?;
        }
    }
    let save = ask_yes_no(input, out, &format!("Save configuration '{name}'?"), true)?;
    Ok(save.then_some(entries))
}

/// Ask before replacing an existing file; refuse when not interactive.
fn confirm_overwrite(what: &str) -> Result<(), String> {
    if !io::stdin().is_terminal() {
        return Err(format!("{what} already exists"));
    }
    let ok = ask_yes_no(&mut io::stdin().lock(), &mut io::stdout(), &format!("{what} already exists. Overwrite?"), false)
        .map_err(|e| e.to_string())?;
    if ok {
        Ok(())
    } else {
        Err("cancelled".into())
    }
}

fn edit(name: &str, must_exist: bool, must_not_exist: bool) -> Result<(), String> {
    let path = path_of(name)?;
    let exists = path.exists();
    if must_exist && !exists {
        return Err(format!("configuration '{name}' not found, create it with 'modbus-repl config add {name}'"));
    }
    if must_not_exist && exists {
        return Err(format!("configuration '{name}' already exists, change it with 'modbus-repl config edit {name}'"));
    }
    let entries = if exists { read_file(&path)? } else { Entries::new() };
    let result = dialog(&mut io::stdin().lock(), &mut io::stdout(), name, entries).map_err(|e| e.to_string())?;
    match result {
        Some(entries) => {
            write_file(&path, name, &entries)?;
            println!("Saved configuration '{name}' to {}", path.display());
        }
        None => println!("Cancelled, nothing saved."),
    }
    Ok(())
}

fn list() -> Result<(), String> {
    let d = dir()?;
    let names = list_names()?;
    if names.is_empty() {
        println!("No configurations stored in {}", d.display());
        println!("Create one with 'modbus-repl config add NAME' or 'modbus-repl config start'.");
        return Ok(());
    }
    println!("Configurations in {}:", d.display());
    let width = names.iter().map(String::len).max().unwrap_or(0);
    for name in names {
        let summary = match read_file(&d.join(format!("{name}.{EXT}"))) {
            Ok(e) if e.is_empty() => "(built-in defaults)".to_string(),
            Ok(e) => e.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "),
            Err(e) => format!("ERROR: {e}"),
        };
        println!("  {name:<width$}  {summary}");
    }
    Ok(())
}

fn remove(name: &str) -> Result<(), String> {
    let path = path_of(name)?;
    if !path.exists() {
        return Err(format!("configuration '{name}' not found"));
    }
    fs::remove_file(&path).map_err(|e| format!("cannot remove '{}': {e}", path.display()))?;
    println!("Removed configuration '{name}'");
    Ok(())
}

fn export(name: &str, target: &Path) -> Result<(), String> {
    let path = path_of(name)?;
    if !path.exists() {
        return Err(format!("configuration '{name}' not found"));
    }
    let entries = read_file(&path)?;
    if target.exists() {
        confirm_overwrite(&format!("file '{}'", target.display()))?;
    }
    fs::write(target, to_text(name, &entries)).map_err(|e| format!("cannot write '{}': {e}", target.display()))?;
    println!("Exported configuration '{name}' to {}", target.display());
    Ok(())
}

fn import(source: &Path, name: &str) -> Result<(), String> {
    let path = path_of(name)?;
    let entries = read_file(source)?;
    if path.exists() {
        confirm_overwrite(&format!("configuration '{name}'"))?;
    }
    write_file(&path, name, &entries)?;
    println!("Imported {} as configuration '{name}'", source.display());
    Ok(())
}

/// Entry point for `modbus-repl config ...` (`args` excludes `config`).
pub fn run(args: &[String]) -> ExitCode {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["start"] => edit(DEFAULT_NAME, false, false),
        ["list"] => list(),
        ["add", name] => edit(name, false, true),
        ["edit", name] => edit(name, true, false),
        ["remove" | "rm", name] => remove(name),
        ["export", name, path] => export(name, Path::new(path)),
        ["import", path, name] => import(Path::new(path), name),
        [] | ["help" | "--help" | "-?"] => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("ERROR: invalid config command\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ERROR: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(pairs: &[(&str, &str)]) -> Entries {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn run_dialog(input: &str, entries: Entries) -> (Option<Entries>, String) {
        let mut out = Vec::new();
        let r = dialog(&mut input.as_bytes(), &mut out, "test", entries).unwrap();
        (r, String::from_utf8(out).unwrap())
    }

    #[test]
    fn all_options_known_to_cli() {
        for (key, _, _) in OPTIONS {
            let e = validate(key, "").err().unwrap_or_default();
            assert!(!e.contains("unknown option"), "{key}");
        }
    }

    #[test]
    fn text_roundtrip() {
        let e = parse_text("# comment\n\n unit = 10 \nPROTO=rtu\ndevice = COM3\n").unwrap();
        assert_eq!(e, kv(&[("proto", "rtu"), ("device", "COM3"), ("unit", "10")]));
        assert_eq!(parse_text(&to_text("x", &e)).unwrap(), e);
        assert!(parse_text("unit = 300").unwrap_err().starts_with("line 1:"));
        assert!(parse_text("\ncolour = red").unwrap_err().contains("line 2: unknown option"));
        assert!(parse_text("command = rh 0 1").is_err());
        assert!(parse_text("just text").is_err());
    }

    #[test]
    fn names() {
        assert!(check_name("ts2ra").is_ok());
        assert!(check_name("site-1.b_2").is_ok());
        for bad in ["", ".hidden", "a/b", "..", "a b", "a\\b"] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn dialog_asks_every_option() {
        // proto (keep), host, port (invalid then valid), device, baud, parity, data-bits,
        // stop-bits, unit, timeout, fmt, endian, number-fmt, coil-fmt, save
        let input = "\n10.0.0.5\n0\n5020\n\n\nodd\n7\n2\n7\n\nlist\n\n\n\ny\n";
        let (r, out) = run_dialog(input, Entries::new());
        assert_eq!(
            r.unwrap(),
            kv(&[
                ("host", "10.0.0.5"),
                ("port", "5020"),
                ("parity", "odd"),
                ("data-bits", "7"),
                ("stop-bits", "2"),
                ("unit", "7"),
                ("fmt", "list")
            ])
        );
        assert!(out.contains("ERROR: 'port' must not be 0"));
        for (key, _, _) in OPTIONS {
            assert!(out.contains(&format!("  {key} ")), "{key} not asked");
        }
    }

    #[test]
    fn dialog_keeps_and_resets() {
        let start = kv(&[("host", "1.2.3.4"), ("unit", "3")]);
        // proto=rtu, host/port kept, device, baud, parity, data/stop kept, unit reset,
        // rest kept, save (default yes)
        let input = "rtu\n\n\nCOM3\n19200\neven\n\n\n-\n\n\n\n\n\n\n";
        let (r, out) = run_dialog(input, start);
        assert_eq!(
            r.unwrap(),
            kv(&[("proto", "rtu"), ("host", "1.2.3.4"), ("device", "COM3"), ("baud", "19200"), ("parity", "even")])
        );
        assert!(out.contains("[default: 9600]"));
    }

    #[test]
    fn dialog_cancel() {
        assert!(run_dialog("tcp\nhost\n", Entries::new()).0.is_none()); // end of input
        let all_enter = "\n".repeat(OPTIONS.len());
        assert!(run_dialog(&format!("{all_enter}n\n"), Entries::new()).0.is_none()); // answered no
    }
}
