#!/usr/bin/env python3
"""RiffLab Demucs stem separation worker.

Communicates with the Rust host via a Unix domain socket using length-prefixed
MessagePack frames.  Protocol:

    [4-byte big-endian length][MessagePack payload]

Accepts WorkerRequest messages (Separate, Cancel, Shutdown) and replies with
WorkerResponse messages (JobStarted, Progress, JobCompleted, JobFailed).
"""

from __future__ import annotations

import argparse
import logging
import os
import signal
import socket
import struct
import sys
import threading
import traceback
from pathlib import Path
from typing import Any

import msgpack

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------

LOG_FORMAT = "%(asctime)s [%(levelname)s] %(name)s: %(message)s"
logging.basicConfig(level=logging.INFO, format=LOG_FORMAT, stream=sys.stderr)
log = logging.getLogger("demucs_worker")

# ---------------------------------------------------------------------------
# Frame I/O helpers
# ---------------------------------------------------------------------------

MAX_FRAME_SIZE = 16 * 1024 * 1024  # 16 MiB, matches Rust side


def recv_exact(conn: socket.socket, n: int) -> bytes:
    """Read exactly *n* bytes from *conn*, or raise ``ConnectionError``."""
    buf = bytearray()
    while len(buf) < n:
        chunk = conn.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("connection closed while reading")
        buf.extend(chunk)
    return bytes(buf)


def read_message(conn: socket.socket) -> dict[str, Any]:
    """Read one length-prefixed MessagePack message and return it as a dict."""
    length_bytes = recv_exact(conn, 4)
    (length,) = struct.unpack("!I", length_bytes)
    if length > MAX_FRAME_SIZE:
        raise ValueError(f"frame too large: {length} bytes")
    payload = recv_exact(conn, length)
    return msgpack.unpackb(payload, raw=False)


def send_message(conn: socket.socket, msg: dict[str, Any]) -> None:
    """Send one length-prefixed MessagePack message."""
    payload = msgpack.packb(msg, use_bin_type=True)
    header = struct.pack("!I", len(payload))
    conn.sendall(header + payload)


# ---------------------------------------------------------------------------
# Response helpers
# ---------------------------------------------------------------------------


def send_job_started(conn: socket.socket, job_id: int) -> None:
    send_message(conn, {"JobStarted": {"job_id": job_id}})


def send_progress(
    conn: socket.socket, job_id: int, percent: float, message: str
) -> None:
    send_message(conn, {"Progress": {"job_id": job_id, "percent": percent, "message": message}})


def send_job_completed(
    conn: socket.socket, job_id: int, output_paths: list[str]
) -> None:
    send_message(conn, {"JobCompleted": {"job_id": job_id, "output_paths": output_paths}})


def send_job_failed(conn: socket.socket, job_id: int, error: str) -> None:
    send_message(conn, {"JobFailed": {"job_id": job_id, "error": error}})


def send_job_cancelled(conn: socket.socket, job_id: int) -> None:
    send_message(conn, {"JobCancelled": {"job_id": job_id}})


# ---------------------------------------------------------------------------
# Device selection
# ---------------------------------------------------------------------------


def select_device() -> "torch.device":
    """Return the best available torch device (CUDA preferred)."""
    import torch

    if torch.cuda.is_available():
        dev = torch.device("cuda")
        name = torch.cuda.get_device_name(0)
        log.info("Using CUDA device: %s", name)
    else:
        dev = torch.device("cpu")
        log.info("CUDA not available — using CPU")
    return dev


# ---------------------------------------------------------------------------
# Stem names per model
# ---------------------------------------------------------------------------

STEM_NAMES: dict[str, list[str]] = {
    "htdemucs": ["vocals", "drums", "bass", "other"],
    "htdemucs_ft": ["vocals", "drums", "bass", "other"],
    "htdemucs_6s": ["vocals", "drums", "bass", "guitar", "piano", "other"],
}


