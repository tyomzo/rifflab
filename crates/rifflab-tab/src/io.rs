use crate::model::TabDocument;
use std::path::Path;

/// Save a TabDocument as a `.rltab` JSON file.
pub fn save_tab(tab: &TabDocument, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(tab)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Load a TabDocument from a `.rltab` JSON file.
pub fn load_tab(path: &Path) -> Result<TabDocument, Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(path)?;
    let tab: TabDocument = serde_json::from_str(&json)?;
    Ok(tab)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    #[test]
    fn round_trip_serialization() {
        let mut tab = TabDocument::new("Test Song");
        tab.artist = Some("Test Artist".into());
        tab.tuning = STANDARD_TUNING;
        tab.tempo = TempoMap::constant(120.0);

        let mut note = TabNote::new(1, 5, NoteSource::AsciiParse);
        note.time_secs = 1.0;
        note.duration_secs = 0.5;
        note.technique = Some(Technique::HammerOn);
        tab.notes.push(note);

        let json = serde_json::to_string_pretty(&tab).unwrap();
        let loaded: TabDocument = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.title, "Test Song");
        assert_eq!(loaded.artist.as_deref(), Some("Test Artist"));
        assert_eq!(loaded.notes.len(), 1);
        assert_eq!(loaded.notes[0].string, 1);
        assert_eq!(loaded.notes[0].fret, 5);
        assert_eq!(loaded.notes[0].technique, Some(Technique::HammerOn));
        assert_eq!(loaded.tuning, STANDARD_TUNING);
    }

    #[test]
    fn save_load_file() {
        let tab = TabDocument::new("File Test");
        let dir = std::env::temp_dir().join("rifflab_test_tab");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.rltab");

        save_tab(&tab, &path).unwrap();
        let loaded = load_tab(&path).unwrap();
        assert_eq!(loaded.title, "File Test");

        let _ = std::fs::remove_file(&path);
    }
}
