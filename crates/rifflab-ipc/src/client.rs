use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use rifflab_core::ipc::{WorkerRequest, WorkerResponse};
use thiserror::Error;

use crate::lifecycle::spawn_worker_and_wait;
use crate::protocol;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("Worker not connected")]
    NotConnected,
    #[error("Worker process exited unexpectedly")]
    WorkerCrashed,
    #[error("Protocol error: {0}")]
    Protocol(#[from] protocol::ProtocolError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Client has been shut down")]
    Shutdown,
}

/// Handle to a submitted job.
#[derive(Debug, Clone)]
pub struct JobHandle {
    pub job_id: u64,
}

/// Client for communicating with a Python worker process over a Unix domain socket.
///
/// The client spawns a background reader thread that continuously reads
/// length-prefixed MessagePack responses from the socket and routes them
/// into per-job queues. The main thread submits requests via `submit()`
/// and checks for responses via `poll()`.
pub struct WorkerClient {
    socket_path: PathBuf,
    worker_script: PathBuf,
    next_job_id: AtomicU64,
    /// The write half of the socket, protected by a mutex so `submit()` is safe
    /// to call from any thread (though typically called from one).
    writer: Option<Arc<Mutex<UnixStream>>>,
    /// Per-job response receivers. The background reader pushes responses here.
    job_receivers: Arc<Mutex<HashMap<u64, Vec<WorkerResponse>>>>,
    /// Channel from the background reader for responses that don't match a known job
    /// or for routing into the per-job map.
    _reader_handle: Option<JoinHandle<()>>,
    /// Handle to the spawned Python worker child process.
    child: Option<Child>,
}

impl WorkerClient {
    /// Create a new `WorkerClient` with the given socket and worker script paths.
    /// Call `connect()` to actually spawn the worker and establish the connection.
    pub fn new(socket_path: PathBuf, worker_script: PathBuf) -> Self {
        Self {
            socket_path,
            worker_script,
            next_job_id: AtomicU64::new(1),
            writer: None,
            job_receivers: Arc::new(Mutex::new(HashMap::new())),
            _reader_handle: None,
            child: None,
        }
    }

    /// Spawn the Python worker process, wait for the socket to appear,
    /// connect, and start the background reader thread.
    pub fn connect(&mut self) -> Result<(), ClientError> {
        // Spawn worker and wait for socket.
        let child = spawn_worker_and_wait(&self.worker_script, &self.socket_path)?;
        self.child = Some(child);

        // Connect to the Unix domain socket.
        let stream = UnixStream::connect(&self.socket_path)?;
        log::info!("Connected to worker socket at {:?}", self.socket_path);

        // Clone the stream for the reader thread.
        let reader_stream = stream.try_clone()?;
        let writer = Arc::new(Mutex::new(stream));
        self.writer = Some(writer);

        // Start the background reader thread.
        let job_receivers = Arc::clone(&self.job_receivers);
        let handle = thread::Builder::new()
            .name("ipc-reader".into())
            .spawn(move || {
                Self::reader_loop(reader_stream, job_receivers);
            })
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        self._reader_handle = Some(handle);
        Ok(())
    }

