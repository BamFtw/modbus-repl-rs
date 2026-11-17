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
| `-c`, `--command` | run command(s) separated by `;`, print the results, then exit | – |
| `-s`, `--script` | run the commands of a script file (`-` reads stdin), then exit | – |
| `-f`, `--fmt` | output format: `json`, `yml`, `columns`, `list` | `columns` (interactive), `json` (`--command`, `--script`) |
| `-e`, `--endian` | register byte order: `big`, `little` | `big` |
| `-n`, `--number-fmt` | register values: `dec`, `signed`, `hex`, `oct`, `bin` | `dec` |
| `--coil-fmt` | coil / discrete input values: `bool`, `int` | `bool` |
| `--config` | load defaults from a stored configuration (see below) | `default` if it exists |
| `--help`, `-V` | help / version | |

Without `--command` or `--script` the program starts an interactive prompt. With either one it runs headless and prints one result per command. The exit code is `1` if any command failed and `2` for invalid arguments.

## Output formats

`json` (the default with `--command`):

```json
{
	"error_code": 0,
	"command": "ri",
	"address": 30000,
	"count": 3,
	"values": [65535, 65535, 0]
}
```

`yml` holds the same fields in YAML. With several commands, the YAML documents are separated by `---`.

`columns` (the default in the interactive prompt):

```
  addr | hex    |    BE |    LE | char
 30020 | 0x0001 |     1 |   256 | ..
 30021 | 0x0A26 |  2598 |  9738 | .&
```

`list`:

```
65535, 65535, 0
```

- `--number-fmt` sets how register values are shown. With `hex`, `oct` or `bin`, JSON and YAML output the values as strings (`"0x0A26"`) so the notation is kept.
- `--endian little` swaps the two bytes of each register. In `columns` the `BE` and `LE` columns are always both shown, and `--endian` applies to the `hex` and `char` columns.
- `--coil-fmt` shows coils as `true`/`false` (`bool`) or `1`/`0` (`int`).
- In `json`/`yml`, writes also produce a document with the written values. In `columns`/`list`, writes print nothing.
- In `json`/`yml`, errors are part of the output on stdout and include an `error` message. `error_code` is `0` on success, the Modbus exception code (1–255) for device exceptions, `-1` for I/O errors, `-2` for timeouts, `-3` for protocol errors and `-4` for invalid commands. In `columns`/`list`, errors go to stderr.

## Scripts

A script is a text file with the same commands as `--command` and the interactive prompt: one or more per line, separated by `;`. Text after `#` is a comment. The `wait MS` command pauses for `MS` milliseconds.

```sh
# setup.mbs: switch the output on, wait for it to settle, read the status
wc 10 1            # enable output
wait 500
ri 30020 2
fmt list; rh 0 4
```

```sh
modbus-repl -h 10.0.0.5 -s setup.mbs
cat setup.mbs | modbus-repl -h 10.0.0.5 -s -
```

- The whole script is checked before anything is sent. If any line has a syntax error, every error is listed with its line number and nothing is run (exit code `2`). A script with no commands (empty, or only comments) is also an error.
- When a command fails at run time (timeout, Modbus exception, ...), the script goes on with the next command and exits with code `1` at the end, the same as `--command`. `exit` ends the script early.
- Each command is printed to stderr as `> COMMAND` just before it runs. stdout only gets the results, so `modbus-repl -s setup.mbs > out.json` still produces clean output.
- Errors name the script line: `ERROR: line 6: ...` on stderr, or a `line` field in `json`/`yml` output.
- `--script` and `--command` can't be used together.

## Stored configurations

A configuration is a named set of default values for the options above, so you don't have to type them every time:

```sh
modbus-repl config add ts2ra            # a dialog asks for each value
modbus-repl --config ts2ra -u 10        # use ts2ra; options on the command line override it
```

| Command | Description |
|---|---|
| `modbus-repl config start` | edit the `default` configuration (same as `config edit default`, but creates it if missing) |
| `modbus-repl config list` | list the stored configurations and their values |
| `modbus-repl config add NAME` | create a configuration with the dialog |
| `modbus-repl config edit NAME` | change a configuration with the dialog |
| `modbus-repl config remove NAME` | delete a configuration |
| `modbus-repl config export NAME PATH` | write a configuration to a file |
| `modbus-repl config import PATH NAME` | store a file as a configuration (the file is checked first) |

- The `default` configuration is used whenever `--config` is not given.
- In the dialog, Enter keeps the value shown in brackets, `-` goes back to the built-in default, and Ctrl+C cancels without saving. All options are asked, the TCP/UDP and RTU ones included, so one configuration can hold both.
- `--command` can't be stored in a configuration.
- If the target of `import` or `export` already exists, you are asked before it is overwritten. When stdin is not a terminal, the command fails instead.
- Configurations are stored as `NAME.conf` text files (`key = value`, keys are the long option names) in `%APPDATA%\modbus-repl` on Windows and `$XDG_CONFIG_HOME/modbus-repl` (or `~/.config/modbus-repl`) elsewhere. Set `MODBUS_REPL_CONFIG_DIR` to use another directory.

```ini
# modbus-repl configuration 'ts2ra'
proto = rtu
device = COM3
baud = 19200
fmt = list
```

## Commands

```sh
rc A N          # read N coils
rd A N          # read N discrete inputs
ri A N          # read N input registers
rh A N          # read N holding registers
wc A C1 .. Cn   # write coils (1/0)
wh A R1 .. Rn   # write holding registers (0..65535, or -32768..-1)
fmt [FORMAT]    # switch the output format (json, yml, columns, list); without an argument, show the current one
wait MS         # pause for MS milliseconds
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
modbus-repl -h 10.0.0.5 -f list -n hex -e little -c "ri 30000 4"
modbus-repl -h 10.0.0.5 -s setup.mbs
```

## Notes

- On Linux, RTU supports the standard baud rates from 300 to 2 000 000.
- In RTU mode, unit `0` (broadcast) can only be used with write commands.
- After a TCP connection drops or a request times out, the client reconnects on the next command.

## License

[MIT](LICENSE)
