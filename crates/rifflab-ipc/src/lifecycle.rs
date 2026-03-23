use std::path::Path;
use std::process::{Child, Command};

/// Spawn a Python worker process.
pub fn spawn_worker(script_path: &Path, socket_path: &Path) -> std::io::Result<Child> {
    Command::new("python3")
        .arg(script_path)
        .arg("--socket")
        .arg(socket_path)
        .spawn()
}
