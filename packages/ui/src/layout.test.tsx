import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { EmptyState, wordRuns } from "./layout";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

describe("wordRuns", () => {
  it("leaves text without CJK in one run", () => {
    expect(wordRuns("Add your first server, or import from ~/.ssh/config.", "en")).toEqual([
      "Add your first server, or import from ~/.ssh/config.",
    ]);
  });

  it("breaks Chinese between words, never inside one", () => {
    const runs = wordRuns("添加第一台服务器，或从 ~/.ssh/config 或其他 SSH 客户端导入。", "zh-CN");
    expect(runs.join("")).toBe("添加第一台服务器，或从 ~/.ssh/config 或其他 SSH 客户端导入。");
    for (const word of ["服务器", "客户端", "导入"]) expect(runs.some((r) => r.includes(word))).toBe(true);
    expect(runs.length).toBeGreaterThan(1);
  });

  it("keeps Japanese particles, endings and katakana words with the word before them", () => {
    const runs = wordRuns("新しい鍵を生成するか、既存の鍵をインポートします。", "ja");
    expect(runs.join("")).toBe("新しい鍵を生成するか、既存の鍵をインポートします。");
    expect(runs.length).toBeGreaterThan(1);
    expect(runs.some((r) => r.includes("インポートします"))).toBe(true);
    expect(runs.every((r) => !/^[\p{Script=Hiragana}]/u.test(r))).toBe(true);
  });
});

describe("EmptyState", () => {
  it("offers a line break only between CJK words, and keeps the text whole", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    act(() => root.render(<EmptyState icon="key" title="还没有密钥" body="生成一把新密钥，或导入已有的密钥。" />));
    unmount = () => {
      root.unmount();
      host.remove();
    };
    expect(host.textContent).toBe("还没有密钥生成一把新密钥，或导入已有的密钥。");
    expect(host.querySelectorAll("wbr").length).toBeGreaterThan(0);
  });
});
