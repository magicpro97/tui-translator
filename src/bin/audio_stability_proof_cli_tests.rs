use super::{parse_args_from, Args, CliAction, DEFAULT_DURATION_SECS};

fn args(parts: &[&str]) -> Vec<String> {
    std::iter::once("audio_stability_proof".to_string())
        .chain(parts.iter().map(|part| (*part).to_string()))
        .collect()
}

#[test]
fn parse_args_accepts_explicit_values() {
    let parsed = parse_args_from(args(&["--duration-secs", "30", "--out", "proof.json"]));
    assert_eq!(
        parsed,
        Ok(CliAction::Run(Args {
            duration_secs: 30,
            out: Some("proof.json".into()),
        }))
    );
}

#[test]
fn parse_args_uses_defaults_when_flags_absent() {
    let parsed = parse_args_from(args(&[]));
    assert_eq!(
        parsed,
        Ok(CliAction::Run(Args {
            duration_secs: DEFAULT_DURATION_SECS,
            out: None,
        }))
    );
}

#[test]
fn parse_args_rejects_invalid_duration() {
    let parsed = parse_args_from(args(&["--duration-secs", "abc"]));
    assert_eq!(parsed, Err("invalid value for --duration-secs: abc".into()));
}

#[test]
fn parse_args_rejects_missing_values() {
    assert_eq!(
        parse_args_from(args(&["--duration-secs"])),
        Err("missing value for --duration-secs".into())
    );
    assert_eq!(
        parse_args_from(args(&["--out"])),
        Err("missing value for --out".into())
    );
}

#[test]
fn parse_args_recognises_help() {
    assert_eq!(parse_args_from(args(&["--help"])), Ok(CliAction::Help));
}
