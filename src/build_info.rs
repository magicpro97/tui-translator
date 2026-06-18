// build_info.rs - constants populated by `build.rs` from git state.
//
// Exposes the short git SHA and package version of the binary that
// is currently running. The onboarding wizard renders these in the
// title bar so that computer-use driver tests (e.g. cua-driver) can
// confirm the artifact on screen was built from the commit under
// test: a binary built from a different commit shows a different
// SHA, and a binary built from a dirty tree shows a "-dirty"
// suffix.
//
// Limitation: the SHA/dirty marker is only as fresh as the last
// time `build.rs` ran. cargo re-runs build.rs when `.git/HEAD` or
// `.git/index` change (see the rerun-if-changed lines there), i.e.
// on commit / checkout / `git add`. A working-tree edit that
// touches neither — e.g. saving a source file without staging it —
// does NOT by itself force a rebuild, so the marker can lag until
// the next rebuild happens for another reason. Treat the marker as
// "the git state at last build", not a live working-tree probe.

// These values are produced by build.rs at compile time and live
// in OUT_DIR.  Including the file as a module gives us a stable
// import path: `crate::build_info::BUILD_SHA`.
include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
