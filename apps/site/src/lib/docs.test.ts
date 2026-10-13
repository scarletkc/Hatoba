import { describe as group, expect, it } from "vitest";
import { DOC_GROUPS, OVERVIEW, SKILL_DIR, adapt, describe, readDocs, sidebar, workingTree } from "./docs";

const source = workingTree();
const references = source
  .list(`${SKILL_DIR}/references`)
  .filter((name) => name.endsWith(".md"))
  .map((name) => name.slice(0, -3));

group("the docs", () => {
  it("list every reference file in the sidebar, once", () => {
    const listed = DOC_GROUPS.flatMap((g) => g.slugs);
    expect(new Set(listed).size).toBe(listed.length);
    expect(listed.filter((slug) => slug !== OVERVIEW).sort()).toEqual([...references].sort());
  });

  it("link the pages they mention, and describe each page", () => {
    const docs = readDocs(source, "1.2.3");
    expect(docs.map((doc) => doc.slug).sort()).toEqual([OVERVIEW, ...references].sort());
    for (const doc of docs) {
      expect(doc.body, doc.slug).not.toMatch(/`references\/[a-z0-9-]+\.md`/);
      expect(doc.body, doc.slug).not.toContain("{{HATOBA_VERSION}}");
      expect(doc.description.length, doc.slug).toBeGreaterThan(20);
      expect(doc.description.length, doc.slug).toBeLessThanOrEqual(160);
    }
    const overview = docs.find((doc) => doc.slug === OVERVIEW)!;
    expect(overview.body).toContain("## Window layout");
    expect(overview.body).toContain("## Not in Hatoba");
    expect(overview.body).not.toContain("## How to answer");
  });

  it("put a page missing from the sidebar under More", () => {
    const groups = sidebar([{ slug: "sftp", title: "SFTP file panel" }, { slug: "new-page", title: "New page" }]);
    expect(groups.map((g) => g.title)).toEqual(["Terminal and files", "More"]);
  });
});

group("adapt", () => {
  it("turns mentions into links and names the label columns", () => {
    const titles = new Map([["ai-panel", "AI assistant panel"]]);
    const text = adapt("See `references/ai-panel.md` and `references/gone.md`.\n\n| en | zh-CN | ja |\n|---|---|---|", titles, "1.2.3");
    expect(text).toBe("See [AI assistant panel](/docs/ai-panel/) and `references/gone.md`.\n\n| English | 简体中文 | 日本語 |\n|---|---|---|");
  });
});

group("describe", () => {
  it("keeps a short paragraph and strips Markdown", () => {
    expect(describe("Open **SFTP** with [the button](/x/) and `Ctrl+O`.")).toBe("Open SFTP with the button and Ctrl+O.");
  });

  it("cuts a long paragraph at a sentence or a word", () => {
    const sentence = "A sentence that is long enough to count. ";
    expect(describe(sentence.repeat(10))).toMatch(/count\.$/);
    expect(describe("word ".repeat(60)).endsWith("…")).toBe(true);
  });
});
