use byteorder::{BigEndian, ByteOrder};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Error, Debug)]
enum StreamerError {
    #[error("Syntax error: {msg}")]
    SyntaxError { msg: String },
    #[error("I/O error")]
    Io(#[from] std::io::Error),
}

fn main() -> Result<(), StreamerError> {
    // rtstreamer <port> <file>
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err(StreamerError::SyntaxError {
            msg: format!("Expected: {} <port> <file>", args[0]),
        });
    }

    let port = args[1]
        .parse::<u16>()
        .map_err(|e| StreamerError::SyntaxError {
            msg: format!("Could not parse port: '{}' ({})", args[1], e.to_string()),
        })?;

    let mut file_reader = {
        let filepath = &args[2];
        let file = File::open(filepath)?;
        BufReader::new(file).take(0)
    };

    let mut tcp_stream = {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))?;
        let (tcp_stream, addr) = listener.accept()?;
        println!("Connection accepted from {}", addr);
        tcp_stream
    };

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

    let start = Instant::now();

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
