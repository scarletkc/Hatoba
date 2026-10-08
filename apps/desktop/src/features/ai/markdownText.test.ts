import { describe, expect, it } from "vitest";
import { cleanTerminalText, fenced, quoted, withSelection } from "./markdownText";

describe("fenced blocks", () => {
  it("uses a fence longer than any backtick run in the text", () => {
    expect(fenced("ls -la")).toBe("```\nls -la\n```");
    expect(fenced("{}", "json")).toBe("```json\n{}\n```");
    expect(fenced("a ``` b")).toBe("````\na ``` b\n````");
    expect(fenced("x ````` y")).toBe("``````\nx ````` y\n``````");
  });

  it("quotes every line", () => {
    expect(quoted("a\n\nb")).toBe("> a\n>\n> b");
  });
});

describe("Ask AI (AI-10)", () => {
  it("cleans terminal text without touching indentation", () => {
    expect(cleanTerminalText("\r\n\n  indented   \r\nnext  \n\n")).toBe("  indented\nnext");
  });

  it("adds the selection as a fenced block after the draft", () => {
    expect(withSelection("", "error: boom  \n")).toBe("```\nerror: boom\n```\n");
    expect(withSelection("Why does this fail?  \n", "error: boom")).toBe("Why does this fail?\n\n```\nerror: boom\n```\n");
  });

  it("keeps the draft when nothing is selected", () => {
    expect(withSelection("draft", " \n \n")).toBe("draft");
  });
});
