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
    derive_more::From,
    serialport::SerialPort as _,
};
#[cfg(unix)] use serialport::TTYPort as NativePort;
#[cfg(windows)] use serialport::COMPort as NativePort;

const TEST_TIMEOUT: Duration = Duration::from_millis(200); // 200ms in the sample code
const REGULAR_TIMEOUT: Duration = Duration::from_secs(2); // 2 seconds in the sample code

#[derive(Debug, From)]
enum ErrorKind {
    Io(io::Error),
    SerialPort(serialport::Error),
    UnknownReply([u8; 4]),
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

trait ResultExt {
    type Ok;

    fn at(self, location: &'static str) -> Result<Self::Ok, Error>;
}

impl<T, E: Into<ErrorKind>> ResultExt for Result<T, E> {
    type Ok = T;

    fn at(self, location: &'static str) -> Result<T, Error> {
        self.map_err(|e| Error { kind: e.into(), location })
    }
}

enum State {
    MainMenu,
    InGame,
}

fn test_port(port_info: serialport::SerialPortInfo) -> Result<(NativePort, State), Error> {
    let mut port = serialport::new(port_info.port_name, 9_600)
        .timeout(TEST_TIMEOUT)
        .open_native().at("test_port open")?;
    write!(port, "cmdt\0\0\0\0\0\0\0\0\0\0\0\0").at("test_port send")?;
    port.flush().at("test_port flush")?;
    let mut cmd = [0; 16];
    port.read_exact(&mut cmd).at("receive prefix read")?;
    match &cmd {
        [b'O', b'o', b'T', b'R', ..] => Ok((port, State::InGame)),
        [b'c', b'm', b'd', b'r', ..] => Ok((port, State::MainMenu)),
        [b'c', b'm', b'd', b'k', ..] => Ok((port, State::MainMenu)), // older versions of EverDrive OS
        _ => Err(ErrorKind::UnknownReply(cmd[..4].try_into().unwrap())).at("receive command check"),
    }
}

#[wheel::main]
fn main() -> Result<(), Error> {
    for port_info in serialport::available_ports().map_err(|e| Error { location: "list available ports", kind: e.into() })? {
        println!("testing {port_info:?}");
        match test_port(port_info) {
            Ok((_, State::MainMenu)) => {
                println!("success, in main menu");
                return Ok(())
            }
            Ok((mut port, State::InGame)) => {
                println!("success, in game");
                port.set_timeout(REGULAR_TIMEOUT).map_err(|e| Error { location: "set regular timeout", kind: e.into() })?;
                return Ok(())
            }
            Err(e) => println!("failed: {e}"),
        }
    }
    println!("all ports failed");
    Ok(())
}
