use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

use anyhow::{bail, Context, Result};

/// Arguments for `--replay-session`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplayArgs {
    /// Path to the session JSONL file to replay.
    pub(crate) path: PathBuf,
}

/// Parse `--replay-session <path>` from an argument iterator.
pub(crate) fn parse_replay_args_from<I>(args: I) -> Result<Option<ReplayArgs>>
where
    I: IntoIterator<Item = OsString>,
{
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == OsStr::new("--replay-session") {
            let value = iter
                .next()
                .with_context(|| "missing value after --replay-session")?;
            if value.to_string_lossy().starts_with("--") {
                bail!("missing value after --replay-session");
            }
            return Ok(Some(ReplayArgs {
                path: PathBuf::from(value),
            }));
        }
    }
    Ok(None)
}
