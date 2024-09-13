use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use rpc_ipc::{
    decode_data, encode_data, Message, Origin, Payload, Req, Res, TestRPCError, TestRPCErrorKind,
    TestRPCService,
};

struct TestServer {
    clients: HashMap<Uuid, Arc<Mutex<ClientConnection>>>,
    client_abort_channels: HashMap<Uuid, mpsc::Sender<()>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_addr = "/tmp/rpc.sock";
    if tokio::fs::metadata(socket_addr).await.is_ok() {
        println!("A socket is already present. Deleting...");
        tokio::fs::remove_file(socket_addr).await?;
    }

    let listener = UnixListener::bind(socket_addr)?;
    let server = Arc::new(Mutex::new(TestServer {
        clients: HashMap::new(),
        client_abort_channels: HashMap::new(),
    }));

    {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let client_id = Uuid::new_v4();
                let (client, (inbound_tx, outbound_rx)) = ClientConnection::new(client_id);
                let mut server_lock = server.lock().await;
                server_lock.clients.insert(client_id, Arc::clone(&client));

                let (abort_tx, abort_rx) = mpsc::channel::<()>(1);

                println!("Client connected {client_id}");

                let server_m = Arc::clone(&server);
                tokio::spawn(async move {
                    ClientConnection::open(
                        Arc::clone(&client),
                        socket,
                        inbound_tx,
                        outbound_rx,
                        abort_rx,
                    )
                    .await;

                    println!("Connection cleanup x {{ {client_id} }}");
                    server_m.lock().await.clients.remove(&client_id);
                });
                server_lock
                    .client_abort_channels
                    .insert(client_id, abort_tx);
            }
        });
    }

    loop {
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .expect("Read line failed");
        input = input[0..input.len() - 1].to_string();

        if !input.is_empty() {
            match input.split_once(" ") {
                Some((command, arg)) => match command {
                    "kill" => {
                        let id = match Uuid::from_str(arg) {
                            Ok(uuid) => uuid,
                            Err(e) => {
                                println!("Kill command failed {e}");
                                continue;
                            }
                        };

                        if let Some(abort) = server.lock().await.client_abort_channels.remove(&id) {
                            abort.send(()).await.unwrap();
                            println!("Closing client {{ {id} }} connection");
                        }
                    }
                    _ => continue,
                },
                None => match input.as_ref() {
                    "list" => {
                        for key in server.lock().await.clients.keys() {
                            println!("Client : {key}");
                        }
                    }
                    _ => continue,
                },
            }
        }
    }
}

struct ClientConnection {
    id: Uuid,
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
}

impl ClientConnection {
    fn new(id: Uuid) -> (Arc<Mutex<Self>>, (Sender<Message>, Receiver<Message>)) {
        let (server_bound_tx, server_bound_rx) = mpsc::channel::<Message>(100);
        let (client_bound_tx, client_bound_rx) = mpsc::channel::<Message>(100);

        (
            Arc::new(Mutex::new(Self {
                id,
                sender: client_bound_tx,
                receiver: server_bound_rx,
                // tasks_hadle: vec![],
            })),
            (server_bound_tx, client_bound_rx),
        )
    }

    async fn read_messages(&mut self) {
        while let Some(mut message) = self.receiver.recv().await {
            match message.payload {
                Payload::Request(req) => {
                    message.payload = match req {
                        Req::Message(_, msg) => Payload::Response(
                            self.on_request_payload(Req::Message(None, msg)).await,
                        ),
                        _ => Payload::Response(self.on_request_payload(req).await),
                    };
                    if self.sender.send(message).await.is_err() {
                        break;
                    };
                }
                Payload::Response(res) => self.on_response_payload(res).await,
            }
        }
    }

