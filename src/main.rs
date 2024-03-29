use anyhow::{bail, Result};
use byteorder::{BigEndian, ByteOrder};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct KymuxAddr {
    addr: SocketAddr,
    endpoint_id: u16,
}

fn parse_kymux_url(url_str: &str) -> Result<KymuxAddr> {
    let url = url::Url::parse(&url_str)?;

    if url.scheme() != "kymux" {
        bail!("Wrong scheme in url: {url}");
    }

    let Some(host) = url.host_str() else {
        bail!("Missing host in url: {url}");
    };

    let Ok(ip) = host.parse::<IpAddr>() else {
        bail!("Invalid IP in url: {url}");
    };

    let Some(port) = url.port() else {
        bail!("Missing port in url: {url}");
    };

    if url.path().len() < 2 {
        // the first char is '/'
        bail!("Empty path in url: {url}");
    }

    let path = &url.path()[1..];
    let Ok(endpoint_id) = u16::from_str_radix(path, 0x10) else {
        bail!("Invalid endpoint: {path}");
    };

    Ok(KymuxAddr {
        addr: SocketAddr::new(ip, port),
        endpoint_id,
    })
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        bail!("Syntax error, expected: {} <file> <kymux_url>", args[0]);
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
    let mut pts_origin = None;

    let sid_and_codec_packet = [b'h', b'2', b'6', b'4', 0, 0, 0, 0, 0, 0, 0, 0];
    tcp_stream.write(&sid_and_codec_packet)?;

    loop {
        let mut header = [0; 12];
        file_reader.set_limit(12);
        if let Err(_) = file_reader.read_exact(&mut header) {
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
            if let Some(pts_origin) = pts_origin {
                let target = Duration::from_micros(pts - pts_origin);
                if target > elapsed {
                    let to_wait = target - elapsed;
                    std::thread::sleep(to_wait);
                }
            } else {
                pts_origin = Some(pts)
            }
        }

        print!("\rStreaming pts={}", pts);
        let _ = std::io::stdout().flush();

        // header format changed due to config packet
        header[0] = 0x80 | ((header[0] & 0xC0) >> 1) | (header[0] & 0x1F);

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
