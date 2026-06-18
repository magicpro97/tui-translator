use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    // `git status --porcelain` reports BOTH staged and unstaged
    // changes to tracked files (and untracked files), so the dirty
    // marker is accurate whenever build.rs actually runs.  This is
    // stricter than `git diff-index --quiet HEAD`, which only sees
    // changes relative to the index and so misses a tracked file
    // that was edited but never `git add`-ed.
    //
    // Caveat (documented in src/build_info.rs): cargo only re-runs
    // this script when one of the `rerun-if-changed` paths below
    // changes.  A working-tree edit that touches neither HEAD nor
    // the index will not, by itself, trigger a rebuild — so the
    // marker can lag until the next rebuild for another reason.
    let dirty = if Path::new(".git").exists() {
        Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=no"])
            .output()
            .ok()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false)
    } else {
        false
    };
    let sha = if dirty { format!("{sha}-dirty") } else { sha };
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    fs::write(
        Path::new(&out_dir).join("build_info.rs"),
        format!(
            "pub const BUILD_SHA: &str = \"{sha}\";\npub const BUILD_VERSION: &str = \"{version}\";\n"
        ),
    ).expect("write build_info.rs");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
