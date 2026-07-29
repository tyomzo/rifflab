//! Session persistence: save/load session directories with stem WAVs and metadata.

use std::path::PathBuf;

/// Manages session save/load state and background operations.
pub struct SessionManager {
    /// Current session directory (None = never saved).
    pub session_path: Option<PathBuf>,
    /// Original file path for session metadata.
    pub original_file_path: Option<PathBuf>,
    /// Background save result receiver.
    pub save_receiver: Option<std::sync::mpsc::Receiver<(bool, String)>>,
    /// Pending folder dialog result (for non-blocking Save As).
    pub save_dialog_rx: Option<std::sync::mpsc::Receiver<Option<PathBuf>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            session_path: None,
            original_file_path: None,
            save_receiver: None,
            save_dialog_rx: None,
        }
    }

    /// Poll background save operation. Returns (is_error, message) if complete.
    pub fn poll_save(&mut self) -> Option<(bool, String)> {
        if let Some(ref rx) = self.save_receiver {
            match rx.try_recv() {
                Ok(result) => {
                    self.save_receiver = None;
                    return Some(result);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.save_receiver = None;
                }
                _ => {}
            }
        }
        None
    }

    /// Poll the async Save As folder dialog. Returns Some(path) if user chose,
    /// Some(None) if cancelled, or None if still open.
    pub fn poll_dialog(&mut self) -> Option<Option<PathBuf>> {
        if let Some(ref rx) = self.save_dialog_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.save_dialog_rx = None;
                    return Some(result);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.save_dialog_rx = None;
                }
                _ => {}
            }
        }
        None
    }

    /// Launch a non-blocking folder picker dialog (uses zenity on Wayland).
    /// `start_dir`, if set, seeds zenity's initial directory.
    pub fn start_save_as_dialog(&mut self, start_dir: Option<PathBuf>) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut cmd = std::process::Command::new("zenity");
            cmd.args([
                "--file-selection",
                "--directory",
                "--title=Save Session — choose folder",
            ]);
            if let Some(dir) = start_dir {
                // zenity uses --filename to seed the dialog; append a trailing slash so it treats it as a folder.
                let mut s = dir.to_string_lossy().into_owned();
                if !s.ends_with('/') {
                    s.push('/');
                }
                cmd.arg(format!("--filename={}", s));
            }
            let result = cmd.output();
            let path = match result {
                Ok(out) if out.status.success() => {
                    let s = String::from_utf8_lossy(&out.stdout);
                    let s = s.trim();
                    if s.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(s))
                    }
                }
                _ => None,
            };
            let _ = tx.send(path);
        });
        self.save_dialog_rx = Some(rx);
    }

    /// Whether a save or dialog operation is in flight.
    pub fn is_busy(&self) -> bool {
        self.save_receiver.is_some() || self.save_dialog_rx.is_some()
    }

    /// Reset session state for a new file.
    pub fn new_file(&mut self, original_path: &std::path::Path) {
        self.original_file_path = Some(original_path.to_path_buf());
        self.session_path = None;
    }
}
