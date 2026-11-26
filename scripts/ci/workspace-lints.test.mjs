import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { ROOT } from "../lib/git.mjs";

function stripTomlComment(line) {
  let quote = null;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (quote === '"') {
      if (ch === "\\") i++;
      else if (ch === '"') quote = null;
    } else if (quote === "'") {
      if (ch === "'") quote = null;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (ch === "#") {
      return line.slice(0, i);
    }
  }
  return line;
}

function bracketDelta(line) {
  let quote = null;
  let delta = 0;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (quote === '"') {
      if (ch === "\\") i++;
      else if (ch === '"') quote = null;
    } else if (quote === "'") {
      if (ch === "'") quote = null;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (ch === "[" || ch === "{") {
      delta++;
    } else if (ch === "]" || ch === "}") {
      delta--;
    }
  }
  return delta;
}

function parseTomlTables(text) {
  const tables = new Map([["", new Map()]]);
  let current = "";
  let pending = "";
  let depth = 0;

  for (const rawLine of text.replaceAll("\r\n", "\n").split("\n")) {
    const line = stripTomlComment(rawLine).trim();
    if (!line) continue;

    if (depth === 0) {
      if (line.startsWith("[[") && line.endsWith("]]")) {
        current = line;
        if (!tables.has(current)) tables.set(current, new Map());
        continue;
      }
      if (line.startsWith("[") && line.endsWith("]")) {
        current = line.slice(1, -1).trim();
        if (!tables.has(current)) tables.set(current, new Map());
        continue;
      }
    }

    pending = pending ? `${pending}\n${line}` : line;
    depth += bracketDelta(line);
    if (depth === 0) {
      const eq = pending.indexOf("=");
      if (eq !== -1) {
        const key = pending.slice(0, eq).trim();
        const value = pending.slice(eq + 1).trim();
        tables.get(current).set(key, value);
      }
      pending = "";
    }
  }

  if (depth !== 0 || pending !== "") {
    throw new Error("Unclosed bracket or brace in TOML manifest.");
  }
  return tables;
}

function parseStringArray(raw, label) {
  if (!raw || !raw.startsWith("[") || !raw.endsWith("]")) {
    throw new Error(`Expected ${label} to be a TOML string array.`);
  }
  const normalized = raw.replace(/'([^']*)'/g, (_, s) => JSON.stringify(s)).replace(/,\s*\]$/, "]");
  const parsed = JSON.parse(normalized);
  if (!Array.isArray(parsed) || parsed.some((item) => typeof item !== "string")) {
    throw new Error(`Expected ${label} to contain only strings.`);
  }
  return parsed;
}

function workspaceLintProblems(rootManifest, readMember) {
  const problems = [];
  const rootTables = parseTomlTables(rootManifest);
  const rawMembers = rootTables.get("workspace")?.get("members");
  let members = [];

  if (!rawMembers) {
    problems.push("Cargo.toml: [workspace] is missing 'members'.");
  } else {
    try {
      members = parseStringArray(rawMembers, "Cargo.toml [workspace].members");
      if (members.length === 0) problems.push("Cargo.toml: [workspace].members is empty.");
    } catch (error) {
      problems.push(`Cargo.toml: ${error.message}`);
    }
  }

  for (const section of ["workspace.lints.rust", "workspace.lints.clippy"]) {
    const entries = rootTables.get(section);
    if (!entries || entries.size === 0) {
      problems.push(`Cargo.toml: [${section}] is missing or empty; define at least one lint in [${section}].`);
    }
  }

  for (const member of members) {
    const manifestPath = `${member}/Cargo.toml`;
    const memberManifest = readMember(manifestPath);
    if (memberManifest === null) {
      problems.push(`${manifestPath}: file does not exist.`);
      continue;
    }
    const memberTables = parseTomlTables(memberManifest);
    const lints = memberTables.get("lints");
    if (!lints) {
      problems.push(`${manifestPath}: missing [lints] section; add '[lints]' with 'workspace = true'.`);
    } else if (lints.get("workspace") !== "true" || lints.size !== 1) {
      problems.push(`${manifestPath}: [lints] must contain only 'workspace = true'.`);
    }
    for (const tableName of memberTables.keys()) {
      if (tableName.startsWith("lints.")) {
        problems.push(`${manifestPath}: [${tableName}] overrides workspace lints; use '[lints] workspace = true'.`);
      }
    }
  }

  return problems;
}

test("every workspace crate inherits [workspace.lints]", () => {
  const rootManifest = readFileSync(join(ROOT, "Cargo.toml"), "utf8");
  const problems = workspaceLintProblems(rootManifest, (relativePath) => {
    const fullPath = join(ROOT, relativePath);
    return existsSync(fullPath) ? readFileSync(fullPath, "utf8") : null;
  });
  assert.deepEqual(problems, []);

  const rootTables = parseTomlTables(rootManifest);
  const rustLints = rootTables.get("workspace.lints.rust");
  const clippyLints = rootTables.get("workspace.lints.clippy");
  assert.equal(rustLints?.get("non_ascii_idents"), '"forbid"');
  assert.equal(rustLints?.get("unused_must_use"), '"deny"');
  assert.equal(clippyLints?.get("dbg_macro"), '"warn"');
  assert.equal(clippyLints?.get("todo"), '"warn"');
  assert.equal(clippyLints?.get("unimplemented"), '"warn"');
});

test("workspaceLintProblems accepts multiline members and workspace dependency syntax", () => {
  const root = `
[workspace]
members = [
  "crates/alpha", # comment
  'crates/beta',
]

[workspace.lints.rust]
unused_must_use = "deny"

[workspace.lints.clippy]
todo = "warn"
`;
  const member = `
[package]
name = "alpha"
version.workspace = true

[dependencies]
tokio = { workspace = true, features = [
  "rt-multi-thread",
] }

[lints]
workspace = true
`;
  const files = { "crates/alpha/Cargo.toml": member, "crates/beta/Cargo.toml": member };
  assert.deepEqual(
    workspaceLintProblems(root, (path) => files[path] ?? null),
    [],
  );
});

test("workspaceLintProblems reports missing sections, false opt-in, overrides, and missing manifests", () => {
  const root = `
[workspace]
members = ["crates/missing-lints", "crates/false-lints", "crates/override-lints", "crates/absent"]

[workspace.lints.rust]
unused_must_use = "deny"
`;
  const files = {
    "crates/missing-lints/Cargo.toml": `
[package]
version.workspace = true
[dependencies]
tokio = { workspace = true }
# [lints]
# workspace = true
`,
    "crates/false-lints/Cargo.toml": `
[lints]
workspace = false
`,
    "crates/override-lints/Cargo.toml": `
[lints]
workspace = true
[lints.rust]
unused_must_use = "allow"
`,
  };
  const problems = workspaceLintProblems(root, (path) => files[path] ?? null);
  assert.equal(problems.length, 5);
  assert.ok(problems.some((p) => p.includes("[workspace.lints.clippy]")));
  assert.ok(problems.some((p) => p.includes("crates/missing-lints/Cargo.toml: missing [lints]")));
  assert.ok(problems.some((p) => p.includes("crates/false-lints/Cargo.toml: [lints] must contain only")));
  assert.ok(problems.some((p) => p.includes("crates/override-lints/Cargo.toml: [lints.rust] overrides")));
  assert.ok(problems.some((p) => p.includes("crates/absent/Cargo.toml: file does not exist")));
});
