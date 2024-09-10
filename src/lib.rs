use bincode::{Decode, Encode};
pub use rpc_ipc_macros::{RpcClient, RpcService};

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

#[derive(RpcService, RpcClient, Encode, Decode)]
#[rpc_service(serialize = "bincode")]
pub enum Packet {
    Message(u32, Option<String>, String),
}
