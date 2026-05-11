#!/usr/bin/env python3
"""
preToolUse hook — enforce Rust and repository config coding standards.

PURPOSE (plain language):
  This guard inspects every file the AI writes or modifies and blocks writes
  that violate the coding rules for this Rust project. It covers three file
  types: Rust source (.rs), Cargo manifest (Cargo.toml), and GitHub Actions
  workflow YAML files (.github/workflows/*.yml).

  False positives are kept low: test files and test modules have relaxed rules
  because some patterns (like unwrap()) are acceptable in test helpers.

─── RUST SOURCE (.rs) RULES ────────────────────────────────────────────────

  unsafe {} without // SAFETY: comment  [BLOCKED]
    Any `unsafe` block must be preceded immediately by a comment that explains
    why the code is sound. Example:
        // SAFETY: we checked ptr is non-null and aligned above
        unsafe { *ptr = value; }

  dbg!() macro  [BLOCKED]
    dbg!() prints to stderr and is a debug-only tool. It must never appear in
    committed source. Use the `tracing` or `log` crate for production logging.

  .unwrap() without justification  [BLOCKED in src/, allowed in tests]
    Calling .unwrap() can panic and crash the translator mid-meeting. In
    non-test source code, either:
      (a) Use .expect("context message") so panics are self-documenting, or
      (b) Propagate the error with `?`, or
      (c) Add `// OK: <reason>` on the preceding line if the value is provably
          non-None/non-Err (e.g., regex that was just validated to compile).
    Inside #[test] functions and test modules this rule is relaxed.

─── CARGO.TOML RULES ───────────────────────────────────────────────────────

  Wildcard dependency version "*"  [BLOCKED]
    Wildcards allow any future breaking version to be pulled in silently.
    Specify at least a minimum version: `some-crate = "0.1"`.

  Path dependencies that escape the workspace  [BLOCKED]
    Path dependencies pointing two or more levels above the project root
    (`../../other`) create implicit coupling to the local filesystem layout
    that breaks CI and other developers' machines. Document and justify them.

─── GITHUB ACTIONS WORKFLOW (.yml) RULES ───────────────────────────────────

  continue-on-error: true without an explanation comment  [BLOCKED]
    Silently ignoring job failures hides real problems. If you need it, add
    a comment above the line explaining why it is acceptable here.

  Echoing secret values to the log  [BLOCKED]
    `echo ${{ secrets.FOO }}` prints a secret value into CI logs, which may
    be public. GitHub attempts to redact them, but redaction is not foolproof.
"""

import json
import os
import re
import sys

if os.name == "nt":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

RS_EXT = re.compile(r"\.rs$")
CARGO_TOML = re.compile(r"(^|[/\\])Cargo\.toml$")
WORKFLOW = re.compile(r"\.github[/\\]workflows[/\\].+\.ya?ml$", re.IGNORECASE)


def deny(reason: str) -> None:
    print(json.dumps({"permissionDecision": "deny", "permissionDecisionReason": reason}))
    raise SystemExit(0)


def get_args(data: dict) -> dict:
    args = data.get("toolArgs", {})
    if isinstance(args, str):
        try:
            return json.loads(args)
        except json.JSONDecodeError:
            return {}
    return args if isinstance(args, dict) else {}


def is_test_file(path: str) -> bool:
    """Return True when the whole file is test-only (relaxed rules apply)."""
    return bool(
        re.search(r"[/\\](tests?|integration)[/\\]", path)
        or path.endswith("_test.rs")
        or path.endswith("_tests.rs")
    )


