//! Failing tests for `parse_remove_args_from` + `run_local_remove`
//! (T12b, #818).
//!
//! RED: these functions don't exist yet.  They will be added to
//! `src/local_model_cli.rs` to support the v3 ModelManager's
//! "Remove" keybind (M → select → Remove).

use crate::local_model_cli::parse_remove_args_from;

fn os(s: &str) -> std::ffi::OsString {
    std::ffi::OsString::from(s)
}

#[test]
fn parse_remove_returns_none_when_no_remove_flags() {
    let args = vec![os("--install-local-mt-model"), os("/tmp/m.json")];
    let result = parse_remove_args_from(args).expect("must not error on no-remove args");
    assert!(result.is_none(), "no --remove-* flag must yield None");
}

#[test]
fn parse_remove_stt_only() {
    let args = vec![os("--remove-stt=tiny.en")];
    let result = parse_remove_args_from(args)
        .expect("must parse")
        .expect("must be Some when --remove-stt is present");
    assert!(result.stt.is_some(), "stt id must be set");
    assert!(result.mt.is_none());
    assert!(result.tts.is_none());
}

#[test]
fn parse_remove_mt_only() {
    let args = vec![os("--remove-mt=opus-mt-ja-vi")];
    let result = parse_remove_args_from(args)
        .expect("must parse")
        .expect("must be Some when --remove-mt is present");
    assert!(result.stt.is_none());
    assert!(result.mt.is_some(), "mt id must be set");
    assert!(result.tts.is_none());
}

#[test]
fn parse_remove_tts_only() {
    let args = vec![os("--remove-tts=supertonic-1")];
    let result = parse_remove_args_from(args)
        .expect("must parse")
        .expect("must be Some when --remove-tts is present");
    assert!(result.stt.is_none());
    assert!(result.mt.is_none());
    assert!(result.tts.is_some(), "tts id must be set");
}

#[test]
fn parse_remove_all_three_stages() {
    let args = vec![
        os("--remove-stt=tiny.en"),
        os("--remove-mt=opus-mt-ja-vi"),
        os("--remove-tts=supertonic-1"),
    ];
    let result = parse_remove_args_from(args)
        .expect("must parse")
        .expect("must be Some");
    assert!(result.stt.is_some());
    assert!(result.mt.is_some());
    assert!(result.tts.is_some());
}

#[test]
fn parse_remove_yes_flag() {
    let args = vec![os("--remove-stt=tiny.en"), os("--yes")];
    let result = parse_remove_args_from(args)
        .expect("must parse")
        .expect("must be Some");
    assert!(result.yes, "--yes must be honoured");
}

#[test]
fn parse_remove_rejects_unknown_stt_id() {
    let args = vec![os("--remove-stt=does-not-exist")];
    let err = parse_remove_args_from(args).expect_err("unknown id must error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("unknown") || msg.contains("does-not-exist"),
        "error must mention the unknown id: {msg}"
    );
}

#[test]
fn parse_remove_accepts_funasr_id() {
    // T7 added FunASR variants to ModelId.  The remove parser must
    // accept them too.
    let args = vec![os("--remove-stt=funasr-small")];
    let result = parse_remove_args_from(args)
        .expect("must parse")
        .expect("must be Some");
    assert!(result.stt.is_some());
}

// ---------------------------------------------------------------------------
// run_local_remove integration tests (dry-run + actual remove)
// ---------------------------------------------------------------------------

use crate::local_model_cli::{run_local_remove, LocalRemoveArgs};
use crate::providers::local::ModelId;
use std::sync::Mutex;

/// Serialise the run_local_remove tests that touch the
/// real `<cache_root>/stt/tiny.en` path.  Two tests in the
/// same process would race on the same filesystem location.
static CACHE_LOCK: Mutex<()> = Mutex::new(());

fn make_args(stt: Option<&str>, mt: Option<&str>, tts: Option<&str>, yes: bool) -> LocalRemoveArgs {
    LocalRemoveArgs {
        stt: stt.and_then(ModelId::parse),
        mt: mt.map(str::to_string),
        tts: tts.map(str::to_string),
        yes,
    }
}

