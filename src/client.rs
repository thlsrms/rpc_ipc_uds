use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;

use rpc_ipc::{Input, RpcRequest, RpcResponse, Serializer as _};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_addr = "/tmp/rpc.sock";
    let mut socket = UnixStream::connect(socket_addr)?;

    // Prepare the RPC request
    let request = RpcRequest {
        method: "rpc_method1".to_string(),
        input: Input {
            data: vec![1, 2, 3],
        },
    }
    .encode()?;

    // Send the request
    socket.write_all(&request)?;

    // Read the response
    let mut buffer = vec![0u8; 1024];
    let n = socket.read(&mut buffer)?;
    let (response, _) = RpcResponse::decode(&buffer[..n])?;

    if let Some(output) = response.output {
        println!("Received output: {:?}", output);
    } else if let Some(error) = response.error {
        println!("Error: {:?}", error);
    }

    Ok(())
}
