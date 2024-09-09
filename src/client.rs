use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::thread;

use rpc_ipc::{Input, RpcRequest, RpcResponse, Serializer as _};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_addr = "/tmp/rpc.sock";
    let mut socket = UnixStream::connect(socket_addr)?;

    // Start a thread to listen for server messages
    let mut socket_handle = socket.try_clone()?;
    thread::spawn(move || {
        let mut buffer = vec![0u8; 1024];
        loop {
            let n = socket_handle.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            let (response, _) = RpcResponse::decode(&buffer[..n]).unwrap();
            if let Some(output) = response.output {
                let (msg, _): (String, usize) =
                    bincode::decode_from_slice(&output.data, bincode::config::standard()).unwrap();

                println!("Received response: {msg:?}");
            }
        }
    });

    // Client sending requests

    loop {
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .expect("Read Line failed");
        input = input[0..input.len() - 1].to_string();

        let request = RpcRequest {
            method: "rpc_method4".into(),
            input: Input {
                data: bincode::encode_to_vec(&input, bincode::config::standard()).unwrap(),
            },
        }
        .encode()?;

        // Send the request
        socket.write_all(&request)?;
    }

    unreachable!("The input loop above hogs the thread");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
    Ok(())
}
