# modbus-repl

Interactive / headless Modbus client written in Rust. It builds into a single standalone executable with **no external crates**: CLI parsing, Modbus framing and serial port access all use only the standard library plus the OS APIs (Win32 comm API / Linux termios).

Supported protocols:

| `--proto` | Transport                          |
|-----------|------------------------------------|
| `tcp`     | Modbus TCP (MBAP), default         |
| `udp`     | Modbus over UDP (MBAP datagrams)   |
| `rtu`     | Modbus RTU over a serial port (CRC16) |

## Building

```sh
cargo build --release
```

Static release binaries (what CI produces):

```sh
# Linux, fully static (musl)
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl

# Windows (native, or cross from Linux with mingw-w64 installed)
rustup target add x86_64-pc-windows-gnu
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
  cargo build --release --target x86_64-pc-windows-gnu
```

The Forgejo pipeline in `.forgejo/workflows/build.yml` runs clippy and the tests, then builds both executables and uploads them as artifacts.

## CLI arguments

| Option | Description | Default |
|---|---|---|
| `-P`, `--proto` | `tcp`, `udp` or `rtu` | `tcp` |
| `-h`, `--host` | server address (TCP/UDP) | `localhost` |
| `-p`, `--port` | server port (TCP/UDP) | `502` |
| `-d`, `--device` | serial port for RTU, e.g. `COM3`, `/dev/ttyUSB0` | – |
| `-b`, `--baud` | baud rate (RTU) | `9600` |
| `--parity` | `none`, `even`, `odd` (RTU) | `none` |
| `--data-bits` | 7 or 8 (RTU) | `8` |
| `--stop-bits` | 1 or 2 (RTU) | `1` |
| `-u`, `--unit` | unit / slave id | `1` |
| `-t`, `--timeout` | response timeout in ms | `1500` |
| `-c`, `--command` | run command(s) separated by `;`, print JSON, then exit | – |
| `--help`, `-V` | help / version | |

Without `--command` the program starts an interactive prompt. With `--command` it runs headless: read results are printed as one JSON array per command (`[1,2,3]` or `[true,false]`). The exit code is `1` if any command failed and `2` for invalid arguments.

## Commands

```sh
rc A N          # read N coils
rd A N          # read N discrete inputs
ri A N          # read N input registers
rh A N          # read N holding registers
wc A C1 .. Cn   # write coils (1/0)
wh A R1 .. Rn   # write holding registers (0..65535, or -32768..-1)
help
exit | quit
```

Numbers can be decimal, hex (`0x..`) or binary (`0b..`). `N` defaults to 1. Several commands can go on one line, separated by `;`.

## Examples

```sh
modbus-repl -h 192.168.1.10
modbus-repl -P udp -h 10.0.0.5 -p 5020 -c "rh 0 10"
modbus-repl -P rtu -d COM3 -b 19200 --parity even -u 17
modbus-repl -P rtu -d /dev/ttyUSB0 -c "wh 100 0x1234 42; rh 100 2"
```

## Notes

- On Linux, RTU supports the standard baud rates from 300 to 2 000 000.
- In RTU mode, unit `0` (broadcast) can only be used with write commands.
- After a TCP connection drops or a request times out, the client reconnects on the next command.

## License

[MIT](LICENSE)
