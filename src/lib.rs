use bincode::{Decode, Encode};

#[derive(Encode, Decode, Debug)]
pub struct RpcRequest {
    pub method: String,
    pub input: Input,
}

#[derive(Encode, Decode, Debug)]
pub struct RpcResponse {
    pub output: Option<Output>,
    pub error: Option<String>,
}

pub trait Serializer {
    fn encode(&self) -> Result<Vec<u8>, bincode::error::EncodeError>
    where
        Self: bincode::Encode,
    {
        encode_data(self)
    }

    fn decode(buf: &[u8]) -> Result<(Self, usize), bincode::error::DecodeError>
    where
        Self: bincode::Decode,
    {
        decode_data(buf)
    }
}

impl<T> Serializer for T where T: Encode + Decode {}

pub fn encode_data(data: impl Encode) -> Result<Vec<u8>, bincode::error::EncodeError> {
    bincode::encode_to_vec(&data, bincode::config::standard())
}

pub fn decode_data<D: Decode>(buf: &[u8]) -> Result<(D, usize), bincode::error::DecodeError> {
    bincode::decode_from_slice(buf, bincode::config::standard())
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
