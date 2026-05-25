//! Evidence builder for the TEST-01 deterministic simulation harness.
//!
//! Produces a JSON document that matches
//! `verification-evidence/test/TEST-01-evidence-schema.json`. The
//! builder is intentionally typed: required fields are constructor
//! arguments; optional fields use `with_*` setters; the build step
//! returns a fully-formed [`serde_json::Value`] ready to be persisted
//! to `verification-evidence/test/runs/<run_id>.json` by the caller.
//!
//! The builder does *not* read wall-clock time or generate UUIDs by
//! itself — tests pass deterministic stand-ins so evidence comparison
//! is reproducible.

use serde_json::{json, Value};

/// Harness ladder level. Mirrors the `level.enum` in the JSON schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessLevel {
    /// File-source replayer (wave 1).
    L1,
    /// Provider mock with latency / error injection.
    L2,
    /// PTY / TUI golden-frame recorder.
    L3,
    /// Virtual-mic PCM roundtrip.
    L4,
}

impl HarnessLevel {
    /// Wire-format string used in the evidence JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            HarnessLevel::L1 => "L1",
            HarnessLevel::L2 => "L2",
            HarnessLevel::L3 => "L3",
            HarnessLevel::L4 => "L4",
        }
    }
}

/// Run-status verdict. Mirrors the `result.status.enum` in the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// All assertions held.
    Pass,
    /// At least one assertion failed; failure list MUST be populated.
    Fail,
    /// Harness itself errored before evaluating assertions.
    Error,
}

impl RunStatus {
    /// Wire-format string used in the evidence JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Pass => "pass",
            RunStatus::Fail => "fail",
            RunStatus::Error => "error",
        }
    }
}

/// Fixture identity required by the `fixture` object in the schema.
#[derive(Debug, Clone)]
pub struct FixtureInfo {
    /// Repository-relative path to the source fixture (a WAV file or a
    /// virtual identifier for in-memory scripted feeders).
    pub path: String,
    /// Total decoded PCM samples in the fixture (sum across all chunks
    /// for scripted feeders).
    pub total_samples: u64,
    /// Optional hex-encoded SHA-256 of the fixture bytes.
    pub sha256: Option<String>,
}

/// Counters required by the `result` object in the schema.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResultCounters {
    /// Number of audio chunks emitted by the source during the run.
    pub chunks_emitted: u64,
    /// Number of PCM samples emitted across all chunks.
    pub samples_emitted: u64,
    /// Number of complete loops through the fixture (`0` for scripted
    /// feeders that play once and stop).
    pub loops_completed: u64,
    /// Optional wall-clock duration in milliseconds. The harness uses
    /// a [`crate::sim::clock::FakeClock`]; tests typically copy
    /// `clock.elapsed().as_millis()` here.
    pub wall_clock_ms: Option<u64>,
}

/// Fluent builder for one TEST-01 evidence document.
#[derive(Debug, Clone)]
pub struct EvidenceBuilder {
    schema_version: String,
    level: HarnessLevel,
    run_id: String,
    started_at: String,
    finished_at: Option<String>,
    git_sha: Option<String>,
    fixture: Option<FixtureInfo>,
    metrics: Option<Value>,
    notes: Option<String>,
}

