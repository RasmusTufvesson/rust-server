use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex};

use std::collections::HashMap;
use std::{env};
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{io, str};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let state = Arc::new(Mutex::new(Shared::new()));

    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "0.0.0.0:6666".to_string());

    // Bind a TCP listener to the socket address.
    //
    // Note that this is the Tokio TcpListener, which is fully async.
    let listener = TcpListener::bind(&addr).await?;

    println!("server running on {}", addr);

    loop {
        // Asynchronously wait for an inbound TcpStream.
        let (stream, addr) = listener.accept().await?;

        // Clone a handle to the `Shared` state for the new connection.
        let state = Arc::clone(&state);

        // Spawn our handler to be run asynchronously.
        tokio::spawn(async move {
            println!("accepted connection");
            if let Err(e) = process(state, stream, addr).await {
                println!("an error occurred; error = {:?}", e);
            }
        });
    }
}

type Tx = mpsc::UnboundedSender<String>;

type Rx = mpsc::UnboundedReceiver<String>;

struct Shared {
    peers: HashMap<SocketAddr, Tx>,
}

struct Peer {
    rx: Rx,
}

impl Shared {
    fn new() -> Self {
        Shared {
            peers: HashMap::new(),
        }
    }

    async fn broadcast(&mut self, sender: SocketAddr, message: &str) {
        for peer in self.peers.iter_mut() {
            if *peer.0 != sender {
                let _ = peer.1.send(message.into());
            }
        }
    }
}

impl Peer {
    async fn new(
        state: Arc<Mutex<Shared>>,
        addr: SocketAddr,
    ) -> io::Result<Peer> {
        let (tx, rx) = mpsc::unbounded_channel();

        state.lock().await.peers.insert(addr, tx);

        Ok(Peer { rx })
    }
}

struct Connection {
    stream: TcpStream,
    buf: [u8; 1024],
}

impl Connection {
    pub fn new(stream: TcpStream) -> Self {
        Self { stream, buf: [0; 1024] }
    }

    pub async fn recv(&mut self) -> Option<[u8; 1024]> {
        let val = match self.stream.read(&mut self.buf).await {
            Ok(_) => {
                Some(self.buf.clone())
            }
            _ => {
                None
            }
        };
        self.buf.fill(0);
        val
    }

    pub async fn recv_str(&mut self) -> Option<String> {
        let val = match self.stream.read(&mut self.buf).await {
            Ok(_) => {
                match String::from_utf8(self.buf.to_vec()) {
                    Ok(string) => {
                        let string = string.replace("\0", "");
                        if string.len() > 0 {
                            Some(string)
                        } else {
                            None
                        }
                    }
                    _ => {
                        None
                    }
                }
            }
            _ => {
                None
            }
        };
        self.buf.fill(0);
        val
    }

    pub async fn send(&mut self, data: &[u8]) {
        self.stream
            .write_all(data)
            .await
            .expect("failed to write data to socket");
    }

    pub async fn send_str(&mut self, data: String) {
        let data = (data).as_bytes();
        self.send(data).await;
    }
}

async fn process(
    state: Arc<Mutex<Shared>>,
    stream: TcpStream,
    addr: SocketAddr,
) -> Result<(), Box<dyn Error>> {

    let mut socket = Connection::new(stream);

    socket.send_str("Please enter your username:".to_string()).await;

    let username = match socket.recv_str().await {
        Some(string) => string,
        _ => {
            println!("Failed to get username from {}. Client disconnected.", addr);
            return Ok(());
        }
    };

    let mut peer = Peer::new(state.clone(), addr).await?;

    {
        let mut state = state.lock().await;
        let msg = format!("{} has joined the chat", username);
        println!("{}", msg);
        state.broadcast(addr, &msg).await;
    }

    loop {
        tokio::select! {
            Some(msg) = peer.rx.recv() => {
                socket.send_str(msg).await;
            }
            result = socket.recv_str() => match result {
                Some(msg) => {
                    let mut state = state.lock().await;
                    let msg = format!("{}: {}", username, msg);
                    println!("{}, {}", msg, msg.len());
                    state.broadcast(addr, &msg).await;
                }
                None => break,
            },
        }
    }

    {
        let mut state = state.lock().await;
        state.peers.remove(&addr);

        let msg = format!("{} has left the chat", username);
        println!("{}", msg);
        state.broadcast(addr, &msg).await;
    }

    Ok(())
}