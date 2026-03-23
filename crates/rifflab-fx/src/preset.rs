use rifflab_core::preset::EffectPreset;
use std::path::Path;

/// Save a preset to a TOML file.
pub fn save_preset(preset: &EffectPreset, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let toml_str = toml::to_string_pretty(preset)?;
    std::fs::write(path, toml_str)?;
    Ok(())
}

/// Load a preset from a TOML file.
pub fn load_preset(path: &Path) -> Result<EffectPreset, Box<dyn std::error::Error>> {
    let toml_str = std::fs::read_to_string(path)?;
    let preset: EffectPreset = toml::from_str(&toml_str)?;
    Ok(preset)
}
