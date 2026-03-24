use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level application configuration, persisted as TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub audio: AudioSection,
    pub library: LibrarySection,
    pub ui: UiSection,
}

/// Audio engine configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSection {
    /// Audio backend: "auto", "jack", "alsa"
    pub backend: String,
    /// Buffer size in samples.
    pub buffer_size: u32,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Input gain (0.0 - 2.0).
    pub input_gain: f32,
    /// Master volume (0.0 - 1.0).
    pub master_volume: f32,
    /// Output device name (empty = system default).
    #[serde(default)]
    pub output_device: String,
    /// Input device name (empty = system default).
    #[serde(default)]
    pub input_device: String,
}

/// Library paths configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySection {
    /// Override for the library root. If empty, uses XDG default.
    pub root_override: String,
}

/// UI preferences section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSection {
    /// Default zoom level (frames per pixel).
    pub default_zoom: f64,
    /// Whether the view auto-follows the playhead.
    pub auto_follow: bool,
    /// Whether the bottom drawer starts open.
    pub drawer_open: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            audio: AudioSection {
                backend: "auto".to_string(),
                buffer_size: 128,
                sample_rate: 48000,
                input_gain: 1.0,
                master_volume: 1.0,
                output_device: String::new(),
                input_device: String::new(),
            },
            library: LibrarySection {
                root_override: String::new(),
            },
            ui: UiSection {
                default_zoom: 512.0,
                auto_follow: true,
                drawer_open: true,
            },
        }
    }
}

impl AppConfig {
    /// Load config from the default location (~/.config/rifflab/config.toml).
    /// If the file doesn't exist, writes default config and returns it.
    pub fn load_or_create_default() -> Result<Self> {
        let path = default_config_path()
            .context("Could not determine config directory")?;
        Self::load_or_create(&path)
    }

    /// Load config from a specific path.
    /// If the file doesn't exist, writes default config and returns it.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            let config = Self::default();
            config.save(path)?;
            log::info!("Created default config at {}", path.display());
            Ok(config)
        }
    }

    /// Load config from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config {}", path.display()))?;
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config {}", path.display()))?;
        log::info!("Loaded config from {}", path.display());
        Ok(config)
    }

    /// Save config to a TOML file.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create config dir {}", parent.display()))?;
            }
        }
        let contents = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        std::fs::write(path, &contents)
            .with_context(|| format!("Failed to write config {}", path.display()))?;
        Ok(())
    }

    /// Apply CLI argument overrides to this config.
    pub fn apply_cli_overrides(&mut self, backend: &str, buffer_size: u32) {
        if backend != "auto" {
            self.audio.backend = backend.to_string();
        }
        if buffer_size != self.audio.buffer_size {
            self.audio.buffer_size = buffer_size;
        }
    }

    /// Convert this config's audio section into the engine AudioConfig type.
    pub fn to_audio_config(&self) -> rifflab_core::audio::AudioConfig {
        let sample_rate = rifflab_core::audio::SampleRate::from_u32(self.audio.sample_rate)
            .unwrap_or_default();
        let buffer_size = match self.audio.buffer_size {
            64 => rifflab_core::audio::BufferSize::B64,
            128 => rifflab_core::audio::BufferSize::B128,
            512 => rifflab_core::audio::BufferSize::B512,
            1024 => rifflab_core::audio::BufferSize::B1024,
            _ => rifflab_core::audio::BufferSize::B256,
        };
        rifflab_core::audio::AudioConfig {
            sample_rate,
            buffer_size,
            input_channels: 2,
            output_channels: 2,
        }
    }
}

/// Determine the default config file path using XDG conventions.
pub fn default_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "rifflab")
        .map(|dirs| dirs.config_dir().join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_roundtrip() {
        let tmp = std::env::temp_dir().join(format!(
            "rifflab_config_test_{}.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&tmp);

        let config = AppConfig::default();
        config.save(&tmp).expect("save should succeed");

        let loaded = AppConfig::load(&tmp).expect("load should succeed");
        assert_eq!(loaded.audio.backend, "auto");
        assert_eq!(loaded.audio.buffer_size, 256);
        assert_eq!(loaded.audio.sample_rate, 48000);
        assert_eq!(loaded.ui.auto_follow, true);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_load_or_create_creates_file() {
        let tmp = std::env::temp_dir().join(format!(
            "rifflab_config_test_create_{}.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&tmp);

        assert!(!tmp.exists());
        let config = AppConfig::load_or_create(&tmp).expect("should create default");
        assert!(tmp.exists());
        assert_eq!(config.audio.buffer_size, 256);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_cli_overrides() {
        let mut config = AppConfig::default();
        config.apply_cli_overrides("jack", 512);
        assert_eq!(config.audio.backend, "jack");
        assert_eq!(config.audio.buffer_size, 512);
    }

    #[test]
    fn test_cli_overrides_auto_no_change() {
        let mut config = AppConfig::default();
        let original_backend = config.audio.backend.clone();
        config.apply_cli_overrides("auto", 256);
        // "auto" should not override
        assert_eq!(config.audio.backend, original_backend);
    }

    #[test]
    fn test_to_audio_config() {
        let config = AppConfig::default();
        let ac = config.to_audio_config();
        assert_eq!(ac.sample_rate.as_u32(), 48000);
        assert_eq!(ac.buffer_size.as_usize(), 256);
    }

    #[test]
    fn test_custom_values_roundtrip() {
        let tmp = std::env::temp_dir().join(format!(
            "rifflab_config_custom_{}.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&tmp);

        let mut config = AppConfig::default();
        config.audio.sample_rate = 44100;
        config.audio.buffer_size = 128;
        config.audio.backend = "jack".to_string();
        config.ui.default_zoom = 1024.0;
        config.save(&tmp).unwrap();

        let loaded = AppConfig::load(&tmp).unwrap();
        assert_eq!(loaded.audio.sample_rate, 44100);
        assert_eq!(loaded.audio.buffer_size, 128);
        assert_eq!(loaded.audio.backend, "jack");
        assert_eq!(loaded.ui.default_zoom, 1024.0);

        let _ = std::fs::remove_file(&tmp);
    }
}
