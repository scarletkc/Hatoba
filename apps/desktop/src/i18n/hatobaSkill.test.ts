import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { AREAS, labelFiles } from "./hatobaSkillLabels";
import { messages } from "./locales";

/**
 * The built-in `hatoba` skill (crates/hatoba-ai/src/skills/builtin/hatoba) describes the current UI to the
 * AI assistant. It names labels in English, and its `references/labels/` files, written from the message
 * tables, give them in the other languages. A renamed or reworded label fails here until those files are
 * written again (docs/development.md).
 */

const SKILL_DIR = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../crates/hatoba-ai/src/skills/builtin/hatoba");
/** `read_skill` shortens a longer file, so each one stays under this. */
const MAX_CHARS = 15_000;
/** SKILL.md descriptions are limited to this many characters (spec AI-27). */
const MAX_DESCRIPTION_CHARS = 1_024;
const LABELS = "references/labels";

interface SkillFile {
  /** Path relative to the skill folder, with forward slashes. */
  name: string;
  text: string;
}

function readDir(dir: string): SkillFile[] {
  if (!existsSync(resolve(SKILL_DIR, dir))) return [];
  return readdirSync(resolve(SKILL_DIR, dir), { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".md"))
    .map((entry) => ({ name: `${dir}/${entry.name}`, text: readFileSync(resolve(SKILL_DIR, dir, entry.name), "utf8") }));
}

const files: SkillFile[] = [
  { name: "SKILL.md", text: readFileSync(resolve(SKILL_DIR, "SKILL.md"), "utf8") },
  ...readDir("references"),
  ...readDir(LABELS),
];
const referenceFiles = files.filter((f) => f.name !== "SKILL.md");
const generated = labelFiles();
const mentions = (text: string) => new Set(text.match(/references\/[A-Za-z0-9_./-]+\.md/g) ?? []);

describe("built-in hatoba skill", () => {
  it("has SKILL.md and reference files", () => {
    expect(files[0].name).toBe("SKILL.md");
    expect(referenceFiles.length).toBeGreaterThan(0);
  });

  it("has frontmatter with the name hatoba and a description", () => {
    const skill = files[0].text.replace(/^\ufeff/, "").split(/\r?\n/);
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

  it("keeps the app version placeholder in SKILL.md, outside the label files", () => {
    expect(files[0].text).toContain("{{HATOBA_VERSION}}");
    expect(generated.filter((f) => f.text.includes("{{")).map((f) => f.path)).toEqual([]);
  });

  it.each(files.map((f) => [f.name, f] as const))("%s stays under the character limit", (_name, file) => {
    const chars = [...file.text].length;
    expect(chars, `${file.name} has ${chars} characters; read_skill shortens a file over ${MAX_CHARS}`).toBeLessThan(MAX_CHARS);
  });

  it("links every reference file from SKILL.md, and links only existing files", () => {
    const linked = mentions(files[0].text);
    const present = new Set(referenceFiles.map((f) => f.name));
    expect([...present].filter((n) => !linked.has(n)).map((n) => `SKILL.md does not mention ${n}`)).toEqual([]);
    expect([...linked].filter((n) => !present.has(n)).map((n) => `SKILL.md mentions ${n}, which does not exist`)).toEqual([]);

    const dangling: string[] = [];
    for (const f of referenceFiles) {
      for (const n of mentions(f.text)) {
        if (!present.has(n)) dangling.push(`${f.name} mentions ${n}, which does not exist`);
      }
    }
    expect(dangling).toEqual([]);
  });

  it("puts label tables only in the label files", () => {
    const tables = files.filter((f) => !f.name.startsWith(`${LABELS}/`) && /^\|\s*en\s*\|/m.test(f.text)).map((f) => f.name);
    expect(tables, "name labels in English; the other languages are in the label files").toEqual([]);
  });
});

describe("the skill's label files", () => {
  const sections = AREAS.flatMap((area) => Object.entries(area.files).flatMap(([file, s]) => Object.entries(s).map(([heading, keys]) => ({ file, heading, keys }))));

  it("cover each skill file once, by sections it has", () => {
    const covered = AREAS.flatMap((area) => Object.keys(area.files));
    expect(covered.filter((file, i) => covered.indexOf(file) !== i)).toEqual([]);
    const problems: string[] = [];
    for (const { file, heading } of sections) {
      const text = files.find((f) => f.name === file)?.text;
      if (text === undefined) problems.push(`${file} does not exist`);
      else if (heading && !text.split(/\r?\n/).includes(`## ${heading}`)) problems.push(`${file} has no "## ${heading}" section`);
    }
    expect(problems).toEqual([]);
  });

  it("quote only messages without a | that would end the table cell", () => {
    const keys = sections.flatMap((s) => s.keys);
    const unquotable = keys.filter((key) => Object.values(messages).some((table) => table[key].includes("|")));
    expect(unquotable).toEqual([]);
  });

  // `pnpm --filter @hatoba/desktop exec vitest run -u src/i18n/hatobaSkill.test.ts` writes them again.
  it.each(generated.map((f) => [f.path, f] as const))("%s matches the message tables", async (path, file) => {
    await expect(file.text).toMatchFileSnapshot(resolve(SKILL_DIR, path));
  });

  it("are all written from the message tables", () => {
    const paths = new Set(generated.map((f) => f.path));
    expect(readDir(LABELS).map((f) => f.name).filter((name) => !paths.has(name))).toEqual([]);
  });
});
