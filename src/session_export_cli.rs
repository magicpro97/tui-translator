use anyhow::{bail, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write as IoWrite},
    path::PathBuf,
};

use crate::session;

/// Output format for session transcript export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionExportFormat {
    Srt,
    Txt,
}

impl SessionExportFormat {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "srt" => Ok(Self::Srt),
            "txt" => Ok(Self::Txt),
            other => bail!("--export-format must be \"srt\" or \"txt\", got {other:?}"),
        }
    }

    fn render(self, segments: &[session::TranscriptSegment]) -> String {
        match self {
            Self::Srt => session::export_srt(segments),
            Self::Txt => session::export_txt(segments),
        }
    }
}

/// Parsed arguments for `--export-session`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionExportArgs {
    pub(crate) input: PathBuf,
    pub(crate) output: PathBuf,
    pub(crate) format: SessionExportFormat,
}

/// Parse session export flags.
pub(crate) fn parse_session_export_args_from<I>(args: I) -> Result<Option<SessionExportArgs>>
where
    I: IntoIterator<Item = OsString>,
{
    let mut saw_export_arg = false;
    let mut input = None;
    let mut output = None;
    let mut format = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == OsStr::new("--export-session") {
            saw_export_arg = true;
            input = Some(PathBuf::from(next_cli_arg(&mut iter, "--export-session")?));
        } else if arg == OsStr::new("--export-output") {
            saw_export_arg = true;
            output = Some(PathBuf::from(next_cli_arg(&mut iter, "--export-output")?));
        } else if arg == OsStr::new("--export-format") {
            saw_export_arg = true;
            let value = next_cli_arg(&mut iter, "--export-format")?;
            let value = value
                .into_string()
                .map_err(|_| anyhow::anyhow!("--export-format must be valid UTF-8"))?;
            format = Some(SessionExportFormat::parse(&value)?);
        } else if saw_export_arg {
            bail!("unknown session export argument {:?}", arg);
        }
    }

    if !saw_export_arg {
        return Ok(None);
    }

    Ok(Some(SessionExportArgs {
        input: input.context("missing --export-session <session.jsonl>")?,
        output: output.context("missing --export-output <path>")?,
        format: format.context("missing --export-format <srt|txt>")?,
    }))
}

fn next_cli_arg(iter: &mut impl Iterator<Item = OsString>, flag: &'static str) -> Result<OsString> {
    let value = iter
        .next()
        .with_context(|| format!("missing value after {flag}"))?;
    if value.to_string_lossy().starts_with("--") {
        bail!("missing value after {flag}");
    }
    Ok(value)
}

/// Export a recorded session JSONL file to SRT or plain text.
pub(crate) fn run_session_export(args: &SessionExportArgs) -> Result<()> {
    let contents = fs::read_to_string(&args.input)
        .with_context(|| format!("failed to read session log {}", args.input.display()))?;
    let segments = session::transcript_segments_from_jsonl(&contents)
        .with_context(|| format!("failed to parse session log {}", args.input.display()))?;
    let rendered = args.format.render(&segments);

    if let Some(parent) = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create export directory {}", parent.display()))?;
    }
    fs::write(&args.output, rendered)
        .with_context(|| format!("failed to write export {}", args.output.display()))?;

    let mut stdout = io::stdout();
    writeln!(
        stdout,
        "Exported {} transcript segment(s) to {}",
        segments.len(),
        args.output.display()
    )
    .context("failed to write export summary")?;
    Ok(())
}
