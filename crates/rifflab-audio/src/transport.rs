use rifflab_core::transport::{SongPosition, TransportCommand, TransportState};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Transport control — shared between UI and audio threads via atomics.
pub struct Transport {
    state: Arc<AtomicU8>,
    position: Arc<AtomicU64>,
    length: Arc<AtomicU64>,
    sample_rate: u32,
    loop_start: Arc<AtomicU64>,
    loop_end: Arc<AtomicU64>,
    loop_enabled: Arc<AtomicBool>,
    command_tx: rifflab_core::rtrb::Producer<TransportCommand>,
    command_rx: Option<rifflab_core::rtrb::Consumer<TransportCommand>>,
}

/// Atomic u8 for transport state.
use std::sync::atomic::AtomicU8;

impl Transport {
    pub fn new(sample_rate: u32) -> Self {
        let (tx, rx) = rifflab_core::rtrb::RingBuffer::new(64);
        Self {
            state: Arc::new(AtomicU8::new(0)),
            position: Arc::new(AtomicU64::new(0)),
            length: Arc::new(AtomicU64::new(0)),
            sample_rate,
            loop_start: Arc::new(AtomicU64::new(0)),
            loop_end: Arc::new(AtomicU64::new(0)),
            loop_enabled: Arc::new(AtomicBool::new(false)),
            command_tx: tx,
            command_rx: Some(rx),
        }
    }

    /// Take the command receiver (for the audio thread).
    pub fn take_command_rx(&mut self) -> Option<rifflab_core::rtrb::Consumer<TransportCommand>> {
        self.command_rx.take()
    }

    /// Send a command from the UI thread to the audio thread.
    pub fn send_command(&mut self, cmd: TransportCommand) {
        let _ = self.command_tx.push(cmd);
    }

    pub fn play(&mut self) {
        self.send_command(TransportCommand::Play);
    }

    pub fn pause(&mut self) {
        self.send_command(TransportCommand::Pause);
    }

    pub fn stop(&mut self) {
        self.send_command(TransportCommand::Stop);
    }

    pub fn seek(&mut self, frame: u64) {
        self.send_command(TransportCommand::Seek(frame));
    }

    /// Get current state (readable from any thread).
    pub fn state(&self) -> TransportState {
        match self.state.load(Ordering::Relaxed) {
            1 => TransportState::Playing,
            2 => TransportState::Paused,
            _ => TransportState::Stopped,
        }
    }

    /// Get current position (readable from any thread).
    pub fn position(&self) -> SongPosition {
        SongPosition::new(self.position.load(Ordering::Relaxed), self.sample_rate)
    }

    /// Set the total length in frames.
    pub fn set_length(&self, frames: u64) {
        self.length.store(frames, Ordering::Relaxed);
    }

    /// Get a handle for the audio thread to update position/state.
    pub fn rt_handle(&self) -> TransportRtHandle {
        TransportRtHandle {
            state: Arc::clone(&self.state),
            position: Arc::clone(&self.position),
            length: Arc::clone(&self.length),
            sample_rate: self.sample_rate,
            loop_start: Arc::clone(&self.loop_start),
            loop_end: Arc::clone(&self.loop_end),
            loop_enabled: Arc::clone(&self.loop_enabled),
        }
    }
}

/// Audio-thread side of transport. Only uses atomics — no locks.
pub struct TransportRtHandle {
    state: Arc<AtomicU8>,
    position: Arc<AtomicU64>,
    length: Arc<AtomicU64>,
    sample_rate: u32,
    loop_start: Arc<AtomicU64>,
    loop_end: Arc<AtomicU64>,
    loop_enabled: Arc<AtomicBool>,
}

impl TransportRtHandle {
    /// Process pending commands and advance position by `frames`.
    /// Returns true if transport is currently playing.
    pub fn advance(
        &self,
        frames: usize,
        commands: &mut rifflab_core::rtrb::Consumer<TransportCommand>,
    ) -> bool {
        // Process commands
        while let Ok(cmd) = commands.pop() {
            match cmd {
                TransportCommand::Play => {
                    self.state.store(1, Ordering::Relaxed);
                }
                TransportCommand::Pause => {
                    self.state.store(2, Ordering::Relaxed);
                }
                TransportCommand::Stop => {
                    self.state.store(0, Ordering::Relaxed);
                    self.position.store(0, Ordering::Relaxed);
                }
                TransportCommand::Seek(frame) => {
                    self.position.store(frame, Ordering::Relaxed);
                }
                TransportCommand::SetLoop(region) => {
                    if let Some(r) = region {
                        self.loop_start.store(r.start_frame, Ordering::Relaxed);
                        self.loop_end.store(r.end_frame, Ordering::Relaxed);
                        self.loop_enabled.store(true, Ordering::Relaxed);
                    } else {
                        self.loop_enabled.store(false, Ordering::Relaxed);
                    }
                }
            }
        }

        let playing = self.state.load(Ordering::Relaxed) == 1;
        if playing {
            let mut pos = self.position.load(Ordering::Relaxed);
            pos += frames as u64;

            // Check loop boundary first
            if self.loop_enabled.load(Ordering::Relaxed) {
                let loop_end = self.loop_end.load(Ordering::Relaxed);
                if loop_end > 0 && pos >= loop_end {
                    let loop_start = self.loop_start.load(Ordering::Relaxed);
                    pos = loop_start;
                }
            }

            // Check song end
            let length = self.length.load(Ordering::Relaxed);
            if length > 0 && pos >= length {
                pos = 0;
                self.state.store(0, Ordering::Relaxed);
            }

            self.position.store(pos, Ordering::Relaxed);
        }

        playing
    }

    pub fn position_frames(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn is_playing(&self) -> bool {
        self.state.load(Ordering::Relaxed) == 1
    }
}
