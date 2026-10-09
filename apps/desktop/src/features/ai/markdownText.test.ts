import { describe, expect, it } from "vitest";
import { fenced, quoted } from "./markdownText";

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
