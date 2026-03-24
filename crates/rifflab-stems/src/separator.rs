use std::path::Path;

use rifflab_core::ipc::{DemucsModel, WorkerRequest, WorkerResponse};
use rifflab_ipc::client::{ClientError, JobHandle, WorkerClient};

/// Manages stem separation jobs via the Demucs Python worker.
pub struct StemSeparator {
    client: WorkerClient,
}

impl StemSeparator {
    /// Create a new `StemSeparator` wrapping a connected `WorkerClient`.
    pub fn new(client: WorkerClient) -> Self {
        Self { client }
    }

    /// Submit a separation job. Returns a handle for progress polling.
    pub fn separate(
        &self,
        input_path: &Path,
        output_dir: &Path,
        model: DemucsModel,
    ) -> Result<JobHandle, ClientError> {
        let req = WorkerRequest::Separate {
            input_path: input_path.to_string_lossy().into_owned(),
            output_dir: output_dir.to_string_lossy().into_owned(),
            model: model.as_str().to_owned(),
        };
        self.client.submit(req)
    }

    /// Poll for the next response for a given job (non-blocking).
    ///
    /// Returns `Some(response)` if a progress update, completion, or failure
    /// is available, `None` otherwise. Call this periodically (e.g. from a UI
    /// tick or background loop).
    pub fn poll_progress(&self, handle: &JobHandle) -> Option<WorkerResponse> {
        self.client.poll(handle)
    }

    /// Drain all pending responses for a given job (non-blocking).
    pub fn poll_all(&self, handle: &JobHandle) -> Vec<WorkerResponse> {
        self.client.poll_all(handle)
    }

    /// Cancel an in-progress separation job.
    pub fn cancel(&self, handle: &JobHandle) -> Result<(), ClientError> {
        let req = WorkerRequest::Cancel {
            job_id: handle.job_id,
        };
        self.client.submit(req)?;
        Ok(())
    }

    /// Shut down the underlying worker process.
    pub fn shutdown(mut self) -> Result<(), ClientError> {
        self.client.shutdown()
    }
}