impl EvidenceBuilder {
    /// Begin building an evidence document for `level`.
    ///
    /// `run_id` should be a globally-unique opaque token (e.g. a UUID
    /// v4). Tests typically pass a deterministic constant.
    /// `started_at` must be an RFC 3339 UTC timestamp string.
    pub fn new(
        level: HarnessLevel,
        run_id: impl Into<String>,
        started_at: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: "1".to_string(),
            level,
            run_id: run_id.into(),
            started_at: started_at.into(),
            finished_at: None,
            git_sha: None,
            fixture: None,
            metrics: None,
            notes: None,
        }
    }

    /// Override the schema version (default `"1"`).
    pub fn with_schema_version(mut self, v: impl Into<String>) -> Self {
        self.schema_version = v.into();
        self
    }

    /// Set the fixture descriptor. Required before [`Self::build`].
    pub fn with_fixture(mut self, fixture: FixtureInfo) -> Self {
        self.fixture = Some(fixture);
        self
    }

    /// Set the optional finished-at timestamp (RFC 3339 UTC).
    pub fn with_finished_at(mut self, ts: impl Into<String>) -> Self {
        self.finished_at = Some(ts.into());
        self
    }

    /// Set the optional commit SHA the harness was built from.
    pub fn with_git_sha(mut self, sha: impl Into<String>) -> Self {
        self.git_sha = Some(sha.into());
        self
    }

    /// Set the optional level-specific metrics payload (open-shape).
    pub fn with_metrics(mut self, metrics: Value) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Set the optional free-form operator notes.
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    /// Materialise the evidence document.
    ///
    /// Returns an error string if any required field was omitted. The
    /// builder never panics — schema-required-field omissions are
    /// caller bugs that the test will surface as a normal `Result`.
    pub fn build(
        self,
        status: RunStatus,
        counters: ResultCounters,
        failures: Vec<String>,
    ) -> Result<Value, String> {
        let fixture = self
            .fixture
            .ok_or_else(|| "EvidenceBuilder: fixture is required".to_string())?;

        let mut fixture_obj = json!({
            "path": fixture.path,
            "sample_rate_hz": 16_000,
            "channels": 1,
            "bit_depth": 16,
            "total_samples": fixture.total_samples,
        });
        if let Some(sha) = fixture.sha256 {
            if let Some(map) = fixture_obj.as_object_mut() {
                map.insert("sha256".to_string(), Value::String(sha));
            }
        }

        let mut result_obj = json!({
            "status": status.as_str(),
            "chunks_emitted": counters.chunks_emitted,
            "samples_emitted": counters.samples_emitted,
            "loops_completed": counters.loops_completed,
        });
        if let Some(ms) = counters.wall_clock_ms {
            if let Some(map) = result_obj.as_object_mut() {
                map.insert("wall_clock_ms".to_string(), Value::from(ms));
            }
        }
        if !failures.is_empty() {
            if let Some(map) = result_obj.as_object_mut() {
                map.insert(
                    "failures".to_string(),
                    Value::Array(failures.into_iter().map(Value::String).collect()),
                );
            }
        }

        let mut doc = json!({
            "schema_version": self.schema_version,
            "harness_id": "TEST-01",
            "level": self.level.as_str(),
            "run_id": self.run_id,
            "started_at": self.started_at,
            "fixture": fixture_obj,
            "result": result_obj,
        });

        if let Some(map) = doc.as_object_mut() {
            if let Some(ts) = self.finished_at {
                map.insert("finished_at".to_string(), Value::String(ts));
            }
            if let Some(sha) = self.git_sha {
                map.insert("git_sha".to_string(), Value::String(sha));
            }
            if let Some(metrics) = self.metrics {
                map.insert("metrics".to_string(), metrics);
            }
            if let Some(notes) = self.notes {
                map.insert("notes".to_string(), Value::String(notes));
            }
        }

        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> FixtureInfo {
        FixtureInfo {
            path: "tests/sim/in-memory".into(),
            total_samples: 32_000,
            sha256: None,
        }
    }

    #[test]
    fn level_strings_match_schema_enum() {
        assert_eq!(HarnessLevel::L1.as_str(), "L1");
        assert_eq!(HarnessLevel::L2.as_str(), "L2");
        assert_eq!(HarnessLevel::L3.as_str(), "L3");
        assert_eq!(HarnessLevel::L4.as_str(), "L4");
    }

    #[test]
    fn build_emits_required_top_level_fields() {
        let doc = EvidenceBuilder::new(
            HarnessLevel::L2,
            "00000000-0000-0000-0000-000000000002",
            "2026-01-01T00:00:00Z",
        )
        .with_fixture(fixture())
        .build(
            RunStatus::Pass,
            ResultCounters {
                chunks_emitted: 4,
                samples_emitted: 32_000,
                loops_completed: 0,
                wall_clock_ms: Some(120),
            },
            vec![],
        )
        .expect("builds");

        for f in [
            "schema_version",
            "harness_id",
            "level",
            "run_id",
            "started_at",
            "fixture",
            "result",
        ] {
            assert!(doc.get(f).is_some(), "missing top-level field {f}");
        }
        assert_eq!(doc["harness_id"], "TEST-01");
        assert_eq!(doc["level"], "L2");
        assert_eq!(doc["result"]["status"], "pass");
        assert_eq!(doc["result"]["chunks_emitted"], 4);
        assert_eq!(doc["result"]["wall_clock_ms"], 120);
        assert_eq!(doc["fixture"]["sample_rate_hz"], 16_000);
        assert_eq!(doc["fixture"]["channels"], 1);
        assert_eq!(doc["fixture"]["bit_depth"], 16);
    }

    #[test]
    fn build_fails_without_fixture() {
        let err = EvidenceBuilder::new(HarnessLevel::L3, "rid", "2026-01-01T00:00:00Z")
            .build(RunStatus::Pass, ResultCounters::default(), vec![])
            .unwrap_err();
        assert!(err.contains("fixture is required"));
    }

    #[test]
    fn failures_list_is_emitted_only_when_non_empty() {
        let pass_doc = EvidenceBuilder::new(HarnessLevel::L4, "rid", "2026-01-01T00:00:00Z")
            .with_fixture(fixture())
            .build(RunStatus::Pass, ResultCounters::default(), vec![])
            .expect("builds");
        assert!(pass_doc["result"].get("failures").is_none());

        let fail_doc = EvidenceBuilder::new(HarnessLevel::L4, "rid", "2026-01-01T00:00:00Z")
            .with_fixture(fixture())
            .build(
                RunStatus::Fail,
                ResultCounters::default(),
                vec!["assertion x".into()],
            )
            .expect("builds");
        assert_eq!(fail_doc["result"]["failures"][0], "assertion x");
    }
}
