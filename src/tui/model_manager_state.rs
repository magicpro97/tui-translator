//! TUI state for the ModelManager overlay (T9, #815).
//!
//! Owns:
//! * the currently-active tab (one of three: Whisper, FunASR, History),
//! * the per-tab cursor (selected model index, clamped to the
//!   tab's model count),
//! * the catalogs (a list of `Entry { label, ... }` per tab) used
//!   by T10's `render_model_manager` to draw the list pane.
//!
//! `ModelManagerState` is a value type: `Clone + Copy + Default +
//! Send + Sync`. The orchestrator (T9 caller) owns the `AppState`
//! which embeds this struct.
//!
//! The 3 tab catalogs are static for v3: the Whisper tab mirrors
//! the 8 built-in Whisper variants from the local-STT manifest
//! (T5, src/providers/local/manifest.rs), the FunASR tab mirrors
//! the 3 new FunASR variants (T5), and the History tab starts
//! empty (T11 wires the history log).

use super::model_manager_tokens::ModelManagerTab;

/// A single row in a tab's catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Display label, e.g. `"ggml-medium.bin"`.
    pub label: &'static str,
    /// Stable kind tag for downstream consumers (T10 renderer,
    /// T11 history log, T12 backend selection). For v3 the value
    /// is the `ModelId::ALL_*` short name (`"Whisper"`, `"FunAsr"`).
    pub kind: &'static str,
}

/// Tab catalog (per-tab).
#[derive(Debug, Clone, Copy)]
struct TabCatalog {
    entries: &'static [Entry],
}

const WHISPER_CATALOG: &[Entry] = &[
    Entry {
        label: "ggml-tiny.en.bin",
        kind: "Whisper",
    },
    Entry {
        label: "ggml-tiny.bin",
        kind: "Whisper",
    },
    Entry {
        label: "ggml-base.en.bin",
        kind: "Whisper",
    },
    Entry {
        label: "ggml-base.bin",
        kind: "Whisper",
    },
    Entry {
        label: "ggml-small.en.bin",
        kind: "Whisper",
    },
    Entry {
        label: "ggml-small.bin",
        kind: "Whisper",
    },
    Entry {
        label: "ggml-medium.en.bin",
        kind: "Whisper",
    },
    Entry {
        label: "ggml-medium.bin",
        kind: "Whisper",
    },
];

const FUNASR_CATALOG: &[Entry] = &[
    Entry {
        label: "sherpa-onnx-funasr-small",
        kind: "FunAsr",
    },
    Entry {
        label: "sherpa-onnx-funasr-medium",
        kind: "FunAsr",
    },
    Entry {
        label: "sherpa-onnx-funasr-large",
        kind: "FunAsr",
    },
];

const HISTORY_CATALOG: &[Entry] = &[];

const TABS: &[TabCatalog] = &[
    TabCatalog {
        entries: WHISPER_CATALOG,
    },
    TabCatalog {
        entries: FUNASR_CATALOG,
    },
    TabCatalog {
        entries: HISTORY_CATALOG,
    },
];

/// 3-tab state machine for the ModelManager overlay.
///
/// `selected_index` is *per-tab* via `selected_per_tab`, so switching
/// tabs does not lose the user's cursor position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelManagerState {
    current_tab: ModelManagerTab,
    /// One cursor per tab (3 entries). Index 0 = Whisper, 1 = FunASR,
    /// 2 = History.
    selected_per_tab: [usize; 3],
}

impl Default for ModelManagerState {
    fn default() -> Self {
        Self {
            current_tab: ModelManagerTab::Whisper,
            selected_per_tab: [0, 0, 0],
        }
    }
}

impl ModelManagerState {
    /// Currently-active tab.
    pub fn current_tab(&self) -> ModelManagerTab {
        self.current_tab
    }

    /// Selected index within the current tab.
    pub fn selected_index(&self) -> usize {
        self.selected_per_tab[self.current_tab.tab_index()]
    }

    /// Number of models in the current tab.
    pub fn model_count(&self) -> usize {
        self.model_count_for_tab(self.current_tab)
    }

    /// Number of models in a specific tab.
    pub fn model_count_for_tab(&self, tab: ModelManagerTab) -> usize {
        TABS[tab.tab_index()].entries.len()
    }

    /// Label for a specific `(tab, index)`. Returns `None` if
    /// `index >= count`.
    pub fn model_label(&self, tab: ModelManagerTab, idx: usize) -> Option<&'static str> {
        let entry = TABS[tab.tab_index()].entries.get(idx)?;
        Some(entry.label)
    }

    /// Kind tag for a specific `(tab, index)`. Returns `None` if
    /// `index >= count`.
    pub fn model_kind(&self, tab: ModelManagerTab, idx: usize) -> Option<&'static str> {
        let entry = TABS[tab.tab_index()].entries.get(idx)?;
        Some(entry.kind)
    }

    /// Move to the next tab (cycles back to the first after the last).
    /// Resets the per-tab cursor to 0.
    pub fn next_tab(&mut self) {
        self.current_tab = self.current_tab.next();
    }

    /// Move to the previous tab (cycles forward to the last before
    /// the first). Resets the per-tab cursor to 0.
    pub fn prev_tab(&mut self) {
        self.current_tab = self.current_tab.previous();
    }

    /// Jump directly to a specific tab. Resets the per-tab cursor
    /// to 0.
    pub fn select_tab(&mut self, tab: ModelManagerTab) {
        if self.current_tab != tab {
            self.current_tab = tab;
        }
    }

    /// Select the next model within the current tab. Returns `true`
    /// if the cursor advanced, `false` if it was already at the
    /// last entry (no wrap).
    pub fn select_next(&mut self) -> bool {
        let tab = self.current_tab;
        let count = self.model_count_for_tab(tab);
        if count == 0 {
            return false;
        }
        let i = self.selected_per_tab[tab.tab_index()];
        if i + 1 < count {
            self.selected_per_tab[tab.tab_index()] = i + 1;
            true
        } else {
            false
        }
    }

    /// Select the previous model within the current tab. Returns
    /// `true` if the cursor moved, `false` if it was already at 0.
    pub fn select_prev(&mut self) -> bool {
        let tab = self.current_tab;
        let i = self.selected_per_tab[tab.tab_index()];
        if i > 0 {
            self.selected_per_tab[tab.tab_index()] = i - 1;
            true
        } else {
            false
        }
    }

    /// Set the selected index directly. Clamps to a valid range
    /// for the current tab.
    pub fn select_index(&mut self, idx: usize) {
        let tab = self.current_tab;
        let count = self.model_count_for_tab(tab);
        self.selected_per_tab[tab.tab_index()] = if count == 0 { 0 } else { idx.min(count - 1) };
    }
}

trait TabIndex {
    fn tab_index(self) -> usize;
}
impl TabIndex for ModelManagerTab {
    fn tab_index(self) -> usize {
        match self {
            ModelManagerTab::Whisper => 0,
            ModelManagerTab::FunAsr => 1,
            ModelManagerTab::History => 2,
        }
    }
}

#[cfg(test)]
#[path = "model_manager_state_tests.rs"]
mod tests;
