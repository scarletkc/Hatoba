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

  it("keep GitHub read-only and to a few toolsets", () => {
    const github = template("github");
    expect(github.url).toBe("https://api.githubcopilot.com/mcp/readonly");
    expect(github.alwaysAsk).toBe(false);
    expect(github.headers.find((h) => h.key === "X-MCP-Toolsets")?.value).toBe("repos,issues,pull_requests");
    const token = new URL(github.tokenUrl);
    expect(token.pathname).toBe("/settings/personal-access-tokens/new");
    expect(["contents", "issues", "pull_requests"].map((p) => token.searchParams.get(p))).toEqual(["read", "read", "read"]);
  });

  it("turn on Always ask for servers whose tools can change data", () => {
    expect(MCP_TEMPLATES.filter((x) => x.alwaysAsk).map((x) => x.id)).toEqual(["linear", "cloudflare"]);
  });
});

describe("templateMcpForm", () => {
  it("fills in an HTTP server with the header names and empty secret values", () => {
    const f = templateMcpForm(template("github"), []);
    expect(f).toMatchObject({ name: "github", kind: "http", url: "https://api.githubcopilot.com/mcp/readonly", command: "", args: [], env: [] });
    expect(f.headers).toMatchObject([
      { key: "Authorization", saved: false, draft: { mode: "replace", value: "" }, template: { placeholder: "Bearer github_pat_…", bearer: true } },
      { key: "X-MCP-Toolsets", saved: false, draft: { mode: "replace", value: "repos,issues,pull_requests" }, template: { value: "repos,issues,pull_requests" } },
    ]);
    expect(f.headers[0].template?.value).toBeUndefined();
  });

  it("starts with Always ask as the template sets it", () => {
    expect(templateMcpForm(template("linear"), []).alwaysAsk).toBe(true);
    expect(templateMcpForm(template("context7"), []).alwaysAsk).toBe(false);
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

  it("asks for a required token before saving", () => {
    const f = templateMcpForm(template("linear"), []);
    expect(validateMcpForm(f).secrets).toEqual([{ value: "missing" }]);
    expect(hasMcpProblems(validateMcpForm(typed(f, "Authorization", "Bearer lin_api_x")))).toBe(false);
  });

  it("saves through the ordinary input, with nothing that names the template", () => {
    const f = typed(templateMcpForm(template("github"), []), "Authorization", " Bearer github_pat_x ");
    expect(mcpFormToInput(null, f)).toEqual({
      id: null,
      name: "github",
      transport: {
        kind: "http",
        url: "https://api.githubcopilot.com/mcp/readonly",
        headers: [
          { key: "Authorization", value: "Bearer github_pat_x" },
          { key: "X-MCP-Toolsets", value: "repos,issues,pull_requests" },
        ],
      },
      always_ask: false,
    });
  });

  it("leaves an optional header out when no value is typed", () => {
    const f = templateMcpForm(template("context7"), []);
    expect(hasMcpProblems(validateMcpForm(f))).toBe(false);
    expect(mcpFormToInput(null, f).transport).toEqual({ kind: "http", url: "https://mcp.context7.com/mcp", headers: [] });
    expect(mcpFormToInput(null, typed(f, "Authorization", "Bearer k")).transport).toMatchObject({
      headers: [{ key: "Authorization", value: "Bearer k" }],
    });
  });
});
