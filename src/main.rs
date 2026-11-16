mod cli;
mod commands;
mod modbus;
mod serial;
mod transport;

use cli::{Action, Config, Proto};
use commands::Command;
use modbus::{Client, Transport};
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

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
    match e {
        // Usage errors already carry their "ERROR:" prefix.
        modbus::Error::Usage(_) => eprintln!("{e}"),
        _ => eprintln!("ERROR: {e}"),
    }
}

fn headless(client: &mut Client, command: &str) -> ExitCode {
    let mut code = ExitCode::SUCCESS;
    for line in command.split(';').map(str::trim).filter(|l| !l.is_empty()) {
        let result = commands::parse(line).and_then(|cmd| commands::execute(client, &cmd));
        match result {
            Ok(out) => {
                if let Some(json) = out.json() {
                    println!("{json}");
                }
            }
            Err(e) => {
                report(&e);
                code = ExitCode::FAILURE;
            }
        }
    }
    code
}

fn interactive(client: &mut Client, prompt: &str) -> ExitCode {
    println!("Connection established (unit {}). Type 'help' for commands.", client.unit);
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
        for cmd_line in line.split(';').map(str::trim).filter(|l| !l.is_empty()) {
            match commands::parse(cmd_line) {
                Ok(Command::Exit) => return ExitCode::SUCCESS,
                Ok(Command::Help) => {
                    let help = cli::USAGE.split("COMMANDS:").nth(1).unwrap_or("");
                    let help = help.split("EXAMPLES:").next().unwrap_or("");
                    println!("COMMANDS:{}", help.trim_end());
                }
                Ok(cmd) => match commands::execute(client, &cmd) {
                    Ok(out) => print!("{}", out.pretty()),
                    Err(e) => report(&e),
                },
                Err(e) => report(&e),
            }
        }
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let cfg = match cli::parse(std::env::args().skip(1)) {
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

    let (transport, prompt) = match open_transport(&cfg) {
        Ok(t) => t,
        Err(e) => {
            report(&e);
            return ExitCode::FAILURE;
        }
    };
    let mut client = Client::new(transport, cfg.unit);

    match cfg.command.as_deref() {
        Some(cmd) => headless(&mut client, cmd),
        _ => interactive(&mut client, &prompt),
    }
}
