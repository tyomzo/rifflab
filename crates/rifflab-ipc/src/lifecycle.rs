use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Default timeout waiting for the worker socket to appear.
const SOCKET_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Polling interval while waiting for the socket.
const SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Spawn a Python worker process.
pub fn spawn_worker(script_path: &Path, socket_path: &Path) -> std::io::Result<Child> {
    Command::new("python3")
        .arg(script_path)
        .arg("--socket")
        .arg(socket_path)
        .spawn()
}

/// Spawn the Python worker and wait for the socket file to appear.
///
/// Returns the `Child` process handle once the socket exists on disk,
/// or an error if the timeout elapses or the child exits prematurely.
pub fn spawn_worker_and_wait(
    script_path: &Path,
    socket_path: &Path,
) -> std::io::Result<Child> {
    // Remove stale socket file if it exists.
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }

    let mut child = spawn_worker(script_path, socket_path)?;
    let start = Instant::now();

    loop {
        // Check if the socket file has appeared.
        if socket_path.exists() {
            log::info!("Worker socket ready at {:?}", socket_path);
            return Ok(child);
        }

        // Check if the child has exited unexpectedly.
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Worker process exited prematurely with status: {status}"),
                ));
            }
            Ok(None) => { /* still running, keep waiting */ }
            Err(e) => return Err(e),
        }

        // Check timeout.
        if start.elapsed() > SOCKET_WAIT_TIMEOUT {
            // Kill the child since it didn't create the socket in time.
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "Timed out waiting for worker socket at {:?} after {:?}",
                    socket_path, SOCKET_WAIT_TIMEOUT
                ),
            ));
        }

        std::thread::sleep(SOCKET_POLL_INTERVAL);
    }
}
