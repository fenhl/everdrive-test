use {
    std::{
        io::{
            self,
            prelude::*,
        },
        num::NonZeroU8,
        string::FromUtf8Error,
        thread::sleep,
        time::Duration,
    },
    arrayref::array_ref,
    chrono::prelude::*,
    enum_iterator::all,
    ootr_utils::spoiler::HashIcon,
    serialport::SerialPort as _,
};
#[cfg(unix)] use {
    std::{
        ffi::OsString,
        path::{
            Path,
            PathBuf,
        },
    },
    serialport::TTYPort as NativePort,
};
#[cfg(windows)] use serialport::COMPort as NativePort;

const TEST_TIMEOUT: Duration = Duration::from_millis(200); // 200ms in the sample code
const REGULAR_TIMEOUT: Duration = Duration::from_secs(10); // twice the ping interval

const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, thiserror::Error)]
enum ErrorKind {
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] SerialPort(#[from] serialport::Error),
    #[error(transparent)] Utf8(#[from] FromUtf8Error),
    #[error("all USB ports failed")]
    AllPortsFailed,
    #[error("failed to decode hash icon")]
    HashIcon,
    #[cfg(unix)]
    #[error("non-UTF-8 string: {}", .0.to_string_lossy())]
    OsString(OsString),
    #[error("N64 reported world 0")]
    PlayerId,
    #[cfg(unix)]
    #[error("found USB port at file system root")]
    PortAtRoot,
    #[error("unsupported randomizer version")]
    RandoVersion,
    #[error("unexpected handshake reply: {0:x?}")]
    UnknownReply([u8; 4]),
}

#[cfg(unix)]
impl From<OsString> for ErrorKind {
    fn from(s: OsString) -> Self {
        Self::OsString(s)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("error at {location}: {kind}")]
struct Error {
    location: &'static str,
    #[source] kind: ErrorKind,
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

#[derive(Debug)]
enum Connection {
    MainMenu,
    InGame {
        rando_version: ootr_utils::Version,
        player_id: NonZeroU8,
        file_hash: [HashIcon; 5],
        port: NativePort,
    },
}

fn connect_to_port(port_info: serialport::SerialPortInfo) -> Result<Connection, Error> {
    #[cfg(unix)] let port_path = PathBuf::from("/dev").join(Path::new(&port_info.port_name).file_name().ok_or(ErrorKind::PortAtRoot).at("connect_to_port path builder")?).into_os_string().into_string().at("connect_to_port path builder")?;
    #[cfg(windows)] let port_path = port_info.port_name;
    let mut port = serialport::new(port_path, 9_600)
        .timeout(TEST_TIMEOUT)
        .open_native().at("test_port open")?;
    port.write_all(b"cmdt\0\0\0\0\0\0\0\0\0\0\0\0").at("connect_to_port send cmdt")?;
    port.flush().at("test_port flush")?;
    let mut cmd = [0; 16];
    port.read_exact(&mut cmd).at("receive prefix read")?;
    match cmd {
        [b'O', b'o', b'T', b'R', PROTOCOL_VERSION, major, minor, patch, branch, supplementary, player_id, hash1, hash2, hash3, hash4, hash5] => {
            port.set_timeout(REGULAR_TIMEOUT).map_err(|e| Error { location: "set regular timeout", kind: e.into() })?;
            let mut buf = [0; 16];
            buf[0] = b'M';
            buf[1] = b'W';
            buf[2] = PROTOCOL_VERSION;
            buf[3] = 1; // enable MW_SEND_OWN_ITEMS
            buf[4] = 1; // enable MW_PROGRESSIVE_ITEMS_ENABLE
            port.write_all(&buf).at("connect_to_port send MW")?;
            Ok(Connection::InGame {
                rando_version: ootr_utils::Version::from_bytes([major, minor, patch, branch, supplementary]).ok_or(ErrorKind::RandoVersion).at("connect_to_port")?,
                player_id: NonZeroU8::new(player_id).ok_or(ErrorKind::PlayerId).at("connect_to_port")?,
                file_hash: [
                    all().nth(hash1.into()).ok_or(ErrorKind::HashIcon).at("connect_to_port")?,
                    all().nth(hash2.into()).ok_or(ErrorKind::HashIcon).at("connect_to_port")?,
                    all().nth(hash3.into()).ok_or(ErrorKind::HashIcon).at("connect_to_port")?,
                    all().nth(hash4.into()).ok_or(ErrorKind::HashIcon).at("connect_to_port")?,
                    all().nth(hash5.into()).ok_or(ErrorKind::HashIcon).at("connect_to_port")?,
                ],
                port,
            })
        }
        [b'c', b'm', b'd', b'r', ..] => Ok(Connection::MainMenu),
        [b'c', b'm', b'd', b'k', ..] => Ok(Connection::MainMenu), // older versions of EverDrive OS
        _ => Err(ErrorKind::UnknownReply(*array_ref!(cmd, 0, 4))).at("receive command check"),
    }
}

#[wheel::main]
fn main() -> Result<(), Error> {
    let mut port_errors = Vec::default();
    for port_info in serialport::available_ports().map_err(|e| Error { location: "list available ports", kind: e.into() })? {
        let port_info_debug = format!("{port_info:?}");
        match connect_to_port(port_info) {
            Ok(Connection::MainMenu) => {
                println!("in main menu");
                return Ok(())
            }
            Ok(Connection::InGame { rando_version, player_id, file_hash, mut port }) => {
                println!("in game, randomizer version: {rando_version}, world {player_id}, hash: {file_hash:?}");
                let mut buf = [0; 16];
                loop {
                    for _ in 0..5 {
                        let bytes_read = port.read(&mut buf).at("read")?;
                        if bytes_read > 0 {
                            port.read_exact(&mut buf[bytes_read..]).at("read rest")?;
                            println!("{} N64: {buf:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"));
                        } else {
                            println!("nothing read");
                        }
                        sleep(Duration::from_secs(1));
                    }
                    port.write_all(b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").at("send ping")?;
                    port.read_exact(&mut buf).at("receive pong")?;
                }
            }
            Err(e) => port_errors.push((port_info_debug, e)),
        }
    }
    if port_errors.is_empty() {
        eprintln!("no ports found");
    } else {
        eprintln!("all ports failed:");
        for (port_info_debug, error) in port_errors {
            eprintln!("{port_info_debug}: {error}");
        }
    }
    Err(ErrorKind::AllPortsFailed).at("main")
}
