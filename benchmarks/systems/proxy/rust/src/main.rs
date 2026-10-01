//! HTTP/1.1 reverse proxy subset: the Rust entry of benchmarks/systems/proxy.
//!
//!   proxy [listen_port] [upstream_port]        (defaults 9301, 9300)
//!
//! Idiomatic tokio: one task per client connection, each owning a keep-alive
//! upstream connection. The request is parsed in place in the read buffer and
//! forwarded with one vectored write of three borrowed slices (head, the
//! X-Forwarded-For bytes, the rest including the body), so it is not copied.
//! Response chunks are written to the client straight out of the read buffer;
//! the response head is scanned in place unless it straddles two reads.

use std::io::IoSlice;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct Head {
    /// Index of the CR of the blank line.
    end: usize,
    /// Content-Length (0 when absent).
    length: usize,
    /// Index of the CR ending an X-Forwarded-For line.
    xff_end: Option<usize>,
}

fn head_end(b: &[u8]) -> Option<usize> {
    b.windows(4).position(|w| w == b"\r\n\r\n")
}

fn name_is(line: &[u8], name: &[u8]) -> bool {
    line.len() > name.len() && line[name.len()] == b':' && line[..name.len()].eq_ignore_ascii_case(name)
}

fn number(b: &[u8]) -> Option<usize> {
    let s = b.iter().position(|&c| c != b' ' && c != b'\t').unwrap_or(b.len());
    let digits = &b[s..];
    let n = digits.iter().take_while(|c| c.is_ascii_digit()).count();
    if n == 0 {
        return None;
    }
    digits[..n].iter().try_fold(0usize, |v, &c| v.checked_mul(10)?.checked_add((c - b'0') as usize))
}

/// `Ok(None)` when the head is incomplete, `Err` when it is unreadable.
fn scan_head(b: &[u8]) -> Result<Option<Head>, ()> {
    let Some(end) = head_end(b) else { return Ok(None) };
    let mut length = 0;
    let mut xff_end = None;
    let mut ls = b.iter().position(|&c| c == b'\r').unwrap() + 2;
    while ls < end + 2 {
        let le = ls + b[ls..].iter().position(|&c| c == b'\r').unwrap();
        let line = &b[ls..le];
        if name_is(line, b"content-length") {
            length = number(&line[15..]).ok_or(())?;
        } else if name_is(line, b"x-forwarded-for") {
            xff_end = Some(le);
        }
        ls = le + 2;
    }
    Ok(Some(Head { end, length, xff_end }))
}

async fn write_all_vectored(s: &mut TcpStream, parts: &[&[u8]]) -> std::io::Result<()> {
    let mut idx = 0;
    let mut off = 0;
    while idx < parts.len() {
        let mut slices: [IoSlice; 3] = [IoSlice::new(&[]), IoSlice::new(&[]), IoSlice::new(&[])];
        let mut k = 0;
        for (j, p) in parts[idx..].iter().enumerate() {
            slices[k] = IoSlice::new(if j == 0 { &p[off..] } else { p });
            k += 1;
        }
        let mut n = s.write_vectored(&slices[..k]).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        while idx < parts.len() && n >= parts[idx].len() - off {
            n -= parts[idx].len() - off;
            idx += 1;
            off = 0;
        }
        off += n;
    }
    Ok(())
}

async fn serve(mut client: TcpStream, peer: SocketAddr, upstream_addr: SocketAddr) -> std::io::Result<()> {
    client.set_nodelay(true)?;
    let mut upstream = TcpStream::connect(upstream_addr).await?;
    upstream.set_nodelay(true)?;
    let ip = peer.ip().to_string();
    let xff_new = format!("X-Forwarded-For: {ip}\r\n").into_bytes();
    let xff_add = format!(", {ip}").into_bytes();

    let mut req: Vec<u8> = Vec::with_capacity(8192);
    let mut resp = vec![0u8; 65536];
    let mut split_head: Vec<u8> = Vec::new();

    loop {
        // A complete request in `req`.
        let head = loop {
            match scan_head(&req) {
                Err(()) => return Ok(()),
                Ok(Some(h)) => break h,
                Ok(None) => {}
            }
            if req.len() > 65536 {
                return Ok(());
            }
            req.reserve(4096);
            if client.read_buf(&mut req).await? == 0 {
                return Ok(());
            }
        };
        let total = head.end + 4 + head.length;
        while req.len() < total {
            req.reserve(total - req.len());
            if client.read_buf(&mut req).await? == 0 {
                return Ok(());
            }
        }
        let (at, mid): (usize, &[u8]) = match head.xff_end {
            Some(le) => (le, &xff_add),
            None => (head.end + 2, &xff_new),
        };
        write_all_vectored(&mut upstream, &[&req[..at], mid, &req[at..total]]).await?;
        req.drain(..total);

        // Relay the response.
        let mut left: Option<usize> = None;
        split_head.clear();
        loop {
            let n = upstream.read(&mut resp).await?;
            if n == 0 {
                return Ok(());
            }
            client.write_all(&resp[..n]).await?;
            match left {
                Some(l) => {
                    if n > l {
                        return Ok(()); // out of step
                    }
                    left = Some(l - n);
                }
                None => {
                    let hb: &[u8] = if split_head.is_empty() {
                        &resp[..n]
                    } else {
                        split_head.extend_from_slice(&resp[..n]);
                        &split_head
                    };
                    match scan_head(hb) {
                        Err(()) => return Ok(()),
                        Ok(None) => {
                            if split_head.is_empty() {
                                split_head.extend_from_slice(&resp[..n]);
                            }
                        }
                        Ok(Some(h)) => {
                            let got = hb.len() - (h.end + 4);
                            if got > h.length {
                                return Ok(());
                            }
                            left = Some(h.length - got);
                        }
                    }
                }
            }
            if left == Some(0) {
                break;
            }
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(9301);
    let up_port: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(9300);
    let upstream_addr: SocketAddr = ([127, 0, 0, 1], up_port).into();
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    println!("proxy listening on 127.0.0.1:{port}, upstream 127.0.0.1:{up_port}");
    loop {
        let (client, peer) = listener.accept().await?;
        tokio::spawn(async move {
            let _ = serve(client, peer, upstream_addr).await;
        });
    }
}
