use std::{ffi::OsString, path::PathBuf};

use crate::session_replay_cli::parse_replay_args_from;

#[test]
fn parse_replay_args_accepts_flag_and_path() {
    let parsed = parse_replay_args_from(vec![
        OsString::from("--replay-session"),
        OsString::from(r"C:\sessions\meeting.jsonl"),
    ])
    .unwrap()
    .expect("replay args should be detected");

    assert_eq!(parsed.path, PathBuf::from(r"C:\sessions\meeting.jsonl"));
}

#[test]
fn parse_replay_args_returns_none_when_flag_absent() {
    let result = parse_replay_args_from(vec![
        OsString::from("--export-session"),
        OsString::from("meeting.jsonl"),
    ])
    .unwrap();

    assert!(result.is_none(), "no replay flag -> must return None");
}

#[test]
fn parse_replay_args_requires_path_value() {
    let err = parse_replay_args_from(vec![OsString::from("--replay-session")])
        .expect_err("missing path should be rejected");
    assert!(
        err.to_string().contains("--replay-session"),
        "error message must name the flag"
    );
}

#[test]
fn parse_replay_args_rejects_another_flag_as_value() {
    let err = parse_replay_args_from(vec![
        OsString::from("--replay-session"),
        OsString::from("--other-flag"),
    ])
    .expect_err("another flag as value must be rejected");
    assert!(err.to_string().contains("--replay-session"));
}

#[test]
fn replay_bypass_not_triggered_with_no_args() {
    let result = parse_replay_args_from(std::iter::empty::<OsString>()).unwrap();
    assert!(
        result.is_none(),
        "no args must not trigger replay mode; audio/provider startup proceeds normally"
    );
}
