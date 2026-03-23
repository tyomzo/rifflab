use serde::{Deserialize, Serialize};

/// Demucs model selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemucsModel {
    /// 4-stem: vocals, drums, bass, other.
    HtDemucs,
    /// 6-stem: vocals, drums, bass, guitar, piano, other.
    HtDemucs6s,
}

impl DemucsModel {
    pub fn as_str(&self) -> &str {
        match self {
            Self::HtDemucs => "htdemucs",
            Self::HtDemucs6s => "htdemucs_6s",
        }
    }

    pub fn stem_count(&self) -> usize {
        match self {
            Self::HtDemucs => 4,
            Self::HtDemucs6s => 6,
        }
    }
}

impl Default for DemucsModel {
    fn default() -> Self {
        Self::HtDemucs
    }
}

/// Requests sent from the Rust client to a Python worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerRequest {
    /// Stem separation job.
    Separate {
        input_path: String,
        output_dir: String,
        model: String,
    },
    /// Beat tracking job.
    DetectBeats {
        input_path: String,
        output_path: String,
    },
    /// Cancel an in-progress job.
    Cancel {
        job_id: u64,
    },
    /// Shutdown the worker process.
    Shutdown,
}

/// Responses sent from a Python worker back to the Rust client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerResponse {
    /// Job has been accepted and started.
    JobStarted {
        job_id: u64,
    },
    /// Progress update for an in-progress job.
    Progress {
        job_id: u64,
        percent: f32,
        message: String,
    },
    /// Job completed successfully.
    JobCompleted {
        job_id: u64,
        output_paths: Vec<String>,
    },
    /// Job failed.
    JobFailed {
        job_id: u64,
        error: String,
    },
    /// Job was cancelled.
    JobCancelled {
        job_id: u64,
    },
}
