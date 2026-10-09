import { describe, expect, it } from "vitest";
import type { SkillDetail, SkillView } from "@/ipc/types";
import {
  SKILL_FILE_MAX_BYTES,
  byteLength,
  describeIssue,
  descriptionLength,
  draftFromDetail,
  hasProblems,
  isValidSkillName,
  nameProblemKey,
  normalizeSkillPath,
  sameDraft,
  skillNameProblem,
  sortSkills,
  sourceName,
  suggestSkillName,
  toSkillInput,
  upsertSkill,
  validateSkill,
  type SkillDraft,
} from "./skillsLogic";

const draft = (extra: Partial<SkillDraft> = {}): SkillDraft => ({
  name: "nginx-ops",
  description: "Operate nginx.",
  body: "# Steps",
  files: [],
  ...extra,
});

const view = (id: string, name: string): SkillView => ({ id, name, description: "d", enabled: true, files: [], updated_at: 1 });

describe("skill names", () => {
  it("accepts 1 to 64 lowercase letters, digits and hyphens", () => {
    expect(isValidSkillName("a")).toBe(true);
    expect(isValidSkillName("nginx-ops-2")).toBe(true);
    expect(isValidSkillName("a".repeat(64))).toBe(true);
    expect(isValidSkillName("a".repeat(65))).toBe(false);
  });

  it("rejects everything else", () => {
    for (const bad of ["", "Nginx", "nginx_ops", "nginx ops", "nginx.ops", "日本語", "-é"]) expect(isValidSkillName(bad)).toBe(false);
  });

  it("tells empty, invalid, and taken names apart", () => {
    expect(skillNameProblem("")).toBe("empty");
    expect(skillNameProblem("Bad Name")).toBe("invalid");
    expect(skillNameProblem("nginx-ops", ["nginx-ops"])).toBe("taken");
    expect(skillNameProblem("nginx-ops", ["other"])).toBeNull();
  });

  it("keeps the built-in skill's name for it (AI-34), even when no saved skill has it", () => {
    expect(skillNameProblem("hatoba")).toBe("reserved");
    expect(skillNameProblem("hatoba", ["hatoba"])).toBe("reserved");
    expect(skillNameProblem("hatoba-notes")).toBeNull();
    expect(suggestSkillName("hatoba", [])).toBe("hatoba-2");
    expect(nameProblemKey("reserved")).toBe("nameReserved");
  });

  it("suggests the first free numbered name", () => {
    expect(suggestSkillName("nginx-ops", ["nginx-ops"])).toBe("nginx-ops-2");
    expect(suggestSkillName("nginx-ops", ["nginx-ops", "nginx-ops-2", "nginx-ops-3"])).toBe("nginx-ops-4");
  });

  it("keeps a suggestion inside 64 characters and turns an invalid name into a valid one", () => {
    const long = "a".repeat(64);
    const suggested = suggestSkillName(long, [long]);
    expect(suggested).toBe(`${"a".repeat(62)}-2`);
    expect(isValidSkillName(suggested)).toBe(true);
    expect(suggestSkillName("Nginx Ops!", [])).toBe("nginx-ops-2");
    expect(suggestSkillName("", [])).toBe("skill-2");
  });
});

describe("lengths", () => {
  it("counts the description in characters, not UTF-16 units, without surrounding space", () => {
    expect(descriptionLength("  abc  ")).toBe(3);
    expect(descriptionLength("日本語")).toBe(3);
    expect(descriptionLength("😀😀")).toBe(2);
  });

  it("counts file sizes in UTF-8 bytes", () => {
    expect(byteLength("abc")).toBe(3);
    expect(byteLength("日本")).toBe(6);
    expect(byteLength("😀")).toBe(4);
  });
});

describe("normalizeSkillPath", () => {
  it("normalizes separators and drops empty and dot parts", () => {
    expect(normalizeSkillPath("references/nginx.md")).toBe("references/nginx.md");
    expect(normalizeSkillPath("references\\nginx.md")).toBe("references/nginx.md");
    expect(normalizeSkillPath("./references//nginx.md")).toBe("references/nginx.md");
  });

  it("rejects paths that leave the folder", () => {
    for (const bad of ["../x", "a/../b", "/etc/passwd", "\\\\host\\share\\x", "C:\\x", "c:/x", "", ".", "a\u0000b", "a\nb"]) {
      expect(normalizeSkillPath(bad)).toBeNull();
    }
  });

  it("rejects paths that are too deep or too long", () => {
    expect(normalizeSkillPath(Array.from({ length: 10 }, () => "a").join("/"))).not.toBeNull();
    expect(normalizeSkillPath(Array.from({ length: 11 }, () => "a").join("/"))).toBeNull();
    expect(normalizeSkillPath("a".repeat(257))).toBeNull();
  });
});

