#!/usr/bin/env python3
"""RiffLab madmom beat tracking worker.

Communicates with the Rust host via a Unix domain socket using length-prefixed
MessagePack frames.  Protocol:

    [4-byte big-endian length][MessagePack payload]

Accepts WorkerRequest messages (DetectBeats, Cancel, Shutdown) and replies with
WorkerResponse messages (JobStarted, Progress, JobCompleted, JobFailed).

Output: a JSON file (beat_grid.json) containing detected beats with bar/beat
numbering and estimated tempo.
"""

from __future__ import annotations

import argparse
import json
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
log = logging.getLogger("madmom_worker")

# ---------------------------------------------------------------------------
# Frame I/O helpers (same protocol as demucs_worker)
# ---------------------------------------------------------------------------

MAX_FRAME_SIZE = 16 * 1024 * 1024  # 16 MiB


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
# Beat detection
# ---------------------------------------------------------------------------

# Default assumed time signature for bar/beat numbering.
BEATS_PER_BAR = 4


def estimate_bpm(beat_times: list[float]) -> float:
    """Estimate BPM from a list of beat timestamps (seconds).

    Uses the median inter-beat interval to be robust against outliers.
    """
    if len(beat_times) < 2:
        return 120.0  # sensible default

    intervals = [
        beat_times[i + 1] - beat_times[i] for i in range(len(beat_times) - 1)
    ]
    intervals.sort()
    median_ibi = intervals[len(intervals) // 2]

    if median_ibi <= 0:
        return 120.0

    return 60.0 / median_ibi


def build_beat_grid(beat_times: list[float], beats_per_bar: int = BEATS_PER_BAR) -> dict:
    """Build the beat_grid.json payload from raw beat timestamps."""
    bpm = estimate_bpm(beat_times)
    beats = []
    for idx, t in enumerate(beat_times):
        bar = (idx // beats_per_bar) + 1
        beat = (idx % beats_per_bar) + 1
        beats.append({
            "time_seconds": round(float(t), 6),
            "bar": bar,
            "beat": beat,
            "bpm": round(bpm, 2),
        })
    return {"beats": beats}


class MadmomWorker:
    """Manages state for the madmom beat tracking worker."""

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
    # DetectBeats
    # ------------------------------------------------------------------ #

    def handle_detect_beats(
        self,
        conn: socket.socket,
        input_path: str,
        output_path: str,
    ) -> None:
        """Run madmom beat tracking and write the result JSON."""
        job_id = self.next_job_id()
        self._current_job_id = job_id
        self._cancel_event.clear()

        send_job_started(conn, job_id)
        send_progress(conn, job_id, 0.0, "Starting beat detection")

        try:
            self._run_beat_detection(conn, job_id, input_path, output_path)
        except Exception:
            error_msg = traceback.format_exc()
            log.error("Beat detection failed for job %d:\n%s", job_id, error_msg)
            send_job_failed(conn, job_id, error_msg)
        finally:
            self._current_job_id = None
            self._cancel_event.clear()

    def _run_beat_detection(
        self,
        conn: socket.socket,
        job_id: int,
        input_path: str,
        output_path: str,
    ) -> None:
        from madmom.features.beats import (
            DBNBeatTrackingProcessor,
            RNNBeatProcessor,
        )

        # Validate input.
        if not Path(input_path).is_file():
            raise FileNotFoundError(f"Input file not found: {input_path}")

        # Ensure output directory exists.
        os.makedirs(Path(output_path).parent, exist_ok=True)

        if self._cancel_event.is_set():
            send_job_cancelled(conn, job_id)
            return

        # Step 1: Extract beat activations with RNN.
        send_progress(conn, job_id, 10.0, "Computing beat activations (RNN)")
        log.info("Running RNNBeatProcessor on %s", input_path)
        rnn_processor = RNNBeatProcessor()
        activations = rnn_processor(input_path)

        if self._cancel_event.is_set():
            send_job_cancelled(conn, job_id)
            return

        # Step 2: Track beats with DBN.
        send_progress(conn, job_id, 60.0, "Tracking beats (DBN)")
        log.info("Running DBNBeatTrackingProcessor")
        beat_processor = DBNBeatTrackingProcessor(fps=100)
        beat_times: list[float] = beat_processor(activations).tolist()

        if self._cancel_event.is_set():
            send_job_cancelled(conn, job_id)
            return

        # Step 3: Build structured beat grid.
        send_progress(conn, job_id, 90.0, "Building beat grid")
        beat_grid = build_beat_grid(beat_times)
        bpm = beat_grid["beats"][0]["bpm"] if beat_grid["beats"] else 0.0
        log.info("Detected %d beats, estimated BPM: %.1f", len(beat_grid["beats"]), bpm)

        # Step 4: Write JSON output.
        with open(output_path, "w", encoding="utf-8") as f:
            json.dump(beat_grid, f, indent=2)
        log.info("Beat grid written to %s", output_path)

        send_progress(conn, job_id, 100.0, "Done")
        send_job_completed(conn, job_id, [output_path])
        log.info("Job %d completed", job_id)


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------


def run_worker(socket_path: str) -> None:
    """Create the Unix socket, accept one connection, and process messages."""
    worker = MadmomWorker()

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


def _message_loop(conn: socket.socket, worker: MadmomWorker) -> None:
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
    conn: socket.socket, worker: MadmomWorker, msg: dict[str, Any]
) -> None:
    """Dispatch a single decoded WorkerRequest message."""
    if "DetectBeats" in msg:
        params = msg["DetectBeats"]
        worker.handle_detect_beats(
            conn,
            input_path=params["input_path"],
            output_path=params["output_path"],
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
        description="RiffLab madmom beat tracking worker"
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

    # Handle SIGTERM gracefully.
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
