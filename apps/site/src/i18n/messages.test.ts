import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { repoRoot } from "../lib/docs";
import { en } from "./en";
import { ja } from "./ja";
import { zh } from "./zh";

const root = repoRoot();
const read = (path: string) => readFileSync(join(root, path), "utf8");

describe("the description of Hatoba", () => {
  it("is the one the README, package.json, and the app share", () => {
    expect(JSON.parse(read("package.json")).description).toBe(en.meta.description);
    expect(JSON.parse(read("apps/desktop/src-tauri/tauri.conf.json")).bundle.shortDescription).toBe(en.meta.description);
    expect(read("README.md")).toContain(`**${en.meta.description}**`);
  });

  it("is translated whole", () => {
    for (const messages of [zh, ja]) {
      for (const word of ["SSH", "AI", "Cloudflare"]) expect(messages.meta.description).toContain(word);
    }
  });
});

describe("the translations", () => {
  it("have as many points and items as the English", () => {
    for (const messages of [zh, ja]) {
      expect(messages.ai.points).toHaveLength(en.ai.points.length);
      expect(messages.sync.points).toHaveLength(en.sync.points.length);
      expect(messages.essentials.items).toHaveLength(en.essentials.items.length);
      expect(Object.keys(messages.download.files).sort()).toEqual(Object.keys(en.download.files).sort());
    }
  });
});
