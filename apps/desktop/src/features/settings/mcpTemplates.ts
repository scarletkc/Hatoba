import { API_TOKENS_URL } from "@/features/sync/external";
import { freeServerName, initialMcpForm, newSecretRow, type McpForm, type SecretRow } from "./mcpLogic";

/*
 * Templates for Settings → AI → MCP servers (spec §13.9, AI-44): no React, no i18n. They ship with the app and only
 * fill in the add dialog, so a server added from one is an ordinary server and nothing records the template. Hatoba
 * has no OAuth sign-in yet, so a template is a remote server that takes a static token in a header.
 */

export type McpTemplateId = "github" | "linear" | "context7" | "cloudflare";

/** A header the template fills in: its name, and what its value looks like. */
export interface McpTemplateHeader {
  key: string;
  placeholder?: string;
}

export interface McpTemplate {
  id: McpTemplateId;
  /** The service's name, shown in the menu. Not translated. */
  label: string;
  icon: string;
  /** The server name the form starts with. */
  name: string;
  url: string;
  headers: McpTemplateHeader[];
  /** Where the user creates the token the template asks for. */
  tokenUrl: string;
}

export const MCP_TEMPLATES: readonly McpTemplate[] = [
  {
    // github/github-mcp-server: the base URL with the default toolsets, as GitHub's own one-click install uses. The
    // token link fills in a fine-grained token with read and write access to what those toolsets reach.
    id: "github",
    label: "GitHub",
    icon: "github-logo",
    name: "github",
    url: "https://api.githubcopilot.com/mcp/",
    headers: [{ key: "Authorization", placeholder: "github_pat_…" }],
    tokenUrl: "https://github.com/settings/personal-access-tokens/new?name=Hatoba+MCP&contents=write&issues=write&pull_requests=write",
  },
  {
    // linear.app/docs/mcp: an API key in `Authorization: Bearer` instead of the OAuth flow.
    id: "linear",
    label: "Linear",
    icon: "kanban",
    name: "linear",
    url: "https://mcp.linear.app/mcp",
    headers: [{ key: "Authorization", placeholder: "lin_api_…" }],
    tokenUrl: "https://linear.app/settings/account/security",
  },
  {
    // upstash/context7: an API key in `Authorization: Bearer`.
    id: "context7",
    label: "Context7",
    icon: "book-open-text",
    name: "context7",
    url: "https://mcp.context7.com/mcp",
    headers: [{ key: "Authorization" }],
    tokenUrl: "https://context7.com/dashboard",
  },
  {
    // developers.cloudflare.com/agents/model-context-protocol/cloudflare/servers-for-cloudflare: a user or account
    // API token as a bearer token, with the permissions the user picks.
    id: "cloudflare",
    label: "Cloudflare",
    icon: "cloud",
    name: "cloudflare",
    url: "https://mcp.cloudflare.com/mcp",
    headers: [{ key: "Authorization" }],
    tokenUrl: API_TOKENS_URL,
  },
];

/**
 * The add form filled from a template: an HTTP server with the template's URL, its header names with empty values and
 * their placeholders, and a name no other server uses.
 */
export function templateMcpForm(template: McpTemplate, otherNames: readonly string[]): McpForm {
  return {
    ...initialMcpForm(null),
    name: freeServerName(template.name, otherNames),
    kind: "http",
    url: template.url,
    headers: template.headers.map((h): SecretRow => ({ ...newSecretRow(), key: h.key, placeholder: h.placeholder })),
  };
}