#[test]
fn run_local_remove_with_no_stages_errors() {
    // The dispatcher only calls `run_local_remove` after
    // `parse_remove_args_from` returns `Some(_)` (so at least
    // one --remove-* was present).  But defence-in-depth: the
    // function must error if invoked with no targets.
    let args = make_args(None, None, None, true);
    let err = run_local_remove(&args).expect_err("must error with no stages");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("requires at least one") || msg.contains("stage"),
        "error must mention stages: {msg}"
    );
}

#[test]
fn run_local_remove_dry_run_for_missing_model_prints_would_remove() {
    // The dev box has no `ggml-tiny.en.bin` model under the
    // local model cache.  `run_local_remove` in dry-run mode
    // (yes=false) must print the path it WOULD remove and not
    // actually delete anything.
    let args = make_args(Some("tiny.en"), None, None, /* yes = */ false);
    run_local_remove(&args).expect("dry-run must not error on missing model");
}

#[test]
fn run_local_remove_with_yes_skips_missing_model_without_error() {
    // yes=true + missing model: skip with a "not found" line,
    // do not error.  Exercises the NotFound arm of the
    // fs::remove_dir_all error path.
    let args = make_args(Some("tiny.en"), None, None, /* yes = */ true);
    run_local_remove(&args).expect("missing-model + yes=true must not error");
}

#[test]
fn run_local_remove_with_mt_and_tts() {
    // Multi-stage: mt + tts, dry-run.  Exercises the targets
    // collection path in `run_local_remove`.
    let args = make_args(
        None,
        Some("opus-mt-ja-vi"),
        Some("supertonic-1"),
        /* yes = */ false,
    );
    run_local_remove(&args).expect("dry-run with mt+tts must succeed");
}

#[test]
fn run_local_remove_yes_actually_removes_existing_dir() {
    let _lock = CACHE_LOCK.lock().unwrap();
    // Create a stub directory at the expected cache path
    // `<model_cache_root>/stt/tiny.en/`, then call
    // `run_local_remove` with yes=true. The Ok(()) arm of
    // `fs::remove_dir_all` must be exercised, and the dir must
    // be gone afterwards.
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let _ = COUNTER.fetch_add(1, Ordering::SeqCst);

    // Find the cache root (same logic as `run_local_remove`).
    let cache_root =
        crate::providers::local::model_cache_dir().expect("cache root must be resolvable");
    // tiny.en is the ModelId used by the test — its display_name
    // is the path component under <cache_root>/stt/.
    let target_dir = cache_root.join("stt").join("tiny.en");
    std::fs::create_dir_all(&target_dir).expect("create stub dir");
    assert!(target_dir.is_dir(), "stub dir must exist before remove");

    let args = make_args(Some("tiny.en"), None, None, /* yes = */ true);
    run_local_remove(&args).expect("yes=true with existing dir must succeed");

    assert!(!target_dir.is_dir(), "stub dir must be gone after remove");
}

#[test]
fn run_local_remove_yes_on_path_that_is_a_file_errors() {
    let _lock = CACHE_LOCK.lock().unwrap();
    // The `fs::remove_dir_all` non-NotFound error path.  Create
    // a regular file at a path where `remove_dir_all` is called.
    let cache_root =
        crate::providers::local::model_cache_dir().expect("cache root must be resolvable");
    let target = cache_root.join("stt").join("tiny.en");
    // Save + remove any prior state.
    let backup = if target.exists() {
        let b = cache_root.join(format!("tiny.en.bak.{}", std::process::id()));
        std::fs::rename(&target, &b).ok();
        Some(b)
    } else {
        None
    };

    // Place a regular file at the target path. `fs::remove_dir_all`
    // on a file returns an error.
    std::fs::create_dir_all(target.parent().unwrap()).ok();
    std::fs::write(&target, b"not a dir").expect("write stub file");
    assert!(target.is_file(), "target must be a file");

    let args = make_args(Some("tiny.en"), None, None, /* yes = */ true);
    let result = run_local_remove(&args);

    // Restore prior state.
    std::fs::remove_file(&target).ok();
    if let Some(b) = backup {
        let _ = std::fs::rename(&b, &target);
    }

    // The call must error (the non-NotFound bail arm).
    result.expect_err("remove_dir_all on a file must error");
}
