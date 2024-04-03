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
        _ => Err(ErrorKind::UnknownReply(*array_ref![cmd, 0, 4])).at("receive command check"),
    }
}

#[wheel::main]
fn main() -> Result<(), Error> {
    'menu_loop: loop {
        let mut port_errors = Vec::default();
        for port_info in serialport::available_ports().map_err(|e| Error { location: "list available ports", kind: e.into() })? {
            let port_info_debug = format!("{port_info:?}");
            match connect_to_port(port_info) {
                Ok(Connection::MainMenu) => {
                    println!("in main menu");
                    sleep(Duration::from_secs(1));
                    continue 'menu_loop
                }
                Ok(Connection::InGame { rando_version, player_id, file_hash, mut port }) => {
                    println!("in game, randomizer version: {rando_version}, world {player_id}, hash: {file_hash:?}");
                    let mut test_item_sent = false;
                    let mut tx_buf = [0; 16];
                    tx_buf[0] = 0x01; // Player Data
                    tx_buf[1] = 2; // world number
                    tx_buf[2] = 0xba; // P
                    tx_buf[3] = 0x02; // 2
                    tx_buf[4] = 0xdf; // space
                    tx_buf[5] = 0xdf; // space
                    tx_buf[6] = 0xdf; // space
                    tx_buf[7] = 0xdf; // space
                    tx_buf[8] = 0xdf; // space
                    tx_buf[9] = 0xdf; // space
                    // hardcode progressive items state as 0 for now
                    port.write_all(&tx_buf).at("send player data")?;
                    let mut rx_buf = [0; 16];
                    loop {
                        for _ in 0..5 {
                            let bytes_read = port.read(&mut rx_buf).at("read")?;
                            if bytes_read > 0 {
                                port.read_exact(&mut rx_buf[bytes_read..]).at("read rest")?;
                                match rx_buf[0] {
                                    0x00 => {} // Ping
                                    0x01 => {
                                        let player_name = array_ref![rx_buf, 1, 8];
                                        println!("{} N64: State: File Select, player name: {player_name:x?}", Utc::now().format("%Y-%m-%d %H:%M:%S"));
                                    }
                                    0x02 => {
                                        let item_count = u16::from_be_bytes(*array_ref![rx_buf, 1, 2]);
                                        println!("{} N64: State: In Game, item count: {item_count}", Utc::now().format("%Y-%m-%d %H:%M:%S"));
                                        if !test_item_sent {
                                            tx_buf = [0; 16];
                                            tx_buf[0] = 0x02; // Get Item
                                            tx_buf[1] = 0x00; // item kind hi
                                            tx_buf[2] = 0x0d; // item kind lo
                                            port.write_all(&tx_buf).at("send item")?;
                                            test_item_sent = true;
                                        }
                                    }
                                    0x03 => {
                                        let override_key = array_ref![rx_buf, 1, 8];
                                        let item_kind = u16::from_be_bytes(*array_ref![rx_buf, 9, 2]);
                                        let target_world = rx_buf[11];
                                        println!("{} N64: Send Item {item_kind:#06x} from location {override_key:x?} to world {target_world}", Utc::now().format("%Y-%m-%d %H:%M:%S"));
                                    }
                                    0x04 => println!("{} N64: Item Received", Utc::now().format("%Y-%m-%d %H:%M:%S")),
                                    _ => println!("{} N64: {rx_buf:?}", Utc::now().format("%Y-%m-%d %H:%M:%S")),
                                }
                            } else {
                                println!("nothing read");
                            }
                            sleep(Duration::from_secs(1));
                        }
                        port.write_all(b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").at("send ping")?;
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
        sleep(Duration::from_secs(1));
        continue
    }
}
