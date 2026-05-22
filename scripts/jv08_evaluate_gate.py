"""Offline evaluator for the JV-08 local MT default eligibility gate.

The script intentionally performs no network calls and writes no files. It
checks that the machine-readable JV-08 verdict is internally consistent, points
only at committed repo-local evidence, and cannot claim a passing/failing gate
without explicit evidence references.
"""
from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
ALLOWED_INPUT_PREFIXES = ("docs/evidence/", "docs/adr/", "tests/fixtures/")
ALLOWED_EVIDENCE_PREFIXES = ("docs/evidence/", "docs/adr/", "src/", "tests/", "config.example.json")
ALLOWED_DECISIONS = {"flip-to-local", "keep-google-default", "defer-no-evidence"}
ALLOWED_GATE_RESULTS = {"pass", "fail", "blocked-no-evidence", "not-applicable"}
SECRET_PATTERNS = [
    re.compile(r"AIza[0-9A-Za-z_\-]{35}"),
    re.compile(r"ya29\.[0-9A-Za-z._\-]{10,}"),
    re.compile(r"(?i)authorization\s*:\s*bearer\s+[A-Za-z0-9._\-]{8,}"),
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(r"(?i)\b(api[_-]?key|client_secret|access_token)\b\s*[:=]\s*[A-Za-z0-9._\-]{8,}"),
]


class GateError(Exception):
    """Raised when the JV-08 gate artifact is invalid."""


def normalized_rel_path(value: str, allowed_prefixes: tuple[str, ...]) -> Path:
    """Return a safe repo-relative path after traversal and prefix checks."""
    if not value or not isinstance(value, str):
        raise GateError("path must be a non-empty string")

    candidate = Path(value)
    if candidate.is_absolute():
        raise GateError(f"path must be repo-relative, got absolute path: {value}")
    if ".." in candidate.parts:
        raise GateError(f"path must not contain '..': {value}")

    display = value.replace("\\", "/")
    if not any(path_matches_allowed_prefix(display, prefix) for prefix in allowed_prefixes):
        raise GateError(f"path is outside allowed prefixes: {value}")

    resolved = (REPO_ROOT / candidate).resolve()
    try:
        resolved.relative_to(REPO_ROOT)
    except ValueError as exc:
        raise GateError(f"path escapes repository: {value}") from exc
    return resolved


def path_matches_allowed_prefix(display: str, prefix: str) -> bool:
    """Match directory prefixes by prefix and file allow-list entries exactly."""
    if prefix.endswith("/"):
        return display == prefix.rstrip("/") or display.startswith(prefix)
    return display == prefix


def evidence_path_from_ref(ref: str) -> str:
    """Extract the repo path portion from a file:line or file::symbol reference."""
    path = ref.split("::", 1)[0].split("#", 1)[0]
    match = re.match(r"^(.+?):\d", path)
    if match:
        return match.group(1)
    return path


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise GateError(f"{path}: invalid JSON: {exc}") from exc


def reject_secrets(raw: str) -> None:
    for pattern in SECRET_PATTERNS:
        if pattern.search(raw):
            raise GateError(f"artifact appears to contain secret-like material: {pattern.pattern}")


def validate_inputs(artifact: dict[str, Any]) -> dict[str, dict[str, Any]]:
    by_wbs: dict[str, dict[str, Any]] = {}
    for item in artifact.get("inputs", []):
        if not isinstance(item, dict):
            raise GateError("inputs entries must be objects")
        wbs = item.get("wbs")
        if not isinstance(wbs, str) or not wbs:
            raise GateError("inputs[].wbs is required")
        by_wbs[wbs] = item

        path_value = item.get("path")
        status = item.get("status")
        if path_value is None:
            if status not in {"missing", "issue-comment-only"}:
                raise GateError(f"{wbs}: null path must use status missing or issue-comment-only")
            if item.get("sha256") is not None:
                raise GateError(f"{wbs}: missing input must not have sha256")
            continue

        resolved = normalized_rel_path(path_value, ALLOWED_INPUT_PREFIXES)
        if not resolved.exists():
            raise GateError(f"{wbs}: referenced input does not exist: {path_value}")

        expected_sha = item.get("sha256")
        if not isinstance(expected_sha, str) or not re.fullmatch(r"[0-9a-f]{64}", expected_sha):
            raise GateError(f"{wbs}: sha256 must be a lowercase 64-char hex string")
        actual_sha = sha256_file(resolved)
        if actual_sha != expected_sha:
            raise GateError(f"{wbs}: sha256 mismatch for {path_value}: expected {expected_sha}, got {actual_sha}")

        if resolved.suffix == ".json":
            payload = load_json(resolved)
            schema = item.get("schema")
            if schema and isinstance(payload, dict) and payload.get("schema_version") != schema:
                raise GateError(
                    f"{wbs}: schema mismatch for {path_value}: expected {schema}, got {payload.get('schema_version')}"
                )
            if status and isinstance(payload, dict) and payload.get("status") and payload.get("status") != status:
                raise GateError(
                    f"{wbs}: status mismatch for {path_value}: expected {status}, got {payload.get('status')}"
                )
    return by_wbs


