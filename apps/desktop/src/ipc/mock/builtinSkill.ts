import type { BuiltinSkillView, SkillFileView } from "../types";

/**
 * The built-in `hatoba` skill (AI-34) for the browser mock: the crate's own files, which Vite reads as text the
 * first time the skill is shown. Like Rust, the version goes in place of `{{HATOBA_VERSION}}`.
 */
const files = import.meta.glob<string>("../../../../../crates/hatoba-ai/src/skills/builtin/hatoba/**/*.md", { query: "?raw", import: "default" });

const PLACEHOLDER = /\{\{HATOBA_VERSION\}\}/g;

export async function builtinSkill(version: string, enabled: boolean): Promise<BuiltinSkillView> {
  const read: SkillFileView[] = await Promise.all(
    Object.entries(files).map(async ([key, load]) => ({
      path: key.slice(key.indexOf("/builtin/hatoba/") + "/builtin/hatoba/".length),
      content: (await load()).replace(PLACEHOLDER, version),
    })),
  );
  const skillMd = read.find((f) => f.path === "SKILL.md")?.content ?? "";
  // The frontmatter is `---` lines around YAML; its description is a quoted string.
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n([\s\S]*)$/.exec(skillMd);
  const yaml = m?.[1] ?? "";
  const quoted = /^description:\s*(".*")\s*$/m.exec(yaml)?.[1];
  let description = /^description:\s*(.*)$/m.exec(yaml)?.[1] ?? "";
  try {
    if (quoted) description = JSON.parse(quoted) as string;
  } catch {
    // keep the raw line
  }
  return {
    name: "hatoba",
    description,
    enabled,
    body: m?.[2] ?? skillMd,
    files: read.filter((f) => f.path !== "SKILL.md").sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0)),
  };
}