def stem_names_for_model(model_name: str) -> list[str]:
    """Return the expected stem names for a given Demucs model."""
    return STEM_NAMES.get(model_name, STEM_NAMES["htdemucs"])


# ---------------------------------------------------------------------------
# Separation logic
# ---------------------------------------------------------------------------


class DemucsWorker:
    """Manages state for the Demucs worker process."""

    def __init__(self) -> None:
        self._cancel_event = threading.Event()
        self._current_job_id: int | None = None
        self._job_counter: int = 0

    def next_job_id(self) -> int:
        self._job_counter += 1
        return self._job_counter

    def request_cancel(self, job_id: int) -> bool:
        """Signal cancellation for *job_id*.  Returns True if it matched."""
        if self._current_job_id == job_id:
            self._cancel_event.set()
            return True
        return False

    # ------------------------------------------------------------------ #
    # Separate
    # ------------------------------------------------------------------ #

    def handle_separate(
        self,
        conn: socket.socket,
        input_path: str,
        output_dir: str,
        model_name: str,
    ) -> None:
        """Run Demucs separation and stream progress back to *conn*."""
        job_id = self.next_job_id()
        self._current_job_id = job_id
        self._cancel_event.clear()

        send_job_started(conn, job_id)
        send_progress(conn, job_id, 0.0, "Starting separation")

        try:
            self._run_separation(conn, job_id, input_path, output_dir, model_name)
        except Exception:
            error_msg = traceback.format_exc()
            log.error("Separation failed for job %d:\n%s", job_id, error_msg)
            send_job_failed(conn, job_id, error_msg)
        finally:
            self._current_job_id = None
            self._cancel_event.clear()

    def _run_separation(
        self,
        conn: socket.socket,
        job_id: int,
        input_path: str,
        output_dir: str,
        model_name: str,
    ) -> None:
        import torch
        import torchaudio
        from demucs.apply import apply_model
        from demucs.pretrained import get_model

        # Validate inputs.
        if not Path(input_path).is_file():
            raise FileNotFoundError(f"Input file not found: {input_path}")
        os.makedirs(output_dir, exist_ok=True)

        device = select_device()

        # Check for cancellation early.
        if self._cancel_event.is_set():
            send_job_cancelled(conn, job_id)
            return

        # Load model.
        send_progress(conn, job_id, 10.0, f"Loading model {model_name}")
        log.info("Loading Demucs model: %s", model_name)
        model = get_model(model_name)
        model.to(device)

        if self._cancel_event.is_set():
            send_job_cancelled(conn, job_id)
            return

        # Load audio.
        send_progress(conn, job_id, 20.0, "Loading audio")
        log.info("Loading audio: %s", input_path)
        wav, sr = torchaudio.load(input_path)

        # Resample if needed.
        if sr != model.samplerate:
            log.info("Resampling from %d to %d Hz", sr, model.samplerate)
            send_progress(conn, job_id, 25.0, f"Resampling {sr} -> {model.samplerate} Hz")
            wav = torchaudio.functional.resample(wav, sr, model.samplerate)
            sr = model.samplerate

        # Ensure stereo.
        if wav.shape[0] == 1:
            wav = wav.repeat(2, 1)
        elif wav.shape[0] > 2:
            wav = wav[:2]

        # Add batch dimension: (channels, samples) -> (1, channels, samples)
        wav = wav.unsqueeze(0).to(device)

        if self._cancel_event.is_set():
            send_job_cancelled(conn, job_id)
            return

        # Run separation.
        send_progress(conn, job_id, 30.0, "Running separation")
        log.info("Running model inference (device=%s)", device)

        with torch.no_grad():
            sources = apply_model(
                model,
                wav,
                device=device,
                progress=False,  # We handle our own progress
                num_workers=0,
            )
        # sources shape: (1, num_sources, channels, samples)

        if self._cancel_event.is_set():
            send_job_cancelled(conn, job_id)
            return

        send_progress(conn, job_id, 80.0, "Saving stems")

        # Determine stem names from model metadata or fallback.
        if hasattr(model, "sources") and model.sources:
            names = list(model.sources)
        else:
            names = stem_names_for_model(model_name)

        # Save each stem.
        output_paths: list[str] = []
        num_sources = sources.shape[1]
        for i in range(num_sources):
            stem_name = names[i] if i < len(names) else f"stem_{i}"
            stem_path = str(Path(output_dir) / f"{stem_name}.wav")

            # Remove batch dim: (1, channels, samples) -> (channels, samples)
            stem_audio = sources[0, i].cpu()

            torchaudio.save(stem_path, stem_audio, sr)
            output_paths.append(stem_path)

            pct = 80.0 + (20.0 * (i + 1) / num_sources)
            send_progress(conn, job_id, pct, f"Saved {stem_name}.wav")
            log.info("Saved stem: %s", stem_path)

        send_job_completed(conn, job_id, output_paths)
        log.info("Job %d completed: %d stems written", job_id, len(output_paths))


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------


