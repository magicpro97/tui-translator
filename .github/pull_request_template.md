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