    /// Background reader loop. Reads responses from the socket and routes
    /// them to the appropriate per-job queue based on job_id.
    fn reader_loop(
        mut stream: UnixStream,
        job_receivers: Arc<Mutex<HashMap<u64, Vec<WorkerResponse>>>>,
    ) {
        loop {
            match protocol::read_response(&mut stream) {
                Ok(response) => {
                    let job_id = Self::extract_job_id(&response);
                    let mut map = job_receivers.lock().unwrap();
                    map.entry(job_id).or_default().push(response);
                }
                Err(protocol::ProtocolError::Io(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    log::info!("Worker socket closed (EOF), reader thread exiting");
                    break;
                }
                Err(e) => {
                    log::error!("Error reading from worker socket: {e}");
                    break;
                }
            }
        }
    }

    /// Extract the job_id from a WorkerResponse.
    fn extract_job_id(response: &WorkerResponse) -> u64 {
        match response {
            WorkerResponse::JobStarted { job_id } => *job_id,
            WorkerResponse::Progress { job_id, .. } => *job_id,
            WorkerResponse::JobCompleted { job_id, .. } => *job_id,
            WorkerResponse::JobFailed { job_id, .. } => *job_id,
            WorkerResponse::JobCancelled { job_id } => *job_id,
        }
    }

    /// Submit a request to the worker. Returns a `JobHandle` that can be used
    /// to poll for responses.
    pub fn submit(&self, req: WorkerRequest) -> Result<JobHandle, ClientError> {
        let writer = self.writer.as_ref().ok_or(ClientError::NotConnected)?;
        let job_id = self.next_job_id.fetch_add(1, Ordering::Relaxed);

        // Pre-register the job in the receivers map so the reader thread
        // can route responses even before poll() is called.
        {
            let mut map = self.job_receivers.lock().unwrap();
            map.entry(job_id).or_default();
        }

        // Write the request to the socket.
        {
            let mut stream = writer.lock().unwrap();
            protocol::write_request(&mut *stream, &req)?;
        }

        log::debug!("Submitted job {job_id}: {req:?}");
        Ok(JobHandle { job_id })
    }

    /// Poll for the next response for a given job (non-blocking).
    ///
    /// Returns `Some(response)` if one is available, `None` otherwise.
    /// Call this repeatedly to receive progress updates, completion, or failure.
    pub fn poll(&self, handle: &JobHandle) -> Option<WorkerResponse> {
        let mut map = self.job_receivers.lock().unwrap();
        if let Some(queue) = map.get_mut(&handle.job_id) {
            if !queue.is_empty() {
                return Some(queue.remove(0));
            }
        }
        None
    }

    /// Drain all pending responses for a given job (non-blocking).
    pub fn poll_all(&self, handle: &JobHandle) -> Vec<WorkerResponse> {
        let mut map = self.job_receivers.lock().unwrap();
        if let Some(queue) = map.get_mut(&handle.job_id) {
            std::mem::take(queue)
        } else {
            Vec::new()
        }
    }

    /// Send a shutdown request to the worker and wait for the child process to exit.
    pub fn shutdown(&mut self) -> Result<(), ClientError> {
        // Send the Shutdown request.
        if let Some(ref writer) = self.writer {
            let mut stream = writer.lock().unwrap();
            if let Err(e) = protocol::write_request(&mut *stream, &WorkerRequest::Shutdown) {
                log::warn!("Error sending shutdown request: {e}");
            }
        }

        // Drop the writer so the socket closes, which will cause the reader
        // thread to see EOF and exit.
        self.writer.take();

        // Wait for the reader thread to finish.
        if let Some(handle) = self._reader_handle.take() {
            let _ = handle.join();
        }

        // Wait for the child process to exit.
        if let Some(ref mut child) = self.child {
            match child.wait() {
                Ok(status) => {
                    log::info!("Worker process exited with status: {status}");
                }
                Err(e) => {
                    log::warn!("Error waiting for worker process: {e}");
                }
            }
        }
        self.child.take();

        Ok(())
    }

    /// Check whether the client is currently connected to a worker.
    pub fn is_connected(&self) -> bool {
        self.writer.is_some()
    }
}

impl Drop for WorkerClient {
    fn drop(&mut self) {
        if self.is_connected() {
            if let Err(e) = self.shutdown() {
                log::warn!("Error during WorkerClient drop shutdown: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn new_client_is_not_connected() {
        let client = WorkerClient::new(
            PathBuf::from("/tmp/test.sock"),
            PathBuf::from("/tmp/worker.py"),
        );
        assert!(!client.is_connected());
    }

    #[test]
    fn submit_without_connect_returns_not_connected() {
        let client = WorkerClient::new(
            PathBuf::from("/tmp/test.sock"),
            PathBuf::from("/tmp/worker.py"),
        );
        let result = client.submit(WorkerRequest::Shutdown);
        assert!(result.is_err());
        match result.unwrap_err() {
            ClientError::NotConnected => {}
            other => panic!("Expected NotConnected, got: {other}"),
        }
    }

    #[test]
    fn poll_without_connect_returns_none() {
        let client = WorkerClient::new(
            PathBuf::from("/tmp/test.sock"),
            PathBuf::from("/tmp/worker.py"),
        );
        let handle = JobHandle { job_id: 1 };
        assert!(client.poll(&handle).is_none());
    }
}
