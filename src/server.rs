use rpc_ipc::{Input, Output, RpcRequest, RpcResponse, Serializer, Service};
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::Mutex;

#[derive(Clone)]
struct TestServer;

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

        tokio::spawn(async move {
            let mut buffer = vec![0u8; 1024];
            let n = socket.try_read(&mut buffer).unwrap();

            if n == 0 {
                return;
            }

            let (request, _) = RpcRequest::decode(&buffer[..n]).unwrap();
            let response = handle_request(Arc::clone(&server), request).await;

            let response_data = response.encode().unwrap();
            socket.try_write(&response_data).unwrap();
        });
    }
}

async fn handle_request(server: Arc<Mutex<TestServer>>, request: RpcRequest) -> RpcResponse {
    let mut server = server.lock().await;

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
        _ => RpcResponse {
            output: None,
            error: Some("Unknown method".to_string()),
        },
    }
}
