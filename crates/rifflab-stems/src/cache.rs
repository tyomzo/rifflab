use std::path::Path;

/// Check if stems already exist for a given song directory.
pub fn stems_exist(song_dir: &Path, stem_count: usize) -> bool {
    let stems_dir = song_dir.join("stems");
    if !stems_dir.is_dir() {
        return false;
    }
    let required = if stem_count == 6 {
        &["vocals.wav", "drums.wav", "bass.wav", "guitar.wav", "piano.wav", "other.wav"][..]
    } else {
        &["vocals.wav", "drums.wav", "bass.wav", "other.wav"][..]
    };
    required.iter().all(|name| stems_dir.join(name).exists())
}
