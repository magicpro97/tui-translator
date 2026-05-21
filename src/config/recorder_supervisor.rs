//! HC-03: Session recorder change classifier.
//!
//! Classifies `session_store` config changes so the caller can decide whether
//! to trigger a recorder path switch (seal current JSONL, open new one) or
//! treat the change as a no-op.
//!
//! The actual `SessionRecorder::seal_and_reopen` method lives in
//! `crate::session` to keep session-file I/O out of this module.

#![allow(dead_code)]

use super::AppConfig;

/// Result of classifying a `session_store` config change.
#[derive(Debug, PartialEq, Eq)]
pub enum RecorderChangeOutcome {
    /// No recorder-relevant fields changed; hot-reload can proceed unchanged.
    Unchanged,
    /// The recorder should seal the current JSONL and open a new one.
    ///
    /// Required when `directory` changes: old segments stay in the old
    /// directory; new segments are written to the new directory under the
    /// same session-id.
    NeedsPathSwitch {
        /// Human-readable description of what changed.
        reason: String,
    },
    /// The new config is invalid for the recorder.
    Rejected {
        /// Human-readable description.  Safe to surface in UI.
        reason: String,
    },
}

/// Compare old and new [`AppConfig`] and return a typed [`RecorderChangeOutcome`].
///
/// Only `session_store.directory` triggers a path switch in this foundation
/// classifier. Other `session_store` fields do not imply a new JSONL path;
/// applying them to an already-running writer is a separate supervisor concern.
pub fn classify_recorder_change(old: &AppConfig, new: &AppConfig) -> RecorderChangeOutcome {
    let old_dir = old.session_store.directory.as_deref().unwrap_or("");
    let new_dir = new.session_store.directory.as_deref().unwrap_or("");

    if let Some(dir) = &new.session_store.directory {
        if dir.trim().is_empty() {
            return RecorderChangeOutcome::Rejected {
                reason: "session_store.directory is present but empty".to_string(),
            };
        }
    }

    if old_dir == new_dir {
        return RecorderChangeOutcome::Unchanged;
    }

    RecorderChangeOutcome::NeedsPathSwitch {
        reason: format!(
            "session_store.directory changed from {:?} to {:?}; recorder path switch required",
            old.session_store.directory, new.session_store.directory,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> AppConfig {
        AppConfig::default()
    }

    #[test]
    fn unchanged_when_directory_not_set() {
        assert_eq!(
            classify_recorder_change(&base(), &base()),
            RecorderChangeOutcome::Unchanged
        );
    }

    #[test]
    fn unchanged_when_directory_same() {
        let mut old = base();
        old.session_store.directory = Some("C:\\sessions\\recordings".to_string());
        let new = old.clone();
        assert_eq!(
            classify_recorder_change(&old, &new),
            RecorderChangeOutcome::Unchanged
        );
    }

    #[test]
    fn needs_path_switch_when_directory_changes() {
        let mut old = base();
        old.session_store.directory = Some("C:\\sessions\\old".to_string());
        let mut new = base();
        new.session_store.directory = Some("C:\\sessions\\new".to_string());
        assert!(matches!(
            classify_recorder_change(&old, &new),
            RecorderChangeOutcome::NeedsPathSwitch { .. }
        ));
    }

    #[test]
    fn needs_path_switch_when_directory_set_from_none() {
        let old = base();
        let mut new = base();
        new.session_store.directory = Some("D:\\recordings".to_string());
        assert!(matches!(
            classify_recorder_change(&old, &new),
            RecorderChangeOutcome::NeedsPathSwitch { .. }
        ));
    }

    #[test]
    fn needs_path_switch_when_directory_cleared_to_none() {
        let mut old = base();
        old.session_store.directory = Some("D:\\recordings".to_string());
        let new = base();
        assert!(matches!(
            classify_recorder_change(&old, &new),
            RecorderChangeOutcome::NeedsPathSwitch { .. }
        ));
    }

    #[test]
    fn rejected_when_directory_whitespace_only() {
        let old = base();
        let mut new = base();
        new.session_store.directory = Some("   ".to_string());
        assert!(matches!(
            classify_recorder_change(&old, &new),
            RecorderChangeOutcome::Rejected { .. }
        ));
    }

    #[test]
    fn rejected_when_empty_directory_is_same_as_absent_after_normalization() {
        let mut old = base();
        old.session_store.directory = None;
        let mut new = base();
        new.session_store.directory = Some(String::new());
        assert!(matches!(
            classify_recorder_change(&old, &new),
            RecorderChangeOutcome::Rejected { .. }
        ));
    }

    #[test]
    fn reason_string_mentions_both_directories() {
        let mut old = base();
        old.session_store.directory = Some("olddir".to_string());
        let mut new = base();
        new.session_store.directory = Some("newdir".to_string());
        if let RecorderChangeOutcome::NeedsPathSwitch { reason } =
            classify_recorder_change(&old, &new)
        {
            assert!(reason.contains("olddir"));
            assert!(reason.contains("newdir"));
        } else {
            panic!("expected NeedsPathSwitch");
        }
    }
}
