// Logic dry-check for issue-hygiene.yml sync-project-priority gating.
// Mirrors the inline github-script in the `inspect` step verbatim, then
// applies the gate expressions from sync-project-priority.
//
// Three scenarios:
//   A. single priority:P0    -> sync writes P0
//   B. single priority:P2    -> sync SKIPS (does NOT write P0)
//   C. no priority label     -> apply-default => effective priority:P0 => sync writes P0

process.env.DEFAULT_PRIORITY_LABEL = "priority:P0";

function inspect(issue) {
  const out = {};
  const setOutput = (k, v) => (out[k] = v);
  if (!issue) {
    setOutput("action", "skip");
    setOutput("effective_priority", "");
    return out;
  }
  const allowed = new Set(["priority:P0","priority:P1","priority:P2","priority:P3"]);
  const priorityLabels = (issue.labels || [])
    .map((l) => (typeof l === "string" ? l : l.name))
    .filter((n) => typeof n === "string" && n.startsWith("priority:"));
  setOutput("count", String(priorityLabels.length));
  setOutput("labels", priorityLabels.join(","));
  let effective = "";
  if (priorityLabels.length === 0) {
    setOutput("action", "apply-default");
    effective = process.env.DEFAULT_PRIORITY_LABEL || "priority:P0";
  } else if (priorityLabels.length === 1) {
    setOutput("action", "ok");
    if (allowed.has(priorityLabels[0])) effective = priorityLabels[0];
  } else {
    setOutput("action", "mismatch");
  }
  setOutput("effective_priority", effective);
  return out;
}

function syncDecision(effective, hasToken = true) {
  // Mirrors: if: steps.token_check.outputs.has_token == 'true' && env.EFFECTIVE_PRIORITY == 'priority:P0'
  const writeP0 = hasToken && effective === "priority:P0";
  // Mirrors: skip step if: has_token == 'true' && env.EFFECTIVE_PRIORITY != 'priority:P0'
  const skipNeutral = hasToken && effective !== "priority:P0";
  return { writeP0, skipNeutral };
}

const cases = [
  { name: "A: single priority:P0", issue: { labels: [{ name: "priority:P0" }] }, expectWrite: true },
  { name: "B: single priority:P2", issue: { labels: [{ name: "priority:P2" }] }, expectWrite: false },
  { name: "C: no priority label",  issue: { labels: [{ name: "area:audio" }] },  expectWrite: true },
  { name: "D: mismatch P0+P2",     issue: { labels: [{ name: "priority:P0" }, { name: "priority:P2" }] }, expectWrite: false },
];

let allPass = true;
for (const c of cases) {
  const ins = inspect(c.issue);
  const dec = syncDecision(ins.effective_priority);
  const pass = dec.writeP0 === c.expectWrite && (c.expectWrite ? !dec.skipNeutral : dec.skipNeutral);
  if (!pass) allPass = false;
  console.log(`${pass ? "PASS" : "FAIL"} ${c.name}`);
  console.log(`  action=${ins.action} effective_priority='${ins.effective_priority}' writeP0=${dec.writeP0} skipNeutral=${dec.skipNeutral}`);
}
process.exit(allPass ? 0 : 1);
