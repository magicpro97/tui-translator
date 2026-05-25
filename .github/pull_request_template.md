<!--
  Pull request template — see docs/engineering-standards.md (#483).
  Reviewers must verify each checklist item before approving.
-->

## Summary

<!-- What changed and why? Link the issue this addresses. -->

Closes #

## Engineering standards checklist (STD-01, #483)

- [ ] **TDD:** tests precede or accompany this change (red → green), or a
      clear reason is given why testing is impossible.
- [ ] **Docs:** every new `pub` item has a `///` doc-comment.
- [ ] **No unwrap/expect/panic!** in non-test code (or `// allow-unwrap: #NNN`).
- [ ] **File size:** no new file exceeds 600 lines; refactored files do not
      grow past the limit (or a waiver with linked refactor issue is added
      to `.standards-waivers.txt`).
- [ ] **Function size:** no function exceeds 80 lines (clippy enforced).
- [ ] **TODO/FIXME** markers all reference an issue (`#NNN`).
- [ ] **Conventional Commits**: every commit title uses
      `type(scope?)!?: summary`.
- [ ] **DCO sign-off**: every commit has `Signed-off-by:` (use `git commit -s`).

## Verification

<!-- Paste the commands you ran locally and their results. At minimum:
     cargo fmt --check
     cargo clippy --all-targets --all-features -- -D warnings
     cargo test
     python scripts/standards/check_loc.py
     python scripts/standards/check_todo_refs.py
-->

## PROC-01 Opus review gate (#465)

<!--
  Required. See docs/proc-opus-gate.md. The proc-opus-gate workflow parses
  this section and the file paths in the diff.
-->

Confidence: <!-- one of: 1.0, 0.9, 0.8, 0.7, 0.6, <0.6 -->

### Opus review evidence

<!--
  Required when ANY of the following is true:
    - Confidence < 1.0
    - This PR touches src/audio/, src/providers/, or src/pipeline/
    - The `needs-opus-review` label is set on the PR or the closed issue

  Otherwise write "N/A — confidence 1.0, no sensitive paths touched".

  When required, paste:
    - Reviewer agent (e.g. tui-rust-code-reviewer, nfr-verification-gate)
    - Verdict: CLEAN / CHANGES REQUESTED
    - Link to the review comment or evidence file
    - For confidence < 1.0: link to the spike evidence OR explicit user
      override quote ("override: <user handle>: <reason>").
-->
