use anyhow::{Context, Result};
use std::path::PathBuf;

/// Manages the RiffLab data directory (~/.local/share/rifflab).
///
/// Layout:
/// ```text
/// ~/.local/share/rifflab/
/// ├── songs/          # Per-song subdirectories
/// │   └── {song_id}/
/// │       ├── original.wav
/// │       ├── metadata.toml
/// │       └── stems/  (future: separated stems)
/// ├── effects/        # User effect presets
/// ├── presets/        # Chain presets
/// └── sessions/       # Practice session records
/// ```
pub struct Library {
    pub root: PathBuf,
}

/// Subdirectories created on library init.
const SUBDIRS: &[&str] = &["songs", "effects", "presets", "sessions"];

#[allow(dead_code)]
impl Library {
    /// Initialize the library, creating the directory tree if it doesn't exist.
    pub fn init() -> Result<Self> {
        let root = default_library_root()
            .context("Could not determine data directory (XDG_DATA_HOME unset and no home dir)")?;

        for sub in SUBDIRS {
            let dir = root.join(sub);
            if !dir.exists() {
                std::fs::create_dir_all(&dir)
                    .with_context(|| format!("Failed to create {}", dir.display()))?;
                log::info!("Created library directory: {}", dir.display());
            }
        }

        log::info!("Library root: {}", root.display());
        Ok(Self { root })
    }

    /// Initialize the library at a custom root (for testing).
    pub fn init_at(root: PathBuf) -> Result<Self> {
        for sub in SUBDIRS {
            let dir = root.join(sub);
            if !dir.exists() {
                std::fs::create_dir_all(&dir)
                    .with_context(|| format!("Failed to create {}", dir.display()))?;
            }
        }
        Ok(Self { root })
    }

    /// Return the path to a song's directory (may not exist yet).
    pub fn song_dir(&self, song_id: &str) -> PathBuf {
        self.root.join("songs").join(song_id)
    }

    /// Ensure a song directory exists and return its path.
    pub fn ensure_song_dir(&self, song_id: &str) -> Result<PathBuf> {
        let dir = self.song_dir(song_id);
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("Failed to create song dir {}", dir.display()))?;
        }
        Ok(dir)
    }

    /// Return the path to the songs directory.
    pub fn songs_dir(&self) -> PathBuf {
        self.root.join("songs")
    }

    /// Return the path to the sessions directory.
    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// Return the path to the effects presets directory.
    pub fn effects_dir(&self) -> PathBuf {
        self.root.join("effects")
    }

    /// Return the path to the chain presets directory.
    pub fn presets_dir(&self) -> PathBuf {
        self.root.join("presets")
    }

    /// List song IDs present in the library.
    pub fn list_songs(&self) -> Result<Vec<String>> {
        let songs_dir = self.songs_dir();
        if !songs_dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(&songs_dir)
            .with_context(|| format!("Failed to read {}", songs_dir.display()))?
        {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    ids.push(name.to_string());
                }
            }
        }
        ids.sort();
        Ok(ids)
    }
}

/// Determine the default library root using XDG conventions.
fn default_library_root() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "rifflab")
        .map(|dirs| dirs.data_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_library_init_creates_subdirs() {
        let tmp = std::env::temp_dir().join(format!("rifflab_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let lib = Library::init_at(tmp.clone()).expect("init_at should succeed");
        assert_eq!(lib.root, tmp);

        for sub in SUBDIRS {
            assert!(
                tmp.join(sub).is_dir(),
                "Subdir '{}' should exist",
                sub
            );
        }

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_library_init_idempotent() {
        let tmp = std::env::temp_dir().join(format!("rifflab_test_idem_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let _lib1 = Library::init_at(tmp.clone()).expect("first init");
        let _lib2 = Library::init_at(tmp.clone()).expect("second init should also succeed");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_ensure_song_dir() {
        let tmp = std::env::temp_dir().join(format!("rifflab_test_song_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let lib = Library::init_at(tmp.clone()).unwrap();
        let song_dir = lib.ensure_song_dir("abc-123").unwrap();
        assert!(song_dir.is_dir());
        assert_eq!(song_dir, tmp.join("songs").join("abc-123"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_song_dir_path() {
        let tmp = std::env::temp_dir().join(format!("rifflab_test_path_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let lib = Library::init_at(tmp.clone()).unwrap();
        let path = lib.song_dir("my-song");
        assert_eq!(path, tmp.join("songs").join("my-song"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_list_songs_empty() {
        let tmp = std::env::temp_dir().join(format!("rifflab_test_list_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let lib = Library::init_at(tmp.clone()).unwrap();
        let songs = lib.list_songs().unwrap();
        assert!(songs.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_list_songs_with_entries() {
        let tmp = std::env::temp_dir().join(format!("rifflab_test_list2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let lib = Library::init_at(tmp.clone()).unwrap();
        lib.ensure_song_dir("song-a").unwrap();
        lib.ensure_song_dir("song-b").unwrap();

        let songs = lib.list_songs().unwrap();
        assert_eq!(songs, vec!["song-a", "song-b"]);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
