use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::Sender;
use tokio::sync::{mpsc, Mutex};

use rpc_ipc::{Input, Output, RpcRequest, RpcResponse, Serializer, Service};
use uuid::Uuid;

#[derive(Clone)]
struct TestServer {
    clients: Arc<Mutex<HashMap<Uuid, Sender<Vec<u8>>>>>,
}

impl Service for TestServer {
    fn rpc_method1(&mut self, _input: Input) -> Output {
        println!("RPC method1 {_input:?}");
        Output {
            data: vec![1, 2, 3],
        }
    }

    fn rpc_method2(&mut self, _input: Input) -> Output {
        println!("RPC method2 {_input:?}");
        Output {
            data: vec![4, 5, 6],
        }
    }

    fn rpc_method3(&mut self, _forward_id: u64, _forward_only: bool) -> Result<Output, ()> {
        println!("RPC method3 {_forward_id:?}");
        Ok(Output {
            data: vec![7, 8, 9],
        })
    }

    fn rpc_method4(&mut self, input: Input, client: Uuid) -> Output {
        let (message, _): (String, _) =
            bincode::decode_from_slice(&input.data, bincode::config::standard()).unwrap();
        println!("RPC method4 message received '{message}'");
        Output {
            data: bincode::encode_to_vec(
                format!("client: {client} | msg: {message}"),
                bincode::config::standard(),
            )
            .unwrap(),
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

            let (request, _) = RpcRequest::decode(&buffer[..n]).unwrap();
            let response = handle_request(server_handle.clone(), request, id).await;

            // For single client response
            // if tx.send(response.encode().unwrap()).await.is_err() {
            //     break;
            // }
            for tx in server_handle.lock().await.clients.lock().await.values() {
                let _ = tx.send(response.encode().unwrap()).await;
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
    request: RpcRequest,
    client: Uuid,
) -> RpcResponse {
    let mut server = server.lock().await;

    // TODO: Typed Service functions + Custom Input / Output
    match request.method.as_str() {
        "rpc_method1" => {
            let output = server.rpc_method1(request.input);
            RpcResponse {
                output: Some(output),
                error: None,
            }
        }
        "rpc_method2" => {
            let output = server.rpc_method2(request.input);
            RpcResponse {
                output: Some(output),
                error: None,
            }
        }
        "rpc_method3" => match server.rpc_method3(0, false) {
            Ok(output) => RpcResponse {
                output: Some(output),
                error: None,
            },
            Err(_) => RpcResponse {
                output: None,
                error: Some("Failed".to_string()),
            },
        },
        "rpc_method4" => {
            let output = server.rpc_method4(request.input, client);
            RpcResponse {
                output: Some(output),
                error: None,
            }
        }
        _ => RpcResponse {
            output: None,
            error: Some("Unknown method".to_string()),
        },
    }
}
