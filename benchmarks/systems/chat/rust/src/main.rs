//! Chat broadcast server, Rust: tokio, a task per connection.
//!
//! Protocol (identical in all three servers): "J <channel>\n" first, answered
//! with "OK\n"; every later line is broadcast verbatim to the channel's other
//! members. Per read, everything up to the last complete newline goes out as
//! one write per peer; a partial line waits for the rest.
//!
//! Each client has a reader task and a writer task fed by an unbounded mpsc
//! queue, so a slow receiver delays only itself and nothing is dropped (the
//! benchmark checks every delivery). `tokio::sync::broadcast` is not used
//! because a lagging receiver loses messages.
//!
//! Usage: chat-server [port]   (default 9000; prints "listening <port>")

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

type Msg = Arc<[u8]>;
type Room = RwLock<HashMap<u64, mpsc::UnboundedSender<Msg>>>;

#[derive(Default)]
struct Server {
    rooms: Mutex<HashMap<u64, Arc<Room>>>,
    next_id: AtomicU64,
}

impl Server {
    fn room(&self, id: u64) -> Arc<Room> {
        self.rooms.lock().unwrap().entry(id).or_default().clone()
    }
}

async fn serve(server: Arc<Server>, stream: TcpStream) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let (rd, mut wr) = stream.into_split();
    let mut rd = BufReader::with_capacity(4096, rd);
    let mut line = String::new();
    if rd.read_line(&mut line).await? == 0 {
        return Ok(());
    }
    let channel: u64 = line
        .split_whitespace()
        .nth(1)
        .and_then(|w| w.parse().ok())
        .unwrap_or(0);

    let id = server.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let room = server.room(channel);
    room.write().unwrap().insert(id, tx);

    wr.write_all(b"OK\n").await?;
    let writer = tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            if wr.write_all(&first).await.is_err() {
                return;
            }
            // Whatever else is already queued goes out before waiting again.
            while let Ok(more) = rx.try_recv() {
                if wr.write_all(&more).await.is_err() {
                    return;
                }
            }
        }
    });

    // Bytes that arrived with the join line are still in the BufReader.
    let mut pending: Vec<u8> = rd.buffer().to_vec();
    let mut rd = rd.into_inner();
    let mut buf = vec![0u8; 64 * 1024];
    let result = loop {
        if let Some(last) = pending.iter().rposition(|&b| b == b'\n') {
            let out: Msg = Arc::from(&pending[..=last]);
            pending.drain(..=last);
            let members = room.read().unwrap();
            for (&peer, tx) in members.iter() {
                if peer != id {
                    let _ = tx.send(out.clone());
                }
            }
        }
        match rd.read(&mut buf).await {
            Ok(0) => break Ok(()),
            Ok(n) => pending.extend_from_slice(&buf[..n]),
            Err(e) => break Err(e),
        }
    };
    room.write().unwrap().remove(&id);
    writer.abort();
    result
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(9000);
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    println!("listening {}", listener.local_addr()?.port());
    let server = Arc::new(Server::default());
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let server = server.clone();
        tokio::spawn(async move {
            let _ = serve(server, stream).await;
        });
    }
}
