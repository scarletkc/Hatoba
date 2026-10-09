import type { SkillDetail, SkillFileView, SkillInput, SkillIssue, SkillView } from "@/ipc/types";

/* Pure logic behind the skills section of Settings → AI (spec §13.8, AI-27): no React, no i18n. */

export const SKILL_NAME_MAX = 64;
export const SKILL_DESCRIPTION_MAX = 1024;
/** The limit of every file, `SKILL.md` included, in bytes of UTF-8. */
export const SKILL_FILE_MAX_BYTES = 32 * 1024;
/** Files in one skill, `SKILL.md` included. */
export const SKILL_MAX_FILES = 200;
export const SKILL_MAX_TOTAL_BYTES = 5 * 1024 * 1024;

const SKILL_MD = "SKILL.md";
const MAX_PATH_DEPTH = 10;
const MAX_PATH_BYTES = 256;

// ───────────────────────── Name and description ─────────────────────────

const NAME_RULE = /^[a-z0-9-]{1,64}$/;

/** The built-in skill's name (AI-34), which no skill of the user's may take. */
export const BUILTIN_SKILL_NAME = "hatoba";

export const isValidSkillName = (name: string): boolean => NAME_RULE.test(name);

export type NameProblem = "empty" | "invalid" | "reserved" | "taken";

/** Why `name` can't be used for a skill, or null. `taken` lists the names of the other saved skills. */
export function skillNameProblem(name: string, taken: readonly string[] = []): NameProblem | null {
  if (name === "") return "empty";
  if (!isValidSkillName(name)) return "invalid";
  if (name === BUILTIN_SKILL_NAME) return "reserved";
  return taken.includes(name) ? "taken" : null;
}

/** The message key suffix of a name problem: `aiSettings.skills.err.<suffix>`. */
export function nameProblemKey(problem: NameProblem): "nameRequired" | "nameInvalid" | "nameReserved" | "nameTaken" {
  switch (problem) {
    case "empty":
      return "nameRequired";
    case "invalid":
      return "nameInvalid";
    case "reserved":
      return "nameReserved";
    case "taken":
      return "nameTaken";
  }
}

/** The first unused name built from `name`: `nginx-ops` becomes `nginx-ops-2`, then `-3`. Valid names only. */
export function suggestSkillName(name: string, taken: readonly string[]): string {
  const base = isValidSkillName(name) ? name : name.toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "") || "skill";
  for (let n = 2; ; n++) {
    const suffix = `-${n}`;
    const candidate = `${base.slice(0, SKILL_NAME_MAX - suffix.length)}${suffix}`;
    if (!taken.includes(candidate)) return candidate;
  }
}

/** Characters as the backend counts them (code points), without surrounding whitespace. */
export const descriptionLength = (text: string): number => Array.from(text.trim()).length;

const encoder = new TextEncoder();
export const byteLength = (text: string): number => encoder.encode(text).length;

// ───────────────────────── File paths ─────────────────────────

/**
 * The path as the backend stores it: forward slashes, `.` and empty parts dropped. Null when it is not a safe
 * relative path: a `..` part, a root, a drive (`C:`), a control character, or beyond the depth and length limits.
 */
export function normalizeSkillPath(raw: string): string | null {
  if (/[\u0000-\u001f\u007f-\u009f]/.test(raw)) return null;
  const unified = raw.replace(/\\/g, "/");
  if (unified.startsWith("/")) return null;
  const parts: string[] = [];
  for (const part of unified.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") return null;
    if (parts.length === 0 && /^[A-Za-z]:/.test(part)) return null;
    parts.push(part);
  }
  if (parts.length === 0 || parts.length > MAX_PATH_DEPTH) return null;
  const path = parts.join("/");
  return byteLength(path) <= MAX_PATH_BYTES ? path : null;
}

// ───────────────────────── The edit form ─────────────────────────

export interface SkillFileDraft {
  /** Identifies the row while the path is being edited. */
  uid: number;
  path: string;
  content: string;
}

export interface SkillDraft {
  name: string;
  description: string;
  body: string;
  files: SkillFileDraft[];
}

let nextUid = 1;
export const newUid = (): number => nextUid++;

export const emptyDraft = (): SkillDraft => ({ name: "", description: "", body: "", files: [] });

export function draftFromDetail(detail: SkillDetail): SkillDraft {
  return {
    name: detail.skill.name,
    description: detail.skill.description,
    body: detail.body,
    files: detail.files.map((f) => ({ uid: newUid(), path: f.path, content: f.content })),
  };
}

