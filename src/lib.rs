use bincode::{Decode, Encode};

#[derive(Encode, Decode, Debug)]
pub struct RpcRequest {
    pub method: String,
    pub input: Input,
}

pub trait Serializer {
    fn encode(&self) -> Result<Vec<u8>, bincode::error::EncodeError>
    where
        Self: bincode::Encode,
    {
        bincode::encode_to_vec(self, bincode::config::standard())
    }

    fn decode(buf: &[u8]) -> Result<(Self, usize), bincode::error::DecodeError>
    where
        Self: bincode::Decode,
    {
        bincode::decode_from_slice(buf, bincode::config::standard())
    }
}

impl<T> Serializer for T where T: Encode + Decode {}

#[derive(Encode, Decode, Debug)]
pub struct RpcResponse {
    pub output: Option<Output>,
    pub error: Option<String>,
}

#[derive(Encode, Decode, Debug)]
pub struct Input {
    pub data: Vec<u8>,
}

#[derive(Encode, Decode, Debug)]
pub struct Output {
    pub data: Vec<u8>,
}

pub trait Service {
    fn rpc_method1(&mut self, input: Input) -> Output;
    fn rpc_method2(&mut self, input: Input) -> Output;
    fn rpc_method3(&mut self, forward_id: u64, forward_only: bool) -> Result<Output, ()>;
    fn rpc_method4(&mut self, input: Input, client: uuid::Uuid) -> Output;
}
