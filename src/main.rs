use {
    std::{
        fmt,
        io::{
            self,
            prelude::*,
        },
        string::FromUtf8Error,
        time::Duration,
    },
    byteorder::{
        BigEndian,
        WriteBytesExt as _,
    },
    derive_more::From,
    serialport::SerialPort as _,
};
#[cfg(unix)] use serialport::TTYPort as NativePort;
#[cfg(windows)] use serialport::COMPort as NativePort;

const TEST_TIMEOUT: Duration = Duration::from_millis(200); // 200ms in the sample code
const REGULAR_TIMEOUT: Duration = Duration::from_secs(2); // 2 seconds in the sample code

enum Command {
    RamRead {
        address: u32,
        length: i32,
    },
    TestConnection,
}

#[derive(Debug, From)]
enum ErrorKind {
    Io(io::Error),
    OutdatedOs,
    SerialPort(serialport::Error),
    TestFailed,
    UnknownReply(u8),
    Utf8(FromUtf8Error),
}

struct Error {
    location: &'static str,
    kind: ErrorKind,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error at {}: {:?}", self.location, self.kind)
    }
}

impl Command {
    fn write_parts(&self) -> (u8, u32, i32, u32) {
        match self {
            Command::RamRead { address, length } => (b'r', *address, *length, 0),
            Command::TestConnection => (b't', 0, 0, 0),
        }
    }
}

fn send(port: &mut NativePort, command: &Command) -> io::Result<()> {
    write!(port, "cmd")?;
    let (cmd, address, length, argument) = command.write_parts();
    port.write_u8(cmd)?;
    port.write_u32::<BigEndian>(address)?;
    port.write_i32::<BigEndian>(length)?;
    port.write_u32::<BigEndian>(argument)?;
    port.flush()?;
    Ok(())
}

fn receive(port: &mut NativePort) -> Result<(), Error> {
    let mut cmd = vec![0; 16];
    port.read_exact(&mut cmd).map_err(|e| Error { location: "receive prefix read", kind: e.into() })?;
    if String::from_utf8(cmd[..3].to_owned()).map_err(|e| Error { location: "receive prefix check", kind: e.into() })?.to_ascii_lowercase().starts_with("cmd") {
        match cmd[3] {
            b'r' => Ok(()),
            b'k' => Err(Error { location: "receive command check", kind: ErrorKind::OutdatedOs }),
            c => Err(Error { location: "receive command check", kind: ErrorKind::UnknownReply(c) }),
        }
    } else {
        Err(Error { location: "receive prefix check", kind: ErrorKind::TestFailed })
    }
}

fn test_port(port_info: serialport::SerialPortInfo) -> Result<NativePort, Error> {
    let mut port = serialport::new(port_info.port_name, 9_600)
        .timeout(TEST_TIMEOUT)
        .open_native().map_err(|e| Error { location: "test_port open", kind: e.into() })?;
    send(&mut port, &Command::TestConnection).map_err(|e| Error { location: "test_port send", kind: e.into() })?;
    receive(&mut port)?;
    Ok(port)
}

#[wheel::main]
fn main() -> Result<(), Error> {
    let stdin = io::stdin();
    for port_info in serialport::available_ports().map_err(|e| Error { location: "list available ports", kind: e.into() })? {
        println!("testing {:?}", port_info);
        match test_port(port_info) {
            Ok(mut port) => {
                println!("success");
                port.set_timeout(REGULAR_TIMEOUT).map_err(|e| Error { location: "set regular timeout", kind: e.into() })?;
                println!("start the game, load a save file, then press return");
                stdin.read_line(&mut String::default()).map_err(|e| Error { location: "read_line", kind: e.into() })?;
                send(&mut port, &Command::RamRead { address: 0x11a5d0 + 0x001c, length: 6 }).map_err(|e| Error { location: "RAM read send", kind: e.into() })?;
                let mut buf = vec![0; 6];
                port.read_exact(&mut buf).map_err(|e| Error { location: "RAM read", kind: e.into() })?;
                if buf == b"ZELDAZ" {
                    println!("RAM check successful");
                } else {
                    println!("unexpected RAM contents: {:?}", buf);
                }
                return Ok(())
            }
            Err(e) => println!("failed: {}", e),
        }
    }
    println!("all ports failed");
    Ok(())
}
