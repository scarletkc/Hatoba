import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { messages } from "./locales";

/**
 * The built-in `hatoba` skill (crates/hatoba-ai/src/skills/builtin/hatoba) describes the current UI to the
 * AI assistant. Its label tables must quote the i18n tables exactly, so a renamed or reworded label fails here
 * and the skill gets updated with it (docs/development.md).
 */

const SKILL_DIR = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../crates/hatoba-ai/src/skills/builtin/hatoba");
/** `read_skill` shortens a longer file, so each one stays under this. */
const MAX_CHARS = 15_000;
/** SKILL.md descriptions are limited to this many characters (spec AI-27). */
const MAX_DESCRIPTION_CHARS = 1_024;
const HEADER = "| en | zh-CN | ja |";

interface SkillFile {
  /** Path relative to the skill folder, with forward slashes. */
  name: string;
  text: string;
}

function readSkillFiles(): SkillFile[] {
  const files: SkillFile[] = [{ name: "SKILL.md", text: readFileSync(resolve(SKILL_DIR, "SKILL.md"), "utf8") }];
  for (const entry of readdirSync(resolve(SKILL_DIR, "references"), { withFileTypes: true })) {
    if (entry.isFile() && entry.name.endsWith(".md")) {
      files.push({ name: `references/${entry.name}`, text: readFileSync(resolve(SKILL_DIR, "references", entry.name), "utf8") });
    }
  }
  return files;
}

const files = readSkillFiles();
const referenceFiles = files.filter((f) => f.name !== "SKILL.md");

interface LabelRow {
  file: string;
  line: number;
  cells: string[];
}

/** Splits one Markdown table row into trimmed cells (`\|` is a literal pipe). */
function splitRow(line: string): string[] {
  const cells = line
    .replace(/\\\|/g, "\u0000")
    .split("|")
    .map((c) => c.replace(/\u0000/g, "|").trim());
  return cells.slice(1, line.trimEnd().endsWith("|") ? -1 : undefined);
}

/** Every row of every table whose header is exactly `| en | zh-CN | ja |`, plus header mistakes. */
function labelRows(file: SkillFile): { rows: LabelRow[]; problems: string[] } {
  const rows: LabelRow[] = [];
  const problems: string[] = [];
  const lines = file.text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (/^\|\s*en\s*\|\s*zh-CN\s*\|\s*ja\s*\|$/.test(line) && line !== HEADER) {
      problems.push(`${file.name}:${i + 1}: the header must be exactly "${HEADER}", found "${line}"`);
      continue;
    }
    if (line !== HEADER) continue;
    if (!/^\|\s*-+\s*\|\s*-+\s*\|\s*-+\s*\|$/.test((lines[i + 1] ?? "").trim())) {
      problems.push(`${file.name}:${i + 2}: the line under the header must be |---|---|---|`);
    }
    for (let j = i + 2; j < lines.length && lines[j].trim().startsWith("|"); j++) {
      rows.push({ file: file.name, line: j + 1, cells: splitRow(lines[j]) });
    }
  }
  return { rows, problems };
}

const zh = messages["zh-CN"];
const en = messages.en;
const ja = messages.ja;
const placeholder = /\{\w+\}/;

/** Why a row matches no message key, naming the closest key. */
function explainMismatch(row: LabelRow): string {
  const [e, z, j] = row.cells;
  const keys = Object.keys(zh);
  let best: { key: string; hits: number } | null = null;
  for (const key of keys) {
    const hits = Number(en[key] === e) + Number(zh[key] === z) + Number(ja[key] === j);
    if (hits > 0 && (!best || hits > best.hits)) best = { key, hits };
  }
  if (!best) return "no message key has any of these three strings";
  const k = best.key;
  const diffs = [
    en[k] !== e && `en should be ${JSON.stringify(en[k])}`,
    zh[k] !== z && `zh-CN should be ${JSON.stringify(zh[k])}`,
    ja[k] !== j && `ja should be ${JSON.stringify(ja[k])}`,
  ].filter(Boolean);
  return `closest key is ${k}: ${diffs.join("; ")}`;
}

