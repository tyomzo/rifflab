use super::types::{ChatHistory, ChatMessage};
use std::path::{Path, PathBuf};

/// Soft cap on retained messages. Older ones are dropped from the in-memory
/// history when saving — recent context is what the LLM needs.
pub const MAX_RETAINED: usize = 200;

/// Default location: `~/.config/rifflab/chat.json` (or platform equivalent).
pub fn default_history_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "rifflab")
        .map(|dirs| dirs.config_dir().join("chat.json"))
}

/// Load history from disk. Missing file or parse error → empty history.
pub fn load_history(path: &Path) -> ChatHistory {
    let Ok(bytes) = std::fs::read(path) else {
        return ChatHistory::default();
    };
    match serde_json::from_slice::<ChatHistory>(&bytes) {
        Ok(h) => h,
        Err(e) => {
            log::warn!("Failed to parse chat history at {}: {e}", path.display());
            ChatHistory::default()
        }
    }
}

/// Save history to disk. Caps message count at [`MAX_RETAINED`] (drops oldest).
pub fn save_history(path: &Path, messages: &[ChatMessage]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let trimmed: Vec<&ChatMessage> = if messages.len() > MAX_RETAINED {
        messages.iter().skip(messages.len() - MAX_RETAINED).collect()
    } else {
        messages.iter().collect()
    };
    let history = ChatHistory {
        messages: trimmed.into_iter().cloned().collect(),
    };
    let json = serde_json::to_vec_pretty(&history)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::Role;

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = std::env::temp_dir().join(format!(
            "rifflab_chat_history_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&tmp);

        let msgs = vec![
            ChatMessage::now(Role::User, "hi"),
            ChatMessage::now(Role::Assistant, "hello"),
        ];
        save_history(&tmp, &msgs).unwrap();
        let loaded = load_history(&tmp);
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].text, "hi");
        assert_eq!(loaded.messages[1].role, Role::Assistant);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let path = std::env::temp_dir().join("rifflab_chat_missing_xyz.json");
        let _ = std::fs::remove_file(&path);
        let h = load_history(&path);
        assert!(h.messages.is_empty());
    }

    #[test]
    fn save_trims_to_max_retained() {
        let tmp = std::env::temp_dir().join(format!(
            "rifflab_chat_trim_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&tmp);

        let msgs: Vec<ChatMessage> = (0..(MAX_RETAINED + 10))
            .map(|i| ChatMessage::now(Role::User, format!("msg {i}")))
            .collect();
        save_history(&tmp, &msgs).unwrap();
        let loaded = load_history(&tmp);
        assert_eq!(loaded.messages.len(), MAX_RETAINED);
        // Oldest dropped — first retained is msg 10.
        assert_eq!(loaded.messages[0].text, "msg 10");

        let _ = std::fs::remove_file(&tmp);
    }
}
