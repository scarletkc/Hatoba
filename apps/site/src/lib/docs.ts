import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { REPO_URL } from "../consts";

/** The built-in skill, whose reference files are the user docs. */
export const SKILL_DIR = "crates/hatoba-ai/src/skills/builtin/hatoba";
export const OVERVIEW = "overview";

/** The sidebar. A reference file missing here goes under "More"; docs.test.ts requires each to be listed. */
export const DOC_GROUPS: { title: string; slugs: string[] }[] = [
  { title: "Get started", slugs: [OVERVIEW, "hosts-and-connecting", "settings", "shortcuts"] },
  { title: "Terminal and files", slugs: ["terminal", "sftp", "port-forwarding", "dashboard"] },
  { title: "Keys and vault", slugs: ["keys-vault-and-lock"] },
  { title: "Sync", slugs: ["sync", "sync-worker"] },
  {
    title: "AI assistant",
    slugs: ["ai-panel", "ai-tools", "ai-file-changes", "ai-attachments", "ai-history", "ai-side-questions", "ai-models", "ai-settings", "ai-chatgpt", "ai-instructions", "ai-mcp"],
  },
  { title: "Troubleshooting", slugs: ["troubleshooting", "troubleshooting-connections", "troubleshooting-sync", "troubleshooting-ai"] },
];

/** The sections of SKILL.md that describe Hatoba to its users, which the overview shows after the introduction. */
const OVERVIEW_SECTIONS = ["Window layout", "Not in Hatoba (current version)"];

export interface Doc {
  slug: string;
  title: string;
  description: string;
  body: string;
  editUrl: string;
}

/** Where the docs come from: a release tag, or the working tree. */
export interface DocsSource {
  ref: string | null;
  read(path: string): string;
  list(dir: string): string[];
}

export function repoRoot(): string {
  return execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
}

export function workingTree(root = repoRoot()): DocsSource {
  return {
    ref: null,
    read: (path) => readFileSync(join(root, path), "utf8"),
    list: (dir) => readdirSync(join(root, dir)),
  };
}

export function gitRef(ref: string, root = repoRoot()): DocsSource {
  const git = (...args: string[]) => execFileSync("git", args, { cwd: root, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  git("rev-parse", "--verify", "--quiet", `${ref}^{commit}`);
  return {
    ref,
    read: (path) => git("show", `${ref}:${path}`),
    list: (dir) => git("ls-tree", "--name-only", `${ref}:${dir}`).split("\n").filter(Boolean),
  };
}

/** Plain text of a Markdown paragraph, shortened to a meta description. */
export function describe(markdown: string, limit = 160): string {
  const text = markdown
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/[*_`]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (text.length <= limit) return text;
  const cut = text.slice(0, limit - 1);
  const sentence = cut.lastIndexOf(". ");
  if (sentence > limit / 2) return cut.slice(0, sentence + 1);
  return `${cut.slice(0, cut.lastIndexOf(" "))}…`;
}

function firstParagraph(body: string): string {
  return body.split(/\n\s*\n/).find((block) => block.trim() && !/^\s*[#|>-]/.test(block)) ?? "";
}

/** Splits a Markdown file into its `# ` title and the rest. */
function splitTitle(markdown: string, path: string): { title: string; body: string } {
  const match = /^# (.+)\n/m.exec(markdown);
  if (!match) throw new Error(`${path} has no '# ' title.`);
  return { title: match[1].trim(), body: markdown.slice(match.index + match[0].length).trim() };
}

/** The `## ` sections of `markdown` named in `names`, in their order there. */
function sections(markdown: string, names: string[], path: string): string[] {
  const parts = markdown.split(/^(?=## )/m);
  return names.map((name) => {
    const part = parts.find((candidate) => candidate.startsWith(`## ${name}\n`));
    if (!part) throw new Error(`${path} has no '## ${name}' section, which the website's docs overview shows.`);
    return part.trim();
  });
}

const LABEL_HEADER = /^\| en \| zh-CN \| ja \|$/gm;

/**
 * Turns the skill's Markdown into the site's: mentions of `references/<file>.md` become links to
 * the page, and the language columns of the label tables are named in full.
 */
export function adapt(markdown: string, titles: Map<string, string>, version: string): string {
  return markdown
    .replace(/`references\/([a-z0-9-]+)\.md`/g, (mention, slug: string) => {
      const title = titles.get(slug);
      return title ? `[${title}](${docPath(slug)})` : mention;
    })
    .replace(LABEL_HEADER, "| English | 简体中文 | 日本語 |")
    .replaceAll("{{HATOBA_VERSION}}", version);
}

export function docPath(slug: string): string {
  return slug === OVERVIEW ? "/docs/" : `/docs/${slug}/`;
}

/** Reads the docs: the overview from SKILL.md and a page for each reference file. */
export function readDocs(source: DocsSource, version: string): Doc[] {
  const editUrl = (path: string) => `${REPO_URL}/edit/main/${path}`;
  const references = source
    .list(`${SKILL_DIR}/references`)
    .filter((name) => name.endsWith(".md"))
    .map((name) => {
      const path = `${SKILL_DIR}/references/${name}`;
      return { slug: name.slice(0, -3), path, ...splitTitle(source.read(path), path) };
    });

  const skillPath = `${SKILL_DIR}/SKILL.md`;
  const skill = source.read(skillPath).replace(/^---\n[\s\S]*?\n---\n/, "");
  const { body: skillBody } = splitTitle(skill, skillPath);
  const intro = skillBody.split(/^## /m)[0].trim();
  const overview = { slug: OVERVIEW, path: skillPath, title: "Overview", body: [intro, ...sections(skillBody, OVERVIEW_SECTIONS, skillPath)].join("\n\n") };

  const pages = [overview, ...references];
  const titles = new Map(pages.map((page) => [page.slug, page.title]));
  return pages.map(({ slug, path, title, body }) => {
    const adapted = adapt(body, titles, version);
    return { slug, title, description: describe(firstParagraph(adapted)), body: adapted, editUrl: editUrl(path) };
  });
}

/** The sidebar for `docs`: DOC_GROUPS with the pages that exist, then any other page under "More". */
export function sidebar(docs: { slug: string; title: string }[]): { title: string; pages: { slug: string; title: string }[] }[] {
  const bySlug = new Map(docs.map((doc) => [doc.slug, doc]));
  const listed = new Set(DOC_GROUPS.flatMap((group) => group.slugs));
  const groups = DOC_GROUPS.map((group) => ({
    title: group.title,
    pages: group.slugs.flatMap((slug) => {
      const doc = bySlug.get(slug);
      return doc ? [{ slug, title: doc.title.replace(/^Troubleshooting: (.)/, (_, first: string) => first.toUpperCase()) }] : [];
    }),
  }));
  const more = docs.filter((doc) => !listed.has(doc.slug));
  if (more.length > 0) groups.push({ title: "More", pages: more.map(({ slug, title }) => ({ slug, title })) });
  return groups.filter((group) => group.pages.length > 0);
}