def validate_evidence_refs(refs: list[Any], gate_id: str) -> None:
    if not isinstance(refs, list):
        raise GateError(f"{gate_id}: evidence_refs must be an array")
    if not refs:
        raise GateError(f"{gate_id}: pass/fail gates require evidence_refs")
    for ref in refs:
        if not isinstance(ref, str) or not ref:
            raise GateError(f"{gate_id}: evidence_refs must be non-empty strings")
        path_part = evidence_path_from_ref(ref)
        resolved = normalized_rel_path(path_part, ALLOWED_EVIDENCE_PREFIXES)
        if not resolved.exists():
            raise GateError(f"{gate_id}: evidence_ref does not exist: {ref}")


def validate_gates(artifact: dict[str, Any]) -> None:
    gates = artifact.get("gates")
    if not isinstance(gates, list) or not gates:
        raise GateError("gates must be a non-empty array")

    blocked_count = 0
    for gate in gates:
        if not isinstance(gate, dict):
            raise GateError("gates entries must be objects")
        gate_id = gate.get("id")
        if not isinstance(gate_id, str) or not gate_id:
            raise GateError("gates[].id is required")
        result = gate.get("result")
        if result not in ALLOWED_GATE_RESULTS:
            raise GateError(f"{gate_id}: unknown result {result!r}")
        if result in {"pass", "fail"}:
            validate_evidence_refs(gate.get("evidence_refs", []), gate_id)
        if result == "blocked-no-evidence":
            blocked_count += 1
            if gate.get("observed") is not None:
                raise GateError(f"{gate_id}: blocked-no-evidence gates must keep observed=null")

    decision = artifact.get("default_flip_decision")
    if decision == "flip-to-local" and any(gate.get("result") != "pass" for gate in gates):
        raise GateError("flip-to-local requires every gate to pass")
    if decision == "defer-no-evidence" and blocked_count == 0:
        raise GateError("defer-no-evidence requires at least one blocked-no-evidence gate")


def validate_target(artifact: dict[str, Any]) -> None:
    target = artifact.get("implementation_target")
    if not isinstance(target, dict):
        raise GateError("implementation_target is required")
    candidate = target.get("candidate")
    if candidate != "opus-mt-ja-vi":
        raise GateError(f"implementation_target.candidate must be opus-mt-ja-vi, got {candidate!r}")
    rationale_ref = target.get("rationale_ref")
    runtime_ref = target.get("runtime_ref")
    if not isinstance(rationale_ref, str) or not isinstance(runtime_ref, str):
        raise GateError("implementation_target rationale_ref and runtime_ref are required")
    rationale_path = normalized_rel_path(rationale_ref, ("docs/adr/",))
    runtime_path = normalized_rel_path(runtime_ref, ("docs/adr/",))
    if candidate not in rationale_path.read_text(encoding="utf-8"):
        raise GateError(f"{candidate} not found in rationale_ref")
    if "ORT KV-cache" not in runtime_path.read_text(encoding="utf-8"):
        raise GateError("runtime_ref must contain the ORT KV-cache decision")


def validate(path: Path) -> None:
    raw = path.read_text(encoding="utf-8")
    reject_secrets(raw)
    artifact = json.loads(raw)
    if not isinstance(artifact, dict):
        raise GateError("artifact must be a JSON object")
    if artifact.get("schema_version") != "jv-08-v1":
        raise GateError(f"unknown schema_version: {artifact.get('schema_version')!r}")
    if artifact.get("wbs_key") != "JV-08":
        raise GateError("wbs_key must be JV-08")
    if artifact.get("issue") != 416:
        raise GateError("issue must be 416")
    if artifact.get("routing_confidence") != 1.0:
        raise GateError("routing_confidence must be exactly 1.0")
    if artifact.get("default_flip_decision") not in ALLOWED_DECISIONS:
        raise GateError(f"unknown default_flip_decision: {artifact.get('default_flip_decision')!r}")

    validate_inputs(artifact)
    validate_target(artifact)
    validate_gates(artifact)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: python scripts/jv08_evaluate_gate.py <gate-json>", file=sys.stderr)
        return 2
    path = Path(argv[1])
    if not path.is_absolute():
        path = (Path.cwd() / path).resolve()
    try:
        validate(path)
    except (GateError, OSError, json.JSONDecodeError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1

    payload = load_json(path)
    print(
        "OK: JV-08 gate valid "
        f"(decision={payload['default_flip_decision']}, gates={len(payload['gates'])})"
    )
    for gate in payload["gates"]:
        print(f"  {gate['id']}: {gate['result']}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
