use std::path::Path;
use rifflab_core::ipc::{DemucsModel, WorkerRequest};
use rifflab_ipc::client::{ClientError, JobHandle, WorkerClient};

/// Manages stem separation jobs via the Demucs Python worker.
pub struct StemSeparator {
    client: WorkerClient,
}

impl StemSeparator {
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
}
