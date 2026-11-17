mod cli;
mod commands;
mod config;
mod format;
mod modbus;
mod script;
mod serial;
mod transport;

use cli::{Action, Config, Proto};
use commands::Command;
use format::{Failure, Format, OutFmt};
use modbus::{Client, Transport};
use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::time::Duration;

fn open_transport(cfg: &Config) -> modbus::Result<(Box<dyn Transport>, String)> {
    Ok(match cfg.proto {
        Proto::Tcp => (
            Box::new(transport::TcpTransport::connect(&cfg.host, cfg.port, cfg.timeout)?),
            format!("{}:{}", cfg.host, cfg.port),
        ),
        Proto::Udp => (
            Box::new(transport::UdpTransport::open(&cfg.host, cfg.port, cfg.timeout)?),
            format!("udp://{}:{}", cfg.host, cfg.port),
        ),
        Proto::Rtu => {
            let device = cfg.device.as_deref().unwrap_or_default();
            let settings = serial::SerialSettings {
                baud: cfg.baud,
                parity: cfg.parity,
                data_bits: cfg.data_bits,
                stop_bits: cfg.stop_bits,
            };
            let port = serial::SerialPort::open(device, &settings).map_err(|e| {
                modbus::Error::Io(io::Error::new(e.kind(), format!("cannot open serial port '{device}': {e}")))
            })?;
            let t = transport::RtuTransport::new(port, cfg.baud, settings.bits_per_char(), cfg.timeout);
            (Box::new(t), device.to_string())
        }
    })
}

fn report(e: &modbus::Error) {
    eprintln!("ERROR: {e}");
}

enum Outcome {
    Ok,
    Failed,
    Exit,
}

/// State shared by the headless and interactive front ends.
struct Session {
    client: Client,
    fmt: Format,
    /// Emit document separators (yml `---`) between outputs (headless only).
    separate: bool,
    printed: usize,
    /// Script line of the command being run, reported with errors.
    line: Option<usize>,
}

impl Session {
    fn emit(&mut self, doc: &str) {
        if doc.is_empty() {
            return;
        }
        if self.separate && self.printed > 0 {
            print!("{}", self.fmt.separator());
        }
        self.printed += 1;
        print!("{doc}");
    }

    fn fail(&mut self, f: &Failure) {
        match self.fmt.failure(f) {
            Some(doc) => self.emit(&doc),
            None => match f.line {
                Some(l) => eprintln!("ERROR: line {l}: {}", f.error),
                None => report(f.error),
            },
        }
    }

    fn run(&mut self, line: &str) -> Outcome {
        let cmd = match commands::parse(line) {
            Ok(cmd) => cmd,
            Err(e) => {
                let command = line.split_whitespace().next().unwrap_or_default();
                self.fail(&Failure { command, addr: None, count: None, line: self.line, error: &e });
                return Outcome::Failed;
            }
        };
        match cmd {
            Command::Exit => return Outcome::Exit,
            Command::Help => {
                let help = cli::USAGE.split("COMMANDS:").nth(1).unwrap_or("");
                let help = help.split("SCRIPTS:").next().unwrap_or("");
                println!("COMMANDS:{}", help.trim_end());
            }
            Command::Fmt(Some(f)) => self.fmt.out = f,
            Command::Fmt(None) => println!("fmt: {}", self.fmt.out.name()),
            Command::Wait(ms) => {
                let _ = io::stdout().flush();
                std::thread::sleep(Duration::from_millis(ms));
            }
            _ => match commands::execute(&mut self.client, &cmd) {
                Ok(resp) => {
                    let doc = self.fmt.response(&resp);
                    self.emit(&doc);
                }
                Err(e) => {
                    self.fail(&Failure {
                        command: cmd.name(),
                        addr: cmd.addr(),
                        count: cmd.count(),
                        line: self.line,
                        error: &e,
                    });
                    return Outcome::Failed;
                }
            },
        }
        Outcome::Ok
    }
}

fn commands_in(line: &str) -> impl Iterator<Item = &str> {
    line.split(';').map(str::trim).filter(|l| !l.is_empty())
}

/// Run commands without a prompt: `--command` (no line numbers) or `--script`.
fn headless(session: &mut Session, steps: &[(Option<usize>, String)]) -> ExitCode {
    let mut code = ExitCode::SUCCESS;
    for (line, text) in steps {
        session.line = *line;
        if line.is_some() {
            // Echo script commands on stderr, so stdout stays parseable.
            let _ = io::stdout().flush();
            eprintln!("> {text}");
        }
        match session.run(text) {
            Outcome::Ok => {}
            Outcome::Failed => code = ExitCode::FAILURE,
            Outcome::Exit => break,
        }
    }
    code
}

fn interactive(session: &mut Session, prompt: &str) -> ExitCode {
    println!("Connection established (unit {}). Type 'help' for commands.", session.client.unit);
    let stdin = io::stdin();
    let mut line = String::new();
    loop {
        print!("{prompt} > ");
        let _ = io::stdout().flush();
        line.clear();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                println!();
                break; // EOF (Ctrl+D / Ctrl+Z)
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("ERROR: {e}");
                return ExitCode::FAILURE;
            }
        }
        for cmd_line in commands_in(&line) {
            if let Outcome::Exit = session.run(cmd_line) {
                return ExitCode::SUCCESS;
            }
        }
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("config") {
        return config::run(&args[1..]);
    }

    let cfg = match cli::parse(args, config::load) {
        Ok(Action::Run(cfg)) => cfg,
        Ok(Action::Help) => {
            print!("{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Ok(Action::Version) => {
            println!("modbus-repl {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("ERROR: {e}");
            return ExitCode::from(2);
        }
    };

    // Commands for headless mode. A script is read and checked before
    // connecting, so a broken script never touches the device.
    let steps: Option<Vec<(Option<usize>, String)>> = if let Some(path) = &cfg.script {
        let text = match script::read(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("ERROR: {e}");
                return ExitCode::from(2);
            }
        };
        match script::parse(&text) {
            Ok(steps) if steps.is_empty() => {
                let source = if path == "-" { "stdin".to_string() } else { format!("'{path}'") };
                eprintln!("ERROR: script {source} contains no commands");
                return ExitCode::from(2);
            }
            Ok(steps) => Some(steps.into_iter().map(|s| (Some(s.line), s.text)).collect()),
            Err(errors) => {
                for e in errors {
                    eprintln!("ERROR: {path}: {e}");
                }
                eprintln!("Script not run, nothing was sent to the device.");
                return ExitCode::from(2);
            }
        }
    } else {
        cfg.command.as_deref().map(|c| commands_in(c).map(|t| (None, t.to_string())).collect())
    };

    let (transport, prompt) = match open_transport(&cfg) {
        Ok(t) => t,
        Err(e) => {
            report(&e);
            return ExitCode::FAILURE;
        }
    };
    let headless_mode = steps.is_some();
    let default_fmt = if headless_mode { OutFmt::Json } else { OutFmt::Columns };
    let mut session = Session {
        client: Client::new(transport, cfg.unit),
        fmt: Format { out: cfg.fmt.unwrap_or(default_fmt), endian: cfg.endian, number: cfg.number, coil: cfg.coil },
        separate: headless_mode,
        printed: 0,
        line: None,
    };

    match &steps {
        Some(steps) => headless(&mut session, steps),
        _ => interactive(&mut session, &prompt),
    }
}
