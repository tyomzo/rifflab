use rifflab_core::ipc::{WorkerRequest, WorkerResponse};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("Deserialization error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
}

/// Encode a request into a length-prefixed MessagePack frame.
pub fn encode_request(req: &WorkerRequest) -> Result<Vec<u8>, ProtocolError> {
    let payload = rmp_serde::to_vec(req)?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode a response from a MessagePack payload (without length prefix).
pub fn decode_response(data: &[u8]) -> Result<WorkerResponse, ProtocolError> {
    Ok(rmp_serde::from_slice(data)?)
}