def check_rust(path: str, content: str) -> None:
    in_test_file = is_test_file(path)
    lines = content.splitlines()
    brace_depth = 0
    test_block_depths: list[int] = []
    pending_test_attr = False
    pending_test_block_open = False

    for i, line in enumerate(lines):
        test_block_depths = [depth for depth in test_block_depths if brace_depth >= depth]
        stripped = line.strip()
        if re.search(r"#\[test\]|#\[cfg\(test\)\]", stripped):
            pending_test_attr = True
            continue

        if stripped.startswith("//") or not stripped:
            continue  # skip comment lines

        prev_line = lines[i - 1].strip() if i > 0 else ""
        code_only = line.split("//", 1)[0]
        line_starts_test_module = bool(re.search(r"\bmod\s+tests\s*\{", code_only))
        line_declares_test_fn = bool(pending_test_attr and re.search(r"\bfn\b", code_only))
        in_test_block = bool(test_block_depths)
        line_is_test_context = in_test_file or in_test_block or line_starts_test_module or line_declares_test_fn

        # ── unsafe {} requires an immediately preceding // SAFETY: comment ──
        if re.search(r"\bunsafe\s*\{", code_only):
            if not re.search(r"//\s*SAFETY:", prev_line):
                deny(
                    f"Rust standard: `unsafe {{` at line {i + 1} of {path} "
                    "requires a `// SAFETY: <explanation>` comment on the "
                    "immediately preceding line. Document why this block is sound."
                )

        # ── dbg!() is always a debug artifact — never commit it ──────────────
        if re.search(r"\bdbg!\s*\(", code_only):
            deny(
                f"Rust standard: `dbg!()` found at line {i + 1} of {path}. "
                "Remove debug macros before committing. "
                "Use the `tracing` or `log` crate for structured, levelled logging."
            )

        # ── .unwrap() without justification — only checked in non-test src ───
        if not line_is_test_context and re.search(r"\.unwrap\(\)", code_only):
            # Allow if the caller left a justification comment on the previous line
            if not re.search(
                r"//\s*(OK|SAFETY|unwrap|infallible|cannot fail|always\s+Some|always\s+Ok)",
                prev_line,
                re.IGNORECASE,
            ):
                deny(
                    f"Rust standard: `.unwrap()` at line {i + 1} of {path} may panic "
                    "and crash the translator during a live meeting. "
                    "Use `.expect(\"context\")` for self-documenting panics, `?` to propagate "
                    "errors, or add `// OK: <reason>` on the preceding line if the value is "
                    "provably non-None/non-Err."
                )

        brace_depth_before = brace_depth
        brace_depth += code_only.count("{") - code_only.count("}")

        if line_starts_test_module and brace_depth > brace_depth_before:
            test_block_depths.append(brace_depth)
            pending_test_attr = False
            pending_test_block_open = False
        elif line_declares_test_fn:
            pending_test_attr = False
            if brace_depth > brace_depth_before:
                test_block_depths.append(brace_depth)
                pending_test_block_open = False
            elif "{" in code_only:
                pending_test_block_open = False
            else:
                pending_test_block_open = True
        elif pending_test_block_open and brace_depth > brace_depth_before:
            test_block_depths.append(brace_depth)
            pending_test_block_open = False


def check_cargo_toml(path: str, content: str) -> None:
    lines = content.splitlines()
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("#"):
            continue

        # Wildcard version: crate = "*" or version = "*"
        if re.search(r'(=\s*"[*]"|version\s*=\s*"[*]")', line):
            deny(
                f"Cargo.toml standard: wildcard dependency version `\"*\"` at line {i + 1} of {path}. "
                "Wildcards allow any future breaking version to be pulled in silently. "
                'Specify a minimum version (e.g., "0.1") or an exact pinned version.'
            )

        # Path dependency escaping the workspace by two or more levels
        path_m = re.search(r'path\s*=\s*"([^"]+)"', line)
        if path_m:
            dep_path = path_m.group(1)
            # Count leading ../ traversals
            traversals = len(re.findall(r"\.\.[/\\]", dep_path))
            if traversals >= 2:
                deny(
                    f"Cargo.toml standard: path dependency at line {i + 1} of {path} "
                    f"escapes the workspace by {traversals} levels (`{dep_path}`). "
                    "This breaks CI and other developers' checkouts. "
                    "Publish the crate or vendor it inside the repository instead."
                )


def check_workflow(path: str, content: str) -> None:
    lines = content.splitlines()
    for i, line in enumerate(lines):
        stripped = line.strip()

        # continue-on-error: true without a preceding comment
        if re.match(r"continue-on-error:\s*true", stripped):
            prev = lines[i - 1].strip() if i > 0 else ""
            if not prev.startswith("#"):
                deny(
                    f"CI standard: `continue-on-error: true` at line {i + 1} of {path} "
                    "hides job failures. Add a `# reason:` comment above explaining why "
                    "this is intentional, or remove it."
                )

        # Echoing a secret value directly into CI logs
        if re.search(r"echo\s+\$\{\{\s*secrets\.", line, re.IGNORECASE):
            deny(
                f"CI standard: echoing a secret value at line {i + 1} of {path}. "
                "Never print secret values to CI logs — GitHub's redaction is "
                "not foolproof and the value may appear in forked-PR logs."
            )


def main() -> int:
    try:
        data = json.loads(sys.stdin.read() or "{}")
    except json.JSONDecodeError:
        return 0
    if data.get("toolName") not in {"edit", "create"}:
        return 0

    args = get_args(data)
    path = str(args.get("path", ""))
    content = str(args.get("new_str") or args.get("file_text") or "")
    if not path or not content:
        return 0

    if RS_EXT.search(path):
        check_rust(path, content)
    elif CARGO_TOML.search(path):
        check_cargo_toml(path, content)
    elif WORKFLOW.search(path):
        check_workflow(path, content)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
