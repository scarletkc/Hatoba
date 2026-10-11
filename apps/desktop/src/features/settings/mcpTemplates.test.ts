import { describe, expect, it } from "vitest";
import { hasMcpProblems, mcpFormToInput, validateMcpForm } from "./mcpLogic";
import { MCP_TEMPLATES, templateMcpForm, type McpTemplate } from "./mcpTemplates";

const template = (id: McpTemplate["id"]): McpTemplate => {
  const found = MCP_TEMPLATES.find((x) => x.id === id);
  if (!found) throw new Error(`no template ${id}`);
  return found;
};

/** Types a value into the header row named `key`. */
const typed = (form: ReturnType<typeof templateMcpForm>, key: string, value: string) => ({
  ...form,
  headers: form.headers.map((r) => (r.key === key ? { ...r, draft: { mode: "replace" as const, value } } : r)),
});

describe("MCP templates", () => {
  it("are HTTPS servers with distinct ids and names", () => {
    expect(new Set(MCP_TEMPLATES.map((x) => x.id)).size).toBe(MCP_TEMPLATES.length);
    expect(new Set(MCP_TEMPLATES.map((x) => x.name)).size).toBe(MCP_TEMPLATES.length);
    for (const x of MCP_TEMPLATES) {
      expect(new URL(x.url).protocol, x.id).toBe("https:");
      expect(new URL(x.tokenUrl).protocol, x.id).toBe("https:");
    }
  });

  it("fill in a valid form apart from the token", () => {
    for (const x of MCP_TEMPLATES) {
      const problems = validateMcpForm(templateMcpForm(x, []));
      expect(problems.name, x.id).toBeNull();
      expect(problems.url, x.id).toBeNull();
    }
  });

  it("give GitHub its default toolsets and a token that can write", () => {
    const github = template("github");
    expect(github.url).toBe("https://api.githubcopilot.com/mcp/");
    expect(github.headers.map((h) => h.key)).toEqual(["Authorization"]);
    const token = new URL(github.tokenUrl);
    expect(token.pathname).toBe("/settings/personal-access-tokens/new");
    expect(["contents", "issues", "pull_requests"].map((p) => token.searchParams.get(p))).toEqual(["write", "write", "write"]);
  });

  it("show the token without Bearer, which saving adds", () => {
    for (const x of MCP_TEMPLATES) for (const h of x.headers) expect(h.placeholder ?? "", x.id).not.toMatch(/bearer/i);
  });
});

describe("templateMcpForm", () => {
  it("fills in an HTTP server with the header names and empty values", () => {
    const f = templateMcpForm(template("github"), []);
    expect(f).toMatchObject({ name: "github", kind: "http", url: "https://api.githubcopilot.com/mcp/", command: "", args: [], env: [] });
    expect(f.headers).toMatchObject([
      { key: "Authorization", saved: false, draft: { mode: "replace", value: "" }, placeholder: "github_pat_…" },
    ]);
  });

  it("starts with Always ask off, like a server added by hand", () => {
    for (const x of MCP_TEMPLATES) expect(templateMcpForm(x, []).alwaysAsk, x.id).toBe(false);
  });

  it("picks a name no other server has", () => {
    expect(templateMcpForm(template("github"), ["GitHub"]).name).toBe("github-2");
    expect(templateMcpForm(template("github"), ["github", "github-2"]).name).toBe("github-3");
    expect(templateMcpForm(template("linear"), ["github"]).name).toBe("linear");
  });

  it("gives each row its own uid, also across two forms", () => {
    const uids = [...templateMcpForm(template("github"), []).headers, ...templateMcpForm(template("github"), []).headers].map((r) => r.uid);
    expect(new Set(uids).size).toBe(uids.length);
  });

  it("asks for every template's token before saving, Context7's too", () => {
    for (const x of MCP_TEMPLATES) {
      const f = templateMcpForm(x, []);
      expect(validateMcpForm(f).secrets, x.id).toEqual([{ value: "missing" }]);
      expect(hasMcpProblems(validateMcpForm(typed(f, "Authorization", "token_x"))), x.id).toBe(false);
    }
  });

  it("saves through the ordinary input, with nothing that names the template", () => {
    const f = typed(templateMcpForm(template("github"), []), "Authorization", " github_pat_x ");
    expect(mcpFormToInput(null, f)).toEqual({
      id: null,
      name: "github",
      transport: {
        kind: "http",
        url: "https://api.githubcopilot.com/mcp/",
        headers: [{ key: "Authorization", value: "Bearer github_pat_x" }],
      },
      always_ask: false,
    });
  });
});
