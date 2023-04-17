use byteorder::{BigEndian, ByteOrder};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Error, Debug)]
enum StreamerError {
    #[error("Syntax error: {msg}")]
    SyntaxError { msg: String },
    #[error("Invalid url {url}: {msg}")]
    InvalidUrl { url: String, msg: String },
    #[error("I/O error")]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
struct KymuxAddr {
    addr: SocketAddr,
    endpoint_id: u64,
}

fn parse_kymux_url(url_str: &str) -> Result<KymuxAddr, StreamerError> {
    let url = url::Url::parse(&url_str).map_err(|e| StreamerError::InvalidUrl { url: url_str.to_string(), msg: e.to_string() })?;

    if url.scheme() != "kymux" {
        return Err(StreamerError::InvalidUrl { url: url_str.to_string(), msg: "Wrong scheme".to_string() });
    }

    let Some(host) = url.host_str() else {
        return Err(StreamerError::InvalidUrl { url: url_str.to_string(), msg: "Missing host".to_string() });
    };

    let Ok(ip) = host.parse::<IpAddr>() else {
        return Err(StreamerError::InvalidUrl { url: url_str.to_string(), msg: "Invalid ip".to_string() });
    };

    let Some(port) = url.port() else {
        return Err(StreamerError::InvalidUrl { url: url_str.to_string(), msg: "Missing port".to_string() });
    };

    if url.path().len() < 2 {
        // the first char is '/'
        return Err(StreamerError::InvalidUrl { url: url_str.to_string(), msg: "Empty path".to_string() });
    }

    let path = &url.path()[1..];
    let Ok(endpoint_id) = u64::from_str_radix(path, 0x10) else {
        return Err(StreamerError::InvalidUrl { url: url_str.to_string(), msg: format!("Invalid endpoint: {}", path) });
    };

    Ok(KymuxAddr {
        addr: SocketAddr::new(ip, port),
        endpoint_id,
    })
}

fn main() -> Result<(), StreamerError> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err(StreamerError::SyntaxError {
            msg: format!("Expected: {} <file> <kymux_url>", args[0]),
        });
    }

    let mut file_reader = {
        let filepath = &args[1];
        let file = File::open(filepath)?;
        BufReader::new(file).take(0)
    };

    let kymux_addr = parse_kymux_url(&args[2])?;

    let mut tcp_stream = TcpStream::connect(kymux_addr.addr)?;

    // The "meta" header length is 12 bytes:
    // [. . . . . . . .|. . . .]. . . . . . . . . . . . . . . ...
    //  <-------------> <-----> <-----------------------------...
    //        PTS        packet        raw packet
    //                    size
    //
    // It is followed by <packet_size> bytes containing the packet/frame.
    //
    // The most significant bits of the PTS are used for packet flags:
    //
    //  byte 7   byte 6   byte 5   byte 4   byte 3   byte 2   byte 1   byte 0
    // CK...... ........ ........ ........ ........ ........ ........ ........
    // ^^<------------------------------------------------------------------->
    // ||                                PTS
    // | `- key frame
    //  `-- config packet

    tcp_stream.write(&kymux_addr.endpoint_id.to_be_bytes())?;

    tcp_stream.read(&mut [0u8])?; // sync byte

    let start = Instant::now();

    let sid_and_codec_packet = [0, 0, 0, 0, b'h', b'2', b'6', b'4', 0, 0, 0, 0, 0, 0, 0, 0];
    tcp_stream.write(&sid_and_codec_packet)?;

    loop {
        let mut header = [0; 12];
        file_reader.set_limit(12);
        let r = file_reader.read(&mut header)?;
        if r < 12 {
            // EOF
            break;
        }

        let pts_and_flags = BigEndian::read_u64(&header[..8]);
        let pts = pts_and_flags & 0x3F_FF_FF_FF_FF_FF_FF_FF;
        let is_config = pts_and_flags & 0x80_00_00_00_00_00_00_00 != 0;
        let size = BigEndian::read_u32(&header[8..12]);

        if !is_config {
            // wait until PTS
            let now = Instant::now();
            let elapsed = now.duration_since(start);
            let target = Duration::from_micros(pts);
            if target > elapsed {
                let to_wait = target - elapsed;
                std::thread::sleep(to_wait);
            }
        }

        print!("\rStreaming pts={}", pts);
        let _ = std::io::stdout().flush();

        // header format changed due to config packet
        header[0] = 0x80 | ((header[0] & 0xC0) >> 1) | (header[0] & 0x1F);

        tcp_stream.write(&[0u8; 4])?;
        tcp_stream.write(&header)?;

        file_reader.set_limit(size as u64);
        let r = std::io::copy(&mut file_reader, &mut tcp_stream)?;
        if r < size as u64 {
            // EOF
            break;
        }
    }
    println!("\rComplete");
    Ok(())
}
