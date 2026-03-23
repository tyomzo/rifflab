/// Spectral flux-based onset detection.
/// Operates offline on a full audio buffer.
pub fn detect_onsets(samples: &[f32], sample_rate: u32) -> Vec<f64> {
    // TODO: implement spectral flux onset detection using rustfft
    // For now, return empty
    let _ = (samples, sample_rate);
    Vec::new()
}
