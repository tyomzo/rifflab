use std::path::PathBuf;
use rifflab_core::ipc::{WorkerRequest, WorkerResponse};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("Worker not connected")]
    NotConnected,
    #[error("Worker process exited unexpectedly")]
    WorkerCrashed,
    #[error("Protocol error: {0}")]
    Protocol(#[from] crate::protocol::ProtocolError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Handle to a submitted job.
#[derive(Debug, Clone)]
pub struct JobHandle {
    pub job_id: u64,
}

/// Client for communicating with a Python worker process.
#[allow(dead_code)]
pub struct WorkerClient {
    socket_path: PathBuf,
    worker_script: PathBuf,
    // TODO: tokio runtime, socket connection, background reader thread
}

impl WorkerClient {
    pub fn new(socket_path: PathBuf, worker_script: PathBuf) -> Self {
        Self {
            socket_path,
            worker_script,
        }
    }

    /// Spawn the worker process and connect.
    pub fn connect(&mut self) -> Result<(), ClientError> {
        // TODO: spawn python worker, connect unix socket
        Ok(())
    }

    /// Submit a job to the worker.
    pub fn submit(&self, _req: WorkerRequest) -> Result<JobHandle, ClientError> {
        // TODO: send over socket
        Err(ClientError::NotConnected)
    }

    /// Poll for a response (non-blocking).
    pub fn poll(&self, _handle: &JobHandle) -> Option<WorkerResponse> {
        // TODO: check background reader
        None
    }
}
