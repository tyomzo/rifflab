use rifflab_audio::transport::Transport;
use rifflab_core::transport::TransportState;

/// Helper: process commands then advance the transport.
fn tick(rt: &rifflab_audio::transport::TransportRtHandle, commands: &mut rifflab_core::rtrb::Consumer<rifflab_core::transport::TransportCommand>, frames: usize) -> bool {
    let playing = rt.process_commands(commands);
    rt.advance(frames);
    playing
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

    for _ in 0..10 {
        tick(&rt, &mut rx, 256);
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
        tick(&rt, &mut rx, 256);
    }
    let pos_before_pause = transport.position().frame;

    transport.pause();
    for _ in 0..5 {
        tick(&rt, &mut rx, 256);
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
        tick(&rt, &mut rx, 256);
    }
    assert!(transport.position().frame > 0);

    transport.stop();
    tick(&rt, &mut rx, 256);

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
    tick(&rt, &mut rx, 256);

    transport.seek(48000); // Seek to 1 second
    tick(&rt, &mut rx, 256);

    // Position should be 48000 + 256 (seek + one buffer advance)
    assert_eq!(transport.position().frame, 48000 + 256);
}

#[test]
fn test_loop_wraps_at_end() {
    let mut transport = Transport::new(48000);
    let mut rx = transport.take_command_rx().unwrap();
    let rt = transport.rt_handle();

    transport.set_length(4800); // 100ms
    let loop_region = rifflab_core::transport::LoopRegion {
        start_frame: 1000,
        end_frame: 3000,
    };
    transport.send_command(rifflab_core::transport::TransportCommand::SetLoop(Some(loop_region)));

    // Seek near loop end
    transport.seek(2900);
    transport.play();

    tick(&rt, &mut rx, 256); // Processes seek + play + setloop, advances by 256

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

    for _ in 0..5 {
        tick(&rt, &mut rx, 256);
    }

    // Should have stopped and reset to 0
    assert_eq!(transport.state(), TransportState::Stopped);
    assert_eq!(transport.position().frame, 0);
}

#[test]
fn test_position_seconds() {
    let transport = Transport::new(48000);
    let pos = transport.position();
    assert_eq!(pos.seconds(), 0.0);

    let mut t = Transport::new(48000);
    let mut rx = t.take_command_rx().unwrap();
    let rt = t.rt_handle();
    t.play();
    tick(&rt, &mut rx, 48000); // 1 second worth of samples

    let pos = t.position();
    assert!((pos.seconds() - 1.0).abs() < 0.001,
        "Expected ~1.0s, got {}s", pos.seconds());
}
