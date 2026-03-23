use std::path::Path;
use rifflab_ipc::client::{ClientError, JobHandle, WorkerClient};
use rifflab_core::ipc::WorkerRequest;

/// Beat tracking via madmom Python worker.
pub struct BeatTracker {
    client: WorkerClient,
}

impl BeatTracker {
    pub fn new(client: WorkerClient) -> Self {
        Self { client }
    }

    /// Submit a beat tracking job.
    pub fn detect_beats(
        &self,
        input_path: &Path,
        output_path: &Path,
    ) -> Result<JobHandle, ClientError> {
        let req = WorkerRequest::DetectBeats {
            input_path: input_path.to_string_lossy().into_owned(),
            output_path: output_path.to_string_lossy().into_owned(),
        };
        self.client.submit(req)
    }
}
