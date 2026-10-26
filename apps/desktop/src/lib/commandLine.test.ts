import { describe, expect, it } from "vitest";
import { formatCommandLine, looksLikeCommandLine, quoteArg, splitCommandLine } from "./commandLine";

describe("quoteArg and formatCommandLine", () => {
  it("leaves plain arguments alone", () => {
    expect(quoteArg("-y")).toBe("-y");
    expect(quoteArg("@modelcontextprotocol/server-filesystem")).toBe("@modelcontextprotocol/server-filesystem");
    expect(quoteArg("C:\\Users\\kc\\docs")).toBe("C:\\Users\\kc\\docs");
    expect(quoteArg("--port=8080")).toBe("--port=8080");
  });

  it("quotes empty arguments and ones with spaces or shell characters", () => {
    expect(quoteArg("")).toBe('""');
    expect(quoteArg("My Docs")).toBe('"My Docs"');
    expect(quoteArg("a;rm -rf /")).toBe('"a;rm -rf /"');
    expect(quoteArg("$HOME")).toBe('"$HOME"');
  });

  it("escapes quotes and shows control characters", () => {
    expect(quoteArg('say "hi"')).toBe('"say \\"hi\\""');
    expect(quoteArg("a\nb")).toBe('"a\\nb"');
    expect(quoteArg("a\u001bb")).toBe('"a\\x1bb"');
  });

  it("joins the command and its arguments", () => {
    expect(formatCommandLine("npx", ["-y", "server", "My Docs"])).toBe('npx -y server "My Docs"');
    expect(formatCommandLine("node", [])).toBe("node");
    expect(formatCommandLine("a\nb", [])).toBe("a\\nb");
  });
});

describe("splitCommandLine", () => {
  it("splits on whitespace", () => {
    expect(splitCommandLine("npx -y  @scope/server")).toEqual(["npx", "-y", "@scope/server"]);
    expect(splitCommandLine("  node  ")).toEqual(["node"]);
    expect(splitCommandLine("")).toEqual([]);
  });

  it("groups quoted words", () => {
    expect(splitCommandLine("node \"My Docs/server.js\" 'two words'")).toEqual([
      "node",
      "My Docs/server.js",
      "two words",
    ]);
    expect(splitCommandLine('echo ""')).toEqual(["echo", ""]);
    expect(splitCommandLine('echo "say \\"hi\\""')).toEqual(["echo", 'say "hi"']);
    expect(splitCommandLine('a"b c"d')).toEqual(["ab cd"]);
  });

  it("keeps Windows paths", () => {
    expect(splitCommandLine("node C:\\tools\\server.js")).toEqual(["node", "C:\\tools\\server.js"]);
    expect(splitCommandLine('"\\\\host\\share\\x.exe" --flag')).toEqual(["\\\\host\\share\\x.exe", "--flag"]);
    expect(splitCommandLine("node My\\ Docs")).toEqual(["node", "My Docs"]);
  });

  it("gives up on an unclosed quote", () => {
    expect(splitCommandLine('node "oops')).toBeNull();
  });

  it("reads back what formatCommandLine wrote", () => {
    const args = ["-y", "My Docs", 'say "hi"', "", "plain"];
    expect(splitCommandLine(formatCommandLine("npx", args))).toEqual(["npx", ...args]);
  });
});

describe("looksLikeCommandLine", () => {
  it("spots a whole command line in the command field", () => {
    expect(looksLikeCommandLine("npx -y server")).toBe(true);
    expect(looksLikeCommandLine("uvx mcp-server-git --repository .")).toBe(true);
  });

  it("accepts a program or a path, even with spaces", () => {
    expect(looksLikeCommandLine("npx")).toBe(false);
    expect(looksLikeCommandLine("")).toBe(false);
    expect(looksLikeCommandLine("C:\\Program Files\\nodejs\\node.exe")).toBe(false);
    expect(looksLikeCommandLine("/usr/local/bin/my server")).toBe(false);
    expect(looksLikeCommandLine('"C:\\Program Files\\x.exe" --a')).toBe(false);
  });
});