    async fn open(
        this: Arc<Mutex<Self>>,
        socket: UnixStream,
        inbound_message: mpsc::Sender<Message>,
        mut outbound_message: mpsc::Receiver<Message>,
        mut abort_rx: mpsc::Receiver<()>,
    ) {
        let (mut reader, mut writer) = socket.into_split();

        let outbound_sender = this.lock().await.sender.clone();
        let outbound_sende_2 = outbound_sender.clone();
        tokio::select! {
            _ = abort_rx.recv() => {println!("Connection Closed")},
            _read_messages = async move {
                let mut buffer = vec![0u8; 1024];
                while let Ok(n) = reader.read(&mut buffer).await {
                    if n == 0 {
                        break;
                    }

                    println!("Read {n} bytes: {:?}", &buffer[..n]);

                    let Ok((message, _)) = decode_data::<Message>(&buffer[..n]) else {
                        println!("Error decoding message");
                        if outbound_sende_2
                            .send(Message {
                                id: 0,
                                origin: Origin::Client,
                                payload: Payload::Response(Res::Error(TestRPCError::new(
                                    TestRPCErrorKind::InvalidArgument,
                                ))),
                            })
                            .await
                            .is_err()
                        {
                            break;
                        };
                        continue;
                    };
                    if inbound_message.send(message).await.is_err() {
                        break;
                    }
                }
            } => {},

            _ping = async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

                    if outbound_sender
                        .send(Message {
                            id: 0,
                            origin: Origin::Service,
                            payload: Payload::Request(Req::Ping(
                                SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap()
                                    .as_micros(),
                            )),
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }=> {},

            _socket_writer = async move {
                while let Some(message) = outbound_message.recv().await {
                    let bytes = match encode_data(&message) {
                        Ok(b) => b,
                        Err(e) => {
                            println!("Error encoding message: {e}");
                            continue;
                        }
                    };
                    if writer.write(&bytes).await.is_err() {
                        break;
                    }
                }
            } => {},


            _socket_reader = async move {
                // tokio::time::sleep(tokio::time::Duration::from_micros(1)).await;
                this.lock().await.read_messages().await;
            } => {},
        }
    }
}

impl TestRPCService for ClientConnection {
    async fn send_client_message(
        &self,
        message: Message,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.sender.send(message).await?;
        Ok(())
    }

    async fn on_request_payload(&mut self, payload: Req) -> Res {
        match payload {
            Req::Message(client, msg) => self.handle_message_request(client, msg).await,
            Req::Sum(a, b) => self.handle_sum_request(a, b).await,
            Req::Multiply(a, b) => self.handle_multiply_request(a, b).await,
            Req::Divide(a, b) => self.handle_divide_request(a, b).await,
            _ => Res::Error(TestRPCError::new(TestRPCErrorKind::Unreachable)),
        }
    }

    async fn on_response_payload(&mut self, payload: Res) {
        match payload {
            Res::Ping(t) => self.on_ping_response(t).await,
            _ => {
                println!("Unexpected response received")
            }
        };
    }

    async fn handle_message_request(&self, _client: Option<String>, msg: String) -> Res {
        println!("RPC message received '{msg}'");
        let res = format!("client: {} | msg: {msg}", self.id);
        Res::Message(res)
    }

    async fn handle_sum_request(&self, arg0: f32, arg1: f32) -> Res {
        println!("Sum {arg0} + {arg1}");
        Res::Sum(arg0 + arg1)
    }

    async fn handle_multiply_request(&self, arg0: f32, arg1: f32) -> Res {
        println!("Multiply {arg0} * {arg1}");
        Res::Multiply(arg0 * arg1)
    }

    async fn handle_divide_request(&self, arg0: f32, arg1: f32) -> Res {
        println!("Divide {arg0} / {arg1}");
        if arg0 == 0.0 || arg1 == 0.0 {
            return Res::Error(TestRPCError::new(TestRPCErrorKind::InvalidArgument));
        }
        Res::Divide(arg0 / arg1)
    }

    async fn on_ping_response(&self, timestamp: u128) {
        println!(
            "Pong micros {}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_micros()
                - timestamp
        );
    }
}
