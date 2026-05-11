# Contributing to TUI Translator

Thank you for your interest in contributing!  This guide explains how to report
problems, suggest changes, and submit code — in plain language, step by step.

---

## Table of contents

1. [Reporting a bug](#1-reporting-a-bug)
2. [Requesting a feature](#2-requesting-a-feature)
3. [Setting up a development environment](#3-setting-up-a-development-environment)
4. [Making a code change](#4-making-a-code-change)
5. [Running the checks](#5-running-the-checks)
6. [Opening a pull request](#6-opening-a-pull-request)
7. [Code of conduct](#7-code-of-conduct)

---

## 1. Reporting a bug

Open a [bug report issue](../../issues/new?template=bug_report.md) and fill in:

- What you did, step by step.
- What you expected to happen.
- What actually happened (including any error text from the terminal).
- Your Windows version and terminal emulator.

**Before you post:** search the [existing issues](../../issues) to see if
someone has already reported the same problem.

---

## 2. Requesting a feature

Open a [work package issue](../../issues/new?template=work_package.md) and
describe:

- What user problem would the new feature solve?
- What would the user see or be able to do after the feature exists?
- What is explicitly out of scope for this request?

---

## 3. Setting up a development environment

You need:

- **Windows 10 or 11** (required for audio capture testing)
- **Rust** (stable, 1.77+) — install from [rustup.rs](https://rustup.rs)
- **Git** — install from [git-scm.com](https://git-scm.com)
- A terminal emulator — Windows Terminal is recommended

Clone and build:

```powershell
git clone https://github.com/magicpro97/tui-translator.git
cd tui-translator
cargo build
```

You do not need a Google API key to build or to run the unit tests.

---

## 4. Making a code change

1. **Find or open an issue** for the work you want to do.  Changes without an
   associated issue are harder to review.

2. **Create a branch** from `main`:

   ```powershell
   git checkout -b your-branch-name
   ```

   Use a short, descriptive name such as `fix-cost-rounding` or
   `phase1-audio-stub`.

3. **Make your changes.**  A few guidelines:

   - Keep changes small and focused.  One issue per pull request.
   - Add or update tests for any logic you change.
   - Write doc-comments (`///`) for every public function or type you add.
   - Keep user-facing messages in plain English.
   - Do not commit `config.json` — it may contain real API keys.

4. **Format and check your code** before committing (see [section 5](#5-running-the-checks)).

---

## 5. Running the checks

The CI pipeline runs these four commands on every push.  Run them locally
first to catch problems early:

```powershell
# 1. Check formatting (must produce no output)
cargo fmt --check

# 2. Lint — all warnings are treated as errors
cargo clippy --all-targets --all-features -- -D warnings

# 3. Build (debug and release)
cargo build --all-targets
cargo build --release

# 4. Unit tests
cargo test
```

All four must pass with no errors before a pull request can be merged.

---

## 6. Opening a pull request

1. Push your branch to GitHub:

   ```powershell
   git push origin your-branch-name
   ```

2. Open a pull request against `main`.  In the description:
   - Link the issue it addresses (e.g. `Closes #42`).
   - Describe what changed and why.
   - List any manual steps needed to verify the change.

3. The CI checks run automatically.  Wait for them to pass before asking for
   a review.

4. A maintainer will review and may ask questions or request changes.
   Respond to comments in the pull request thread.

5. Once approved and CI is green, a maintainer will merge the pull request.

---

## 7. Code of conduct

Be respectful and constructive.  This project follows the
[Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/)
code of conduct.  Harassment or exclusionary behaviour will not be tolerated.