/** Whether two drafts hold the same text (the row identities do not count). */
export function sameDraft(a: SkillDraft, b: SkillDraft): boolean {
  return (
    a.name === b.name &&
    a.description === b.description &&
    a.body === b.body &&
    a.files.length === b.files.length &&
    a.files.every((f, i) => f.path === b.files[i].path && f.content === b.files[i].content)
  );
}

export type FileProblem = "path_empty" | "path_invalid" | "path_skill_md" | "path_duplicate" | "too_large";

export interface SkillProblems {
  name: NameProblem | null;
  description: "empty" | "too_long" | null;
  body: "too_large" | null;
  /** Index for index with the draft's files. */
  files: (FileProblem | null)[];
  tooManyFiles: boolean;
  tooLarge: boolean;
}

export function validateSkill(draft: SkillDraft, takenNames: readonly string[] = []): SkillProblems {
  const length = descriptionLength(draft.description);
  const seen = new Set<string>();
  const files = draft.files.map((f): FileProblem | null => {
    if (f.path.trim() === "") return "path_empty";
    const path = normalizeSkillPath(f.path.trim());
    if (path === null) return "path_invalid";
    if (path.toLowerCase() === SKILL_MD.toLowerCase()) return "path_skill_md";
    // Case-insensitive, because the skill is exported to systems whose file names are.
    const key = path.toLowerCase();
    if (seen.has(key)) return "path_duplicate";
    seen.add(key);
    return byteLength(f.content) > SKILL_FILE_MAX_BYTES ? "too_large" : null;
  });
  const total = byteLength(draft.body) + draft.files.reduce((sum, f) => sum + byteLength(f.content), 0);
  return {
    name: skillNameProblem(draft.name, takenNames),
    description: length === 0 ? "empty" : length > SKILL_DESCRIPTION_MAX ? "too_long" : null,
    body: byteLength(draft.body) > SKILL_FILE_MAX_BYTES ? "too_large" : null,
    files,
    tooManyFiles: draft.files.length + 1 > SKILL_MAX_FILES,
    tooLarge: total > SKILL_MAX_TOTAL_BYTES,
  };
}

export function hasProblems(p: SkillProblems): boolean {
  return !!p.name || !!p.description || !!p.body || p.files.some(Boolean) || p.tooManyFiles || p.tooLarge;
}

/** What `skill_save` takes: trimmed name, description and paths, as the backend checks them. */
export function toSkillInput(id: string | null, enabled: boolean, draft: SkillDraft): SkillInput {
  return {
    id,
    name: draft.name.trim(),
    description: draft.description.trim(),
    enabled,
    body: draft.body,
    files: draft.files.map((f): SkillFileView => ({ path: normalizeSkillPath(f.path.trim()) ?? f.path.trim(), content: f.content })),
  };
}

// ───────────────────────── Import issues (AI-27) ─────────────────────────

/** The message key suffix and the placeholders of an issue; the key is `aiSettings.skills.issue.<kind>`. */
export function describeIssue(issue: SkillIssue): { kind: SkillIssue["kind"]; params: Record<string, string | number> } {
  switch (issue.kind) {
    case "invalid_frontmatter":
      return { kind: issue.kind, params: { detail: issue.detail } };
    case "invalid_name":
      return { kind: issue.kind, params: { name: issue.name } };
    case "description_too_long":
      return { kind: issue.kind, params: { chars: issue.chars, max: SKILL_DESCRIPTION_MAX } };
    case "file_too_large":
      return { kind: issue.kind, params: { path: issue.path, size: issue.size, max: SKILL_FILE_MAX_BYTES } };
    case "unsafe_path":
      return { kind: issue.kind, params: { path: issue.path } };
    case "too_many_files":
      return { kind: issue.kind, params: { count: issue.count, max: SKILL_MAX_FILES } };
    case "too_large":
      return { kind: issue.kind, params: { bytes: issue.bytes, max: SKILL_MAX_TOTAL_BYTES } };
    default:
      return { kind: issue.kind, params: {} };
  }
}

// ───────────────────────── The list ─────────────────────────

/** Skills by name, so the list does not move when one is edited. */
export function sortSkills(list: readonly SkillView[]): SkillView[] {
  return [...list].sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
}

/** Replaces the skill with the same ID, or adds it. */
export function upsertSkill(list: readonly SkillView[], skill: SkillView): SkillView[] {
  return sortSkills(list.some((s) => s.id === skill.id) ? list.map((s) => (s.id === skill.id ? skill : s)) : [...list, skill]);
}

/** The last part of a picked folder or `.zip` path, without the extension, for the title of the import. */
export function sourceName(path: string): string {
  const last = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  return last.replace(/\.zip$/i, "");
}
