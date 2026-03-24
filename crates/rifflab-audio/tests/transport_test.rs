use rifflab_audio::transport::Transport;
use rifflab_core::transport::TransportState;

/// Helper: advance the transport by calling rt_handle.advance() with a consumer.
fn advance(transport: &Transport, commands: &mut rifflab_core::rtrb::Consumer<rifflab_core::transport::TransportCommand>, frames: usize) -> bool {
    transport.rt_handle().advance(frames, commands)
}

#[test]
fn test_initial_state_is_stopped() {
    let transport = Transport::new(48000);
    assert_eq!(transport.state(), TransportState::Stopped);
    assert_eq!(transport.position().frame, 0);
}

#[test]
fn test_play_advances_position() {
    let mut transport = Transport::new(48000);
    let mut rx = transport.take_command_rx().unwrap();
    let rt = transport.rt_handle();

    transport.play();

    // Advance a few buffers
    for _ in 0..10 {
        rt.advance(256, &mut rx);
    }

    assert_eq!(transport.state(), TransportState::Playing);
    assert_eq!(transport.position().frame, 256 * 10);
}

#[test]
fn test_pause_holds_position() {
    let mut transport = Transport::new(48000);
    let mut rx = transport.take_command_rx().unwrap();
    let rt = transport.rt_handle();

    transport.play();
    for _ in 0..5 {
        rt.advance(256, &mut rx);
    }
    let pos_before_pause = transport.position().frame;

    transport.pause();
    for _ in 0..5 {
        rt.advance(256, &mut rx);
    }

    assert_eq!(transport.state(), TransportState::Paused);
    assert_eq!(transport.position().frame, pos_before_pause);
}

#[test]
fn test_stop_resets_position() {
    let mut transport = Transport::new(48000);
    let mut rx = transport.take_command_rx().unwrap();
    let rt = transport.rt_handle();

    transport.play();
    for _ in 0..5 {
        rt.advance(256, &mut rx);
    }
    assert!(transport.position().frame > 0);

    transport.stop();
    rt.advance(256, &mut rx); // Process the stop command

    assert_eq!(transport.state(), TransportState::Stopped);
    assert_eq!(transport.position().frame, 0);
}

#[test]
fn test_seek_jumps_position() {
    let mut transport = Transport::new(48000);
    let mut rx = transport.take_command_rx().unwrap();
    let rt = transport.rt_handle();

    transport.set_length(480000); // 10 seconds
    transport.play();
    rt.advance(256, &mut rx);

    transport.seek(48000); // Seek to 1 second
    rt.advance(256, &mut rx);

    // Position should be 48000 + 256 (seek + one buffer advance)
    assert_eq!(transport.position().frame, 48000 + 256);
}

#[test]
fn test_loop_wraps_at_end() {
    let mut transport = Transport::new(48000);
    let mut rx = transport.take_command_rx().unwrap();
    let rt = transport.rt_handle();

    // Set length and loop region
    transport.set_length(4800); // 100ms
    let loop_region = rifflab_core::transport::LoopRegion {
        start_frame: 1000,
        end_frame: 3000,
    };
    transport.send_command(rifflab_core::transport::TransportCommand::SetLoop(Some(loop_region)));

    // Seek near loop end
    transport.seek(2900);
    transport.play();

    // Advance past the loop end
    rt.advance(256, &mut rx); // Processes seek + play + setloop, position = 2900 + 256 = 3156 > 3000

    let pos = transport.position().frame;
    // Should have wrapped to loop start (1000)
    assert!(pos >= 1000 && pos < 3000,
        "Expected position between 1000-3000 after loop wrap, got {pos}");
}

#[test]
fn test_stop_at_end_without_loop() {
    let mut transport = Transport::new(48000);
    let mut rx = transport.take_command_rx().unwrap();
    let rt = transport.rt_handle();

    transport.set_length(500); // Very short
    transport.play();

    // Advance past the end
    for _ in 0..5 {
        rt.advance(256, &mut rx);
    }

    // Should have stopped and reset to 0
    assert_eq!(transport.state(), TransportState::Stopped);
    assert_eq!(transport.position().frame, 0);
}

#[test]
fn test_position_seconds() {
    let transport = Transport::new(48000);
    // Position is 0 initially
    let pos = transport.position();
    assert_eq!(pos.seconds(), 0.0);

    // After some advance
    let mut t = Transport::new(48000);
    let mut rx = t.take_command_rx().unwrap();
    let rt = t.rt_handle();
    t.play();
    rt.advance(48000, &mut rx); // 1 second worth of samples

    let pos = t.position();
    assert!((pos.seconds() - 1.0).abs() < 0.001,
        "Expected ~1.0s, got {}s", pos.seconds());
}
