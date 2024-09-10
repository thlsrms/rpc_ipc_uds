use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::Sender;
use tokio::sync::{mpsc, Mutex};

use rpc_ipc::{decode_data, encode_data, Packet, PacketResponse, PacketService};
use uuid::Uuid;

#[derive(Clone)]
struct TestServer {
    clients: Arc<Mutex<HashMap<Uuid, Sender<Vec<u8>>>>>,
}

impl PacketService for TestServer {
    fn message(&self, id: u32, client: Option<String>, message: String) -> PacketResponse {
        println!("RPC message received '{message}'");
        let res = format!("client: {} | msg: {message}", client.unwrap());
        PacketResponse {
            id,
            data: Some(Packet::Message(id, None, res)),
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
    let server = Arc::new(Mutex::new(TestServer {
        clients: Arc::new(Mutex::new(HashMap::new())),
    }));

    loop {
        let (socket, _) = listener.accept().await?;
        let server = server.clone();

        tokio::spawn(handle_connection(socket, server.clone()));
    }
}

async fn handle_connection(socket: UnixStream, server: Arc<Mutex<TestServer>>) {
    let (mut reader, mut writer) = socket.into_split();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(100);
    let id = Uuid::new_v4();
    server.lock().await.clients.lock().await.insert(id, tx);

    // Task to handle incoming client requests
    let server_handle = server.clone();
    let reader_task = tokio::spawn(async move {
        let mut buffer = vec![0u8; 1024];
        while let Ok(n) = reader.read(&mut buffer).await {
            if n == 0 {
                break;
            }

            let (request, _) = decode_data(&buffer[..n]).unwrap();
            let response = handle_request(server_handle.clone(), request, id).await;

            // For single client response
            // if tx.send(response.encode().unwrap()).await.is_err() {
            //     break;
            // }
            for tx in server_handle.lock().await.clients.lock().await.values() {
                let _ = tx.send(encode_data(&response).unwrap()).await;
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

    // Wait for both tasks to complete
    let _ = tokio::join!(reader_task, writer_task);

    // Remove the client from the list when done
    server.lock().await.clients.lock().await.remove(&id);
}

async fn handle_request(
    server: Arc<Mutex<TestServer>>,
    request: Packet,
    client: Uuid,
) -> PacketResponse {
    let server = server.lock().await;

    match request {
        Packet::Message(id, _, msg) => server.message(id, Some(client.to_string()), msg),
    }
}
