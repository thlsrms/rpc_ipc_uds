use std::sync::Arc;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Mutex};

use rpc_ipc::{decode_data, encode_data, Packet, PacketRequest, PacketResponse, PacketService};
use uuid::Uuid;

#[derive(Clone)]
struct TestServer;

impl PacketService for TestServer {
    fn message(&self, client: Option<String>, message: String) -> PacketResponse {
        println!("RPC message received '{message}'");
        let res = format!("client: {} | msg: {message}", client.unwrap());
        PacketResponse {
            id: 0,
            data: Some(Packet::Message(None, res)),
            error: None,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_addr = "/tmp/rpc.sock";
    if tokio::fs::metadata(socket_addr).await.is_ok() {
        println!("A socket is already present. Deleting...");
        tokio::fs::remove_file(socket_addr).await?;
    }
    let listener = UnixListener::bind(socket_addr)?;
    let server = Arc::new(Mutex::new(TestServer));

    loop {
        let (socket, _) = listener.accept().await?;
        let server = server.clone();

        tokio::spawn(handle_connection(socket, server.clone()));
    }
}

async fn handle_connection(socket: UnixStream, server: Arc<Mutex<TestServer>>) {
    let (mut reader, mut writer) = socket.into_split();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(100);
    let client_id = Uuid::new_v4();

    // Task to handle incoming client requests
    let server_handle = server.clone();
    let reader_task = tokio::spawn(async move {
        let mut buffer = vec![0u8; 1024];
        while let Ok(n) = reader.read(&mut buffer).await {
            if n == 0 {
                break;
            }

            println!("Read {n} bytes: {:?}", &buffer[..n]);
            let response = handle_request(server_handle.clone(), &buffer[..n], client_id).await;
            if tx.send(encode_data(&response).unwrap()).await.is_err() {
                break;
            }
        }
    });

    // Task to handle sending messages to the client
    let writer_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if writer.write(&message).await.is_err() {
                break;
            }
        }
    });

    let _ = tokio::join!(reader_task, writer_task);
}

async fn handle_request(
    server: Arc<Mutex<TestServer>>,
    req_buf: &[u8],
    client: Uuid,
) -> PacketResponse {
    let server = server.lock().await;
    let Ok((req, _)) = decode_data::<PacketRequest>(req_buf) else {
        println!("Error decoding request?");
        return PacketResponse {
            id: 0,
            data: None,
            error: Some("Request invalid".into()),
        };
    };

    let mut response = match req.data {
        Packet::Message(_, msg) => server.message(Some(client.to_string()), msg),
    };
    response.id = req.id;
    response
}
