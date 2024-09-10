use std::collections::HashSet;
use std::io::{ErrorKind, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

use rpc_ipc::{decode_data, encode_data, Packet, PacketClient, PacketRequest, PacketResponse};

// Counter for generating new request IDs
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

// Set of released IDs available for reuse
static RELEASED_IDS: LazyLock<Mutex<HashSet<u32>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

fn release_id(id: u32) {
    // Add the ID to the released set so it can be reused
    RELEASED_IDS.lock().unwrap().insert(id);
}

struct Client {
    sender: Sender<PacketRequest>,
    receiver: Arc<Mutex<Receiver<PacketResponse>>>,
    _socket: Arc<Mutex<UnixStream>>,
}

impl PacketClient for Client {
    fn send_request(&self, request: PacketRequest) -> Result<(), Box<dyn std::error::Error>> {
        self.sender.send(request)?;
        Ok(())
    }

    fn poll_responses(&self) {
        while let Ok(response) = self.receiver.lock().unwrap().recv() {
            // Do somethig with the response, call a function maybe...
            if let Some(packet) = response.data {
                match packet {
                    Packet::Message(_, msg) => println!("Received response: {msg:?}"),
                };
            }
            release_id(response.id);
        }
    }

    fn generate_request_id(&self) -> u32 {
        // Check if there are any released IDs to reuse
        let Ok(mut released_ids) = RELEASED_IDS.lock() else {
            return NEXT_ID.fetch_add(1, Ordering::SeqCst);
        };
        if let Some(&id) = released_ids.iter().next() {
            released_ids.remove(&id); // Remove it from the set once used
            return id;
        }

        // Otherwise, generate a new ID
        NEXT_ID.fetch_add(1, Ordering::SeqCst)
    }
}

impl Client {
    fn new(socket_addr: &str) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let socket = UnixStream::connect(socket_addr)?;
        socket.set_nonblocking(true).unwrap();
        let socket = Arc::new(Mutex::new(socket));

        let (request_tx, request_rx) = mpsc::channel::<PacketRequest>();
        let (response_tx, response_rx) = mpsc::channel::<PacketResponse>();

        // task to handle reading responses from the server
        {
            let socket_handler = Arc::clone(&socket);
            thread::spawn(move || {
                let mut buffer = vec![0u8; 64 * 8];

                loop {
                    let mut socket = match socket_handler.lock() {
                        Ok(lock) => lock,
                        Err(_) => continue,
                    };
                    match socket.read(&mut buffer) {
                        Ok(n) if n > 0 && n <= buffer.len() => {
                            println!("Read {n} bytes");
                            drop(socket); // Release the lock before processing the message
                            if let Ok((response, _)) = decode_data(&buffer[..n]) {
                                if response_tx.send(response).is_err() {
                                    println!("Could no send a response over the channel");
                                };
                            } else {
                                println!("Response decoding failed");
                            }
                        }
                        Ok(_) => {
                            println!("Connection closed by the server");
                            break;
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {
                            // Handle timeout
                            std::thread::sleep(std::time::Duration::from_micros(333));
                            continue; // Continue the loop after timeout
                        }
                        Err(e) => {
                            println!("Error reading from socket: {}", e);
                            break;
                        }
                    }
                }
            });
        }

        // task to handle sending requests to the server
        {
            let socket_handler = Arc::clone(&socket);
            thread::spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let request = encode_data(&request).unwrap();
                    let mut socket = match socket_handler.lock() {
                        Ok(lock) => lock,
                        Err(_) => continue,
                    };
                    if socket.write_all(&request).is_err() {
                        println!("Error writing into the socket");
                        break;
                    }
                }
            });
        }

        Ok(Arc::new(Self {
            sender: request_tx,
            receiver: Arc::new(Mutex::new(response_rx)),
            _socket: socket,
        }))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_addr = "/tmp/rpc.sock";
    let client = Client::new(socket_addr)?;

    // Continuously poll for responses
    {
        let client_handle = Arc::clone(&client);
        thread::spawn(move || loop {
            client_handle.poll_responses();
            std::thread::sleep(std::time::Duration::from_millis(10));
        });
    }

    // Client sending messages
    loop {
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .expect("Read Line failed");
        input = input[0..input.len() - 1].to_string();

        if !input.is_empty() {
            client.message(None, input)?;
        }
    }

    unreachable!("The input loop above hogs the thread");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
    Ok(())
}