describe("built-in hatoba skill", () => {
  it("has SKILL.md and reference files", () => {
    expect(files[0].name).toBe("SKILL.md");
    expect(referenceFiles.length).toBeGreaterThan(0);
  });

  it("has frontmatter with the name hatoba and a description", () => {
    const skill = files[0].text.replace(/^﻿/, "").split(/\r?\n/);
    expect(skill[0], "SKILL.md must start with a --- line").toBe("---");
    const end = skill.indexOf("---", 1);
    expect(end, "SKILL.md frontmatter must be closed by a --- line").toBeGreaterThan(0);
    const fields = new Map<string, string>();
    for (const line of skill.slice(1, end)) {
      const m = /^([A-Za-z_-]+):\s*(.*)$/.exec(line);
      if (!m) continue;
      let value = m[2].trim();
      const quoted = value.length >= 2 && (value.startsWith('"') || value.startsWith("'")) && value.endsWith(value[0]);
      if (quoted) value = value.slice(1, -1);
      // A YAML plain scalar cannot hold ": " or " #", so such a value must be quoted.
      if (!quoted) expect(value, `SKILL.md: the unquoted ${m[1]} must not contain ": " or " #" (quote it)`).not.toMatch(/: | #/);
      fields.set(m[1], value);
    }
    expect(fields.get("name")).toBe("hatoba");
    const description = fields.get("description") ?? "";
    expect(description.length, "SKILL.md needs a description").toBeGreaterThan(0);
    expect([...description].length, "SKILL.md description is too long").toBeLessThanOrEqual(MAX_DESCRIPTION_CHARS);
  });

  it("keeps the app version placeholder in SKILL.md, outside the label tables", () => {
    expect(files[0].text).toContain("{{HATOBA_VERSION}}");
    const inTables = files.flatMap((f) => labelRows(f).rows).filter((r) => r.cells.some((c) => c.includes("{{")));
    expect(inTables.map((r) => `${r.file}:${r.line}`)).toEqual([]);
  });

  it.each(files.map((f) => [f.name, f] as const))("%s stays under the character limit", (_name, file) => {
    const chars = [...file.text].length;
    expect(chars, `${file.name} has ${chars} characters; read_skill shortens a file over ${MAX_CHARS}`).toBeLessThan(MAX_CHARS);
  });

  it("links every reference file from SKILL.md, and links only existing files", () => {
    const linked = new Set(files[0].text.match(/references\/[A-Za-z0-9_.-]+\.md/g) ?? []);
    const present = new Set(referenceFiles.map((f) => f.name));
    expect([...present].filter((n) => !linked.has(n)).map((n) => `SKILL.md does not mention ${n}`)).toEqual([]);
    expect([...linked].filter((n) => !present.has(n)).map((n) => `SKILL.md mentions ${n}, which does not exist`)).toEqual([]);

    const dangling: string[] = [];
    for (const f of referenceFiles) {
      for (const n of new Set(f.text.match(/references\/[A-Za-z0-9_.-]+\.md/g) ?? [])) {
        if (!present.has(n)) dangling.push(`${f.name} mentions ${n}, which does not exist`);
      }
    }
    expect(dangling).toEqual([]);
  });

  it("has well-formed label tables", () => {
    const problems = files.flatMap((f) => labelRows(f).problems);
    const bad = files.flatMap((f) => labelRows(f).rows).filter((r) => r.cells.length !== 3 || r.cells.some((c) => c === ""));
    expect([...problems, ...bad.map((r) => `${r.file}:${r.line}: a label row needs three non-empty cells (en, zh-CN, ja)`)]).toEqual([]);
    expect(files.flatMap((f) => labelRows(f).rows).length).toBeGreaterThan(100);
  });

  it.each(files.map((f) => [f.name, f] as const))("%s quotes UI labels exactly as the i18n tables", (_name, file) => {
    const failures: string[] = [];
    for (const row of labelRows(file).rows) {
      if (row.cells.length !== 3) continue;
      const [e, z, j] = row.cells;
      const shown = `en=${JSON.stringify(e)} zh-CN=${JSON.stringify(z)} ja=${JSON.stringify(j)}`;
      if (row.cells.some((c) => placeholder.test(c))) {
        failures.push(`${row.file}:${row.line}: a labels table may hold only messages without {placeholders}: ${shown}`);
        continue;
      }
      const match = Object.keys(zh).some((key) => en[key] === e && zh[key] === z && ja[key] === j);
      if (!match) failures.push(`${row.file}:${row.line}: no message key has ${shown} (${explainMismatch(row)})`);
    }
    expect(failures).toEqual([]);
  });
});