describe("validateSkill", () => {
  it("accepts a complete skill", () => {
    expect(hasProblems(validateSkill(draft()))).toBe(false);
  });

  it("flags the name, with the names of other skills taken", () => {
    expect(validateSkill(draft({ name: "" })).name).toBe("empty");
    expect(validateSkill(draft({ name: "Nginx" })).name).toBe("invalid");
    expect(validateSkill(draft(), ["nginx-ops"]).name).toBe("taken");
  });

  it("requires a description of at most 1,024 characters", () => {
    expect(validateSkill(draft({ description: "  " })).description).toBe("empty");
    expect(validateSkill(draft({ description: "x".repeat(1024) })).description).toBeNull();
    expect(validateSkill(draft({ description: "x".repeat(1025) })).description).toBe("too_long");
  });

  it("limits the body and each file to 32 KB", () => {
    const atLimit = "x".repeat(SKILL_FILE_MAX_BYTES);
    expect(validateSkill(draft({ body: atLimit })).body).toBeNull();
    expect(validateSkill(draft({ body: `${atLimit}x` })).body).toBe("too_large");
    const p = validateSkill(draft({ files: [{ uid: 1, path: "a.md", content: `${atLimit}x` }] }));
    expect(p.files).toEqual(["too_large"]);
    // Bytes, not characters: 11,000 three-byte characters are over the limit.
    expect(validateSkill(draft({ body: "日".repeat(11_000) })).body).toBe("too_large");
  });

  it("flags empty, unsafe, SKILL.md, and repeated file paths", () => {
    const files = [
      { uid: 1, path: "", content: "" },
      { uid: 2, path: "../x.md", content: "" },
      { uid: 3, path: "skill.md", content: "" },
      { uid: 4, path: "references/a.md", content: "" },
      { uid: 5, path: "references\\A.md", content: "" },
    ];
    expect(validateSkill(draft({ files })).files).toEqual(["path_empty", "path_invalid", "path_skill_md", null, "path_duplicate"]);
  });

  it("flags too many files", () => {
    const files = Array.from({ length: 200 }, (_, i) => ({ uid: i, path: `f${i}.md`, content: "" }));
    expect(validateSkill(draft({ files })).tooManyFiles).toBe(true);
    expect(validateSkill(draft({ files: files.slice(1) })).tooManyFiles).toBe(false);
  });

  it("flags more than 5 MB in total", () => {
    const content = "x".repeat(SKILL_FILE_MAX_BYTES);
    const files = Array.from({ length: 170 }, (_, i) => ({ uid: i, path: `f${i}.md`, content }));
    expect(validateSkill(draft({ files })).tooLarge).toBe(true);
  });
});

describe("toSkillInput", () => {
  it("trims the name and description and normalizes the file paths", () => {
    const input = toSkillInput("s1", false, draft({ name: " nginx-ops ", description: " Operate. ", files: [{ uid: 1, path: " references\\a.md ", content: "x" }] }));
    expect(input).toEqual({
      id: "s1",
      name: "nginx-ops",
      description: "Operate.",
      enabled: false,
      body: "# Steps",
      files: [{ path: "references/a.md", content: "x" }],
    });
  });

  it("leaves the body exactly as typed", () => {
    expect(toSkillInput(null, true, draft({ body: "  a\r\n\r\n" })).body).toBe("  a\r\n\r\n");
  });
});

describe("drafts", () => {
  const detail: SkillDetail = {
    skill: { ...view("s1", "nginx-ops"), files: ["a.md"] },
    body: "# Steps",
    files: [{ path: "a.md", content: "A" }],
    frontmatter_keys: ["license"],
  };

  it("builds the form from a saved skill", () => {
    expect(draftFromDetail(detail)).toMatchObject({ name: "nginx-ops", body: "# Steps", files: [{ path: "a.md", content: "A" }] });
  });

  it("compares text, not row identities", () => {
    expect(sameDraft(draftFromDetail(detail), draftFromDetail(detail))).toBe(true);
    const changed = draftFromDetail(detail);
    changed.files[0].content = "B";
    expect(sameDraft(draftFromDetail(detail), changed)).toBe(false);
    expect(sameDraft(draftFromDetail(detail), { ...draftFromDetail(detail), files: [] })).toBe(false);
  });
});

describe("describeIssue", () => {
  it("names the placeholders of each issue", () => {
    expect(describeIssue({ kind: "missing_skill_md" })).toEqual({ kind: "missing_skill_md", params: {} });
    expect(describeIssue({ kind: "invalid_name", name: "Bad" }).params).toEqual({ name: "Bad" });
    expect(describeIssue({ kind: "description_too_long", chars: 1300 }).params).toEqual({ chars: 1300, max: 1024 });
    expect(describeIssue({ kind: "file_too_large", path: "a.md", size: 50_000 }).params).toEqual({ path: "a.md", size: 50_000, max: 32_768 });
    expect(describeIssue({ kind: "too_many_files", count: 250 }).params).toEqual({ count: 250, max: 200 });
    expect(describeIssue({ kind: "too_large", bytes: 6_000_000 }).params).toEqual({ bytes: 6_000_000, max: 5 * 1024 * 1024 });
  });
});

describe("the list", () => {
  it("sorts by name", () => {
    expect(sortSkills([view("2", "b"), view("1", "a")]).map((s) => s.name)).toEqual(["a", "b"]);
  });

  it("replaces a skill in place or adds it", () => {
    const list = [view("1", "a"), view("2", "c")];
    expect(upsertSkill(list, { ...view("2", "c"), description: "new" })[1].description).toBe("new");
    expect(upsertSkill(list, view("3", "b")).map((s) => s.name)).toEqual(["a", "b", "c"]);
  });

  it("names an import after its folder or archive", () => {
    expect(sourceName("C:\\Users\\kc\\skills\\docker-compose")).toBe("docker-compose");
    expect(sourceName("/home/kc/Downloads/nginx-ops.zip")).toBe("nginx-ops");
    expect(sourceName("/home/kc/skills/")).toBe("skills");
  });
});