def run_worker(socket_path: str) -> None:
    """Create the Unix socket, accept one connection, and process messages."""
    worker = DemucsWorker()

    # Clean up stale socket file.
    if os.path.exists(socket_path):
        os.unlink(socket_path)

    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        server.bind(socket_path)
        server.listen(1)
        log.info("Listening on %s", socket_path)

        conn, _ = server.accept()
        log.info("Client connected")
    except Exception:
        server.close()
        raise

    # We no longer need the listening socket.
    server.close()

    try:
        _message_loop(conn, worker)
    finally:
        conn.close()
        # Clean up socket file.
        try:
            os.unlink(socket_path)
        except OSError:
            pass
        log.info("Worker shut down")


def _message_loop(conn: socket.socket, worker: DemucsWorker) -> None:
    """Read messages in a loop until Shutdown or disconnect."""
    while True:
        try:
            msg = read_message(conn)
        except ConnectionError:
            log.info("Connection closed by client")
            break
        except Exception:
            log.error("Error reading message:\n%s", traceback.format_exc())
            break

        log.debug("Received: %s", msg)

        try:
            _dispatch(conn, worker, msg)
        except _ShutdownRequested:
            log.info("Shutdown requested")
            break
        except Exception:
            # Protocol-level error — log but keep running.
            log.error("Error dispatching message:\n%s", traceback.format_exc())


class _ShutdownRequested(Exception):
    """Raised to break out of the message loop on Shutdown."""


def _dispatch(
    conn: socket.socket, worker: DemucsWorker, msg: dict[str, Any]
) -> None:
    """Dispatch a single decoded WorkerRequest message."""
    if "Separate" in msg:
        params = msg["Separate"]
        worker.handle_separate(
            conn,
            input_path=params["input_path"],
            output_dir=params["output_dir"],
            model_name=params["model"],
        )
    elif "Cancel" in msg:
        job_id = msg["Cancel"]["job_id"]
        matched = worker.request_cancel(job_id)
        log.info("Cancel request for job %d (matched=%s)", job_id, matched)
    elif "Shutdown" in msg:
        raise _ShutdownRequested()
    else:
        log.warning("Unknown message type: %s", list(msg.keys()))


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(
        description="RiffLab Demucs stem separation worker"
    )
    parser.add_argument(
        "--socket",
        required=True,
        help="Path for the Unix domain socket",
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Enable DEBUG logging",
    )
    args = parser.parse_args()

    if args.verbose:
        logging.getLogger().setLevel(logging.DEBUG)

    # Handle SIGTERM gracefully so systemd / the Rust host can stop us.
    signal.signal(signal.SIGTERM, lambda _sig, _frame: sys.exit(0))

    try:
        run_worker(args.socket)
    except KeyboardInterrupt:
        log.info("Interrupted")
    except Exception:
        log.critical("Fatal error:\n%s", traceback.format_exc())
        sys.exit(1)


if __name__ == "__main__":
    main()
