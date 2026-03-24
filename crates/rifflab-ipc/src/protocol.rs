use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

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
    #[error("Frame too large: {0} bytes")]
    FrameTooLarge(u32),
}

/// Maximum frame payload size (16 MiB). Guards against corrupt length prefixes.
const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

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

/// Write a length-prefixed MessagePack request to a UnixStream.
pub fn write_request(stream: &mut UnixStream, req: &WorkerRequest) -> Result<(), ProtocolError> {
    let frame = encode_request(req)?;
    stream.write_all(&frame)?;
    stream.flush()?;
    Ok(())
}

/// Read a length-prefixed MessagePack response from a UnixStream (blocking).
pub fn read_response(stream: &mut UnixStream) -> Result<WorkerResponse, ProtocolError> {
    // Read the 4-byte big-endian length prefix.
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);

    if len > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge(len));
    }

    // Read the payload.
    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload)?;

    decode_response(&payload)
}
