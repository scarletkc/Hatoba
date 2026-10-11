import { API_TOKENS_URL } from "@/features/sync/external";
import { freeServerName, initialMcpForm, newSecretRow, type McpForm, type McpTemplateHeader, type SecretRow } from "./mcpLogic";

/*
 * Templates for Settings → AI → MCP servers (spec §13.9, AI-44): no React, no i18n. They ship with the app and only
 * fill in the add dialog, so a server added from one is an ordinary server and nothing records the template. Hatoba
 * has no OAuth sign-in yet, so a template is a remote server that takes a static token in a header.
 */

export type McpTemplateId = "github" | "linear" | "context7" | "cloudflare";

export interface McpTemplate {
  id: McpTemplateId;
  /** The service's name, shown in the menu. Not translated. */
  label: string;
  icon: string;
  /** The server name the form starts with. */
  name: string;
  url: string;
  headers: McpTemplateHeader[];
  /** On for a server whose tools can change data (AI-31). */
  alwaysAsk: boolean;
  /** Where the user creates the token the template asks for. */
  tokenUrl: string;
}

export const MCP_TEMPLATES: readonly McpTemplate[] = [
  {
    // github/github-mcp-server, docs/remote-server.md: `/readonly` keeps the default toolsets to their read tools. The
    // token link fills in a fine-grained token with read access to what those toolsets reach.
    id: "github",
    label: "GitHub",
    icon: "github-logo",
    name: "github",
    url: "https://api.githubcopilot.com/mcp/readonly",
    headers: [{ key: "Authorization", placeholder: "Bearer github_pat_…", bearer: true }],
    alwaysAsk: false,
    tokenUrl: "https://github.com/settings/personal-access-tokens/new?name=Hatoba+MCP&contents=read&issues=read&pull_requests=read",
  },
  {
    // linear.app/docs/mcp: an API key in `Authorization: Bearer` instead of the OAuth flow.
    id: "linear",
    label: "Linear",
    icon: "kanban",
    name: "linear",
    url: "https://mcp.linear.app/mcp",
    headers: [{ key: "Authorization", placeholder: "Bearer lin_api_…", bearer: true }],
    alwaysAsk: true,
    tokenUrl: "https://linear.app/settings/account/security",
  },
  {
    // upstash/context7: works without a key; a key in `Authorization: Bearer` raises the rate limit.
    id: "context7",
    label: "Context7",
    icon: "book-open-text",
    name: "context7",
    url: "https://mcp.context7.com/mcp",
    headers: [{ key: "Authorization", placeholder: "Bearer …", bearer: true, optional: true }],
    alwaysAsk: false,
    tokenUrl: "https://context7.com/dashboard",
  },
  {
    // developers.cloudflare.com/agents/model-context-protocol/cloudflare/servers-for-cloudflare: a user or account
    // API token as a bearer token. Its `execute` tool calls any endpoint the token allows.
    id: "cloudflare",
    label: "Cloudflare",
    icon: "cloud",
    name: "cloudflare",
    url: "https://mcp.cloudflare.com/mcp",
    headers: [{ key: "Authorization", placeholder: "Bearer …", bearer: true }],
    alwaysAsk: true,
    tokenUrl: API_TOKENS_URL,
  },
];

/**
 * The add form filled from a template: an HTTP server with the template's URL, its header names with empty values, and
 * a name no other server uses. Each row keeps its template header.
 */
export function templateMcpForm(template: McpTemplate, otherNames: readonly string[]): McpForm {
  return {
    ...initialMcpForm(null),
    name: freeServerName(template.name, otherNames),
    kind: "http",
    url: template.url,
    headers: template.headers.map((h): SecretRow => ({ ...newSecretRow(), key: h.key, template: h })),
    alwaysAsk: template.alwaysAsk,
  };
}
