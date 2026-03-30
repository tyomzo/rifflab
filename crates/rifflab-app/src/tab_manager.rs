//! Tab document management: background parsing, document state, and view state.

use crate::tab_view;
use rifflab_tab::model::TabDocument;

/// Manages tab document state and background parsing.
pub struct TabManager {
    /// The loaded tab document.
    pub document: Option<TabDocument>,
    /// View state (scroll, zoom, selection, undo/redo).
    pub view_state: tab_view::TabViewState,
    /// Pending tab parse result from background thread.
    pub parse_rx: Option<std::sync::mpsc::Receiver<TabDocument>>,
    /// Log messages from background tab parsing.
    pub parse_log_rx: Option<std::sync::mpsc::Receiver<String>>,
}

impl TabManager {
    pub fn new() -> Self {
        Self {
            document: None,
            view_state: tab_view::TabViewState::default(),
            parse_rx: None,
            parse_log_rx: None,
        }
    }

    /// Start parsing a tab file on a background thread.
    pub fn start_parse(&mut self, text: String, title: String) {
        let (tab_tx, tab_rx) = std::sync::mpsc::channel();
        let (log_tx, log_rx) = std::sync::mpsc::channel();
        self.parse_rx = Some(tab_rx);
        self.parse_log_rx = Some(log_rx);

        std::thread::spawn(move || {
            let notes = rifflab_tab::ascii_llm::parse_ascii_tab_llm(&text, Some(&log_tx));
            if notes.is_empty() {
                let _ = log_tx.send("No notes found in file".into());
                return;
            }
            let mut tab = TabDocument::new(title);
            tab.tuning = rifflab_tab::ascii::detect_tuning(&text);
            if let Some(bpm) = rifflab_tab::ascii::detect_tempo(&text) {
                tab.tempo = rifflab_tab::model::TempoMap::constant(bpm);
            }
            tab.notes = notes;
            tab.assign_timing();
            tab.generate_measures();
            let tuning_name = if tab.tuning == rifflab_tab::model::DROP_D_TUNING {
                "Drop D"
            } else {
                "Standard"
            };
            let _ = log_tx.send(format!(
                "Parsed {} notes, {} tuning, {:.0} BPM",
                tab.notes.len(),
                tuning_name,
                tab.tempo.initial_bpm
            ));
            let _ = tab_tx.send(tab);
        });
    }

    /// Poll background parsing. Returns log messages and whether a document was received.
    pub fn poll(&mut self) -> (Vec<String>, bool) {
        let mut logs = Vec::new();

        if let Some(ref rx) = self.parse_log_rx {
            while let Ok(msg) = rx.try_recv() {
                logs.push(msg);
            }
        }

        let mut got_document = false;
        if let Some(ref rx) = self.parse_rx {
            if let Ok(tab) = rx.try_recv() {
                logs.push(format!(
                    "Tab ready: {} ({} notes)",
                    tab.title,
                    tab.notes.len()
                ));
                self.document = Some(tab);
                self.parse_rx = None;
                self.parse_log_rx = None;
                got_document = true;
            }
        }

        (logs, got_document)
    }

    /// Whether a parse is currently in flight.
    pub fn is_parsing(&self) -> bool {
        self.parse_rx.is_some()
    }
}
