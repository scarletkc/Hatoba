import type { HatobaApi } from "../api";
import type {
  AppError,
  EventMap,
  McpImportPreview,
  McpServerInput,
  McpServerState,
  McpServerStatus,
  McpServerView,
  McpToolAnnotations,
  McpToolInfo,
  McpToolView,
  McpTransportView,
  SkillDetail,
  SkillFileView,
  SkillImportPreview,
  SkillIssue,
  SkillView,
} from "../types";

type AiExtensionsApi = Pick<
  HatobaApi,
  | "skills_list"
  | "skill_get"
  | "skill_save"
  | "skill_delete"
  | "skill_set_enabled"
  | "skill_import_preview"
  | "skill_import"
  | "skill_export"
  | "mcp_servers_list"
  | "mcp_server_save"
  | "mcp_server_delete"
  | "mcp_server_set_enabled"
  | "mcp_server_status"
  | "mcp_server_start"
  | "mcp_server_stop"
  | "mcp_set_always_allow"
  | "mcp_tool_info"
  | "mcp_import_preview"
  | "mcp_import"
  | "mcp_export"
>;

type Emit = <K extends keyof EventMap>(event: K, payload: EventMap[K]) => void;

// ───────────────────────── Skills ─────────────────────────

const NAME_RULE = /^[a-z0-9-]{1,64}$/;
const FILE_LIMIT = 32 * 1024;
const bytes = (text: string) => new TextEncoder().encode(text).length;

const NGINX_BODY = `# nginx operations

Use this skill when the user asks about nginx on a host: a site that returns 502 or 504, a config change that
has to go live, or a log that needs reading.

## Steps

1. Run \`nginx -t\` first. Never reload a configuration that fails the syntax check.
2. Read \`references/config-checks.md\` for what to look at in a failing config.
3. Reload with \`systemctl reload nginx\`. A reload keeps open connections; a restart drops them.
4. If the problem continues, read the error log (see \`references/log-locations.md\`) before changing anything.

Ask the user before editing files under /etc/nginx.
`;

const demoSkills = (): { detail: SkillDetail; files: SkillFileView[] }[] => {
  const make = (
    id: string,
    name: string,
    description: string,
    body: string,
    files: SkillFileView[],
    enabled: boolean,
    keys: string[],
    ageMs: number,
  ) => ({
    files,
    detail: {
      skill: { id, name, description, enabled, files: files.map((f) => f.path), updated_at: Date.now() - ageMs },
      body,
      files,
      frontmatter_keys: keys,
    } satisfies SkillDetail,
  });
  return [
    make(
      "sk-nginx",
      "nginx-ops",
      "Diagnose and reload nginx on Linux hosts: check the config syntax, read the error logs, and apply changes without dropping connections.",
      NGINX_BODY,
      [
        {
          path: "references/config-checks.md",
          content: "# Config checks\n\n- `server_name` matches the host the client asked for.\n- `proxy_pass` ends with a slash only when the location does.\n- `upstream` servers listen on the port the config names.\n",
        },
        { path: "references/log-locations.md", content: "# Log locations\n\n- /var/log/nginx/error.log\n- /var/log/nginx/access.log\n- journalctl -u nginx\n" },
      ],
      true,
      ["allowed-tools", "license"],
      3 * 86_400_000,
    ),
    make(
      "sk-backup",
      "postgres-backup",
      "Back up and restore PostgreSQL databases with pg_dump and pg_restore, and check that a backup file is usable.",
      "# PostgreSQL backups\n\nTake a custom-format dump with `pg_dump -Fc`. Restore into an empty database with `pg_restore --clean --if-exists`.\n\nAlways run `pg_restore --list` on a dump before relying on it.\n",
      [{ path: "references/restore.md", content: "# Restore\n\n1. Create the target database.\n2. Run pg_restore with --no-owner when the roles differ.\n" }],
      true,
      [],
      26 * 3_600_000,
    ),
    make("sk-notes", "release-notes", "Write release notes from a list of merged pull requests, grouped by change type.", "# Release notes\n\nGroup changes under Added, Changed, and Fixed. Keep each entry to one line.\n", [], false, [], 9 * 86_400_000),
  ];
};

/** What a stand-in path imports. A browser has no folder picker, so the UI hands these paths to the mock. */
const importPackage = (path: string) => {
  const zip = path.toLowerCase().endsWith(".zip");
  return zip
    ? {
        name: "nginx-ops",
        description: "Diagnose and reload nginx on Linux hosts, with the checks a new config needs before it goes live.",
        body: NGINX_BODY,
        files: [
          { path: "references/config-checks.md", content: "# Config checks\n\n- Run `nginx -T` to print the config nginx actually loaded.\n- Check certificate paths and permissions.\n" },
          { path: "references/log-locations.md", content: "# Log locations\n\n- /var/log/nginx/error.log\n- /var/log/nginx/access.log\n" },
          { path: "assets/example.conf", content: "server {\n  listen 80;\n  server_name example.com;\n  location / { proxy_pass http://127.0.0.1:3000/; }\n}\n" },
        ],
        frontmatter_keys: ["allowed-tools", "license", "metadata"],
        skipped: [],
      }
    : {
        name: "docker-compose",
        description: "Operate Docker Compose projects: bring services up and down, read their logs, and find out why a container keeps restarting.",
        body: "# Docker Compose\n\nRun `docker compose ps` first to see what is running.\n\nTo find a restart loop, read `docker compose logs --tail 100 <service>` and check the exit code with `docker compose ps -a`.\n\nNever run `docker compose down -v` without asking: it deletes the volumes.\n",
        files: [
          { path: "references/restart-loops.md", content: "# Restart loops\n\n- Exit code 137: the container was killed, often out of memory.\n- Exit code 1 right after start: read the first lines of its log.\n" },
          { path: "scripts/status.sh", content: "#!/bin/sh\n# Hatoba never runs this file. It is text the assistant can read.\ndocker compose ps --format json\n" },
        ],
        frontmatter_keys: ["allowed-tools"],
        skipped: ["images/diagram.png", "assets/screenshot.jpg"],
      };
};

// ───────────────────────── MCP servers ─────────────────────────

interface ToolDef {
  tool: string;
  description: string;
  annotations?: Partial<McpToolAnnotations>;
}

const GITHUB_TOOLS: ToolDef[] = [
  {
    tool: "list_issues",
    description: "List the issues of a GitHub repository. Returns the number, title, state, labels and author of each issue. `state` is open or closed.",
    annotations: { title: "List issues", read_only_hint: true, open_world_hint: true },
  },
  {
    tool: "create_issue",
    description: "Create an issue in a GitHub repository. Use it only when the user asked for an issue to be filed, and show the user the title and body first.",
    annotations: { title: "Create issue", read_only_hint: false, destructive_hint: false, idempotent_hint: false, open_world_hint: true },
  },
  {
    tool: "search_code",
    description: "Search the code of public and private repositories the token can read. Supports qualifiers such as `repo:`, `path:` and `language:`.",
    annotations: { read_only_hint: true, idempotent_hint: true, open_world_hint: true },
  },
];

const FILESYSTEM_TOOLS: ToolDef[] = [
  { tool: "read_file", description: "Read the complete contents of a file as text.", annotations: { read_only_hint: true, idempotent_hint: true } },
  { tool: "list_directory", description: "List the files and folders in a directory.", annotations: { read_only_hint: true, idempotent_hint: true } },
  {
    tool: "write_file",
    description:
      "Create a file or completely overwrite an existing one.\n\nIMPORTANT: this replaces the file without keeping a copy. Read the file first when it may already exist, and tell the user which path you will write before you call this tool.\n\nOnly paths inside the allowed directories can be written.",
    annotations: { read_only_hint: false, destructive_hint: true, idempotent_hint: true },
  },
  { tool: "delete_file", description: "Delete a file. This cannot be undone.", annotations: { read_only_hint: false, destructive_hint: true } },
];

const POSTGRES_TOOLS: ToolDef[] = [
  { tool: "list_tables", description: "List the tables of the connected database.", annotations: { read_only_hint: true } },
  { tool: "query", description: "Run a read-only SQL query and return up to 100 rows.", annotations: { read_only_hint: true, open_world_hint: false } },
];

const GENERIC_TOOLS: ToolDef[] = [
  { tool: "echo", description: "Return the text it was given." },
  { tool: "status", description: "Report whether the server is healthy.", annotations: { read_only_hint: true } },
];

const toolsFor = (name: string): ToolDef[] => {
  const n = name.toLowerCase();
  if (n.includes("github")) return GITHUB_TOOLS;
  if (n.includes("filesystem") || n === "files") return FILESYSTEM_TOOLS;
  if (n.includes("postgres")) return POSTGRES_TOOLS;
  return GENERIC_TOOLS;
};

/** `mcp__<server>__<tool>`, cleaned to letters, digits, `_` and `-`, and cut to 64 characters (AI-30). */
const toolName = (server: string, tool: string): string => {
  const clean = (s: string) => s.replace(/[^A-Za-z0-9_-]/g, "_");
  return `mcp__${clean(server)}__${clean(tool)}`.slice(0, 64);
};

const commandLine = (t: { kind: "stdio"; command: string; args: string[] } | { kind: "http"; url: string }): string =>
  t.kind === "stdio" ? [t.command, ...t.args].join(" ") : t.url;

interface Entry {
  view: McpServerView;
  state: McpServerState;
  error: string | null;
  stderr: string[];
  tools: ToolDef[];
  /** Bumped when the server stops, so a start that is still waiting knows it was overtaken. */
  run: number;
}

const stderrFor = (view: McpServerView): string[] =>
  view.transport.kind === "stdio"
    ? [
        `Error: Cannot find module '${view.transport.args[0] ?? view.transport.command}'`,
        "    at Module._resolveFilename (node:internal/modules/cjs/loader:1225:15)",
        "    at node:internal/main/run_main_module:28:49 {",
        "  code: 'MODULE_NOT_FOUND'",
        "}",
        "Node.js v22.11.0",
      ]
    : [];

/**
 * Mock of skills and MCP servers (§13.8, §13.9) for `pnpm dev`. URL parameters (comma-separated, so
 * `?skills=demo,badimport&mcp=demo` works) select demo states:
 *   ?skills=demo        three skills: nginx-ops and postgres-backup (with files, and frontmatter fields kept for
 *                       export) and release-notes (disabled)
 *   ?skills=badimport   importing a folder returns a preview with issues (the import button stays disabled)
 *   ?mcp=demo           five servers: github (http, running, Always allow on list_issues), filesystem (stdio, running,
 *                       Always ask), postgres (stdio, starting, running after 2.5 s), a failed one (its command
 *                       contains `fail`, with stderr lines) and notes (stdio, off on this device)
 * Without parameters both lists are empty. The browser has no folder or file picker, so the UI passes stand-in
 * paths: a folder is `~/skills/docker-compose` (a new skill, with skipped files), a `.zip` is
 * `~/Downloads/nginx-ops.zip` (clashes with nginx-ops while that skill exists). A path containing `broken` has
 * issues, and one containing `empty` has no SKILL.md. Behaviors worth trying in the dialogs:
 *   Start/Restart       starting for a second, then running with tools; a command line containing `fail` fails
 *   a URL with `fail`   an http server that cannot be reached
 *   Import JSON         a real parser: `mcpServers` or VS Code `servers`, `sse` entries are skipped
 * `mcp_tool_info` answers for the tools of running servers (`mcp__github__list_issues`,
 * `mcp__github__create_issue`, `mcp__filesystem__write_file`, …) and `ai://mcp-status` is emitted on every change.
 * Nothing here is secure; it never runs inside the Tauri app.
 */
export function createAiExtensionsMock(emit: Emit): AiExtensionsApi {
  const params = new URLSearchParams(location.search);
  const flags = (key: string) => new Set((params.get(key) ?? "").split(",").map((x) => x.trim()));
  const skillFlags = flags("skills");
  const mcpFlags = flags("mcp");

  let seq = 0;
  const id = (p: string) => `${p}-${(++seq).toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
  function fail(code: AppError["code"], detail: string, field?: string): never {
    const err: AppError = { code, detail, ...(field ? { field } : {}) };
    throw err;
  }

  // ── skills ──
  const skills = new Map<string, SkillDetail>(skillFlags.has("demo") ? demoSkills().map((s) => [s.detail.skill.id, s.detail]) : []);
  const skillView = (d: SkillDetail): SkillView => ({ ...d.skill, files: d.files.map((f) => f.path) });

  const nameTaken = (name: string, except: string | null) => [...skills.values()].some((d) => d.skill.name === name && d.skill.id !== except);

  /** The rules of AI-27, as the backend checks a saved skill. */
  const skillIssues = (name: string, description: string, body: string, files: SkillFileView[]): SkillIssue[] => {
    const issues: SkillIssue[] = [];
    if (!NAME_RULE.test(name)) issues.push({ kind: "invalid_name", name });
    const chars = Array.from(description.trim()).length;
    if (chars === 0) issues.push({ kind: "missing_description" });
    else if (chars > 1024) issues.push({ kind: "description_too_long", chars });
    if (bytes(body) > FILE_LIMIT) issues.push({ kind: "file_too_large", path: "SKILL.md", size: bytes(body) });
    const seen = new Set<string>();
    for (const f of files) {
      if (!f.path || f.path.startsWith("/") || f.path.split(/[\\/]/).includes("..") || f.path === "SKILL.md" || seen.has(f.path)) {
        issues.push({ kind: "unsafe_path", path: f.path });
      }
      seen.add(f.path);
      if (bytes(f.content) > FILE_LIMIT) issues.push({ kind: "file_too_large", path: f.path, size: bytes(f.content) });
    }
    return issues;
  };

  const issueField = (issue: SkillIssue): string => {
    switch (issue.kind) {
      case "invalid_name":
        return "name";
      case "missing_description":
      case "description_too_long":
        return "description";
      case "file_too_large":
        return issue.path === "SKILL.md" ? "body" : "files";
      default:
        return "files";
    }
  };

  const previewFor = (path: string): SkillImportPreview => {
    const lower = path.toLowerCase();
    if (lower.includes("empty")) {
      return { name: null, description: null, body: null, files: [], frontmatter_keys: [], skipped: [], issues: [{ kind: "missing_skill_md" }], existing_id: null };
    }
    if (lower.includes("broken") || (skillFlags.has("badimport") && !lower.endsWith(".zip"))) {
      return {
        name: "Nginx Ops",
        description: "Operate nginx. ".repeat(80).trim(),
        body: "# nginx\n\nReload with `nginx -s reload`.\n",
        files: [
          { path: "references/checks.md", content: "# Checks\n\nRun `nginx -t`.\n" },
          { path: "references/dump.txt", content: "x".repeat(48 * 1024) },
        ],
        frontmatter_keys: ["allowed-tools"],
        skipped: ["assets/logo.png"],
        issues: [
          { kind: "invalid_name", name: "Nginx Ops" },
          { kind: "description_too_long", chars: 1199 },
          { kind: "file_too_large", path: "references/dump.txt", size: 48 * 1024 },
          { kind: "unsafe_path", path: "../outside.sh" },
        ],
        existing_id: null,
      };
    }
    const pkg = importPackage(path);
    const existing = [...skills.values()].find((d) => d.skill.name === pkg.name);
    return { ...pkg, issues: [], existing_id: existing?.skill.id ?? null };
  };

  // ── MCP servers ──
  const entries: Entry[] = [];
  const entry = (sid: string): Entry => entries.find((e) => e.view.id === sid) ?? fail("not_found", "no such server");

  const toolView = (e: Entry, def: ToolDef): McpToolView => ({
    name: toolName(e.view.name, def.tool),
    tool: def.tool,
    description: def.description,
    annotations: {
      title: def.annotations?.title ?? null,
      read_only_hint: def.annotations?.read_only_hint ?? null,
      destructive_hint: def.annotations?.destructive_hint ?? null,
      idempotent_hint: def.annotations?.idempotent_hint ?? null,
      open_world_hint: def.annotations?.open_world_hint ?? null,
    },
    always_allow: e.view.always_allow || e.view.always_allow_tools.includes(def.tool),
  });
  const statusOf = (e: Entry): McpServerStatus => ({
    server_id: e.view.id,
    state: e.state,
    error: e.error,
    stderr: [...e.stderr],
    tools: e.tools.map((d) => toolView(e, d)),
  });
  const publish = (e: Entry) => emit("ai://mcp-status", statusOf(e));
  const settle = (e: Entry) => {
    if (commandLine(e.view.transport.kind === "stdio" ? { kind: "stdio", command: e.view.transport.command, args: e.view.transport.args } : { kind: "http", url: e.view.transport.url }).includes("fail")) {
      e.state = "failed";
      e.error =
        e.view.transport.kind === "stdio"
          ? "the server could not start: it exited with code 1 before it finished initializing"
          : "could not connect to the server: connection refused";
      e.stderr = stderrFor(e.view);
      e.tools = [];
    } else {
      e.state = "running";
      e.error = null;
      e.stderr = e.view.transport.kind === "stdio" ? ["MCP server running on stdio"] : [];
      e.tools = toolsFor(e.view.name);
    }
  };
  const stop = (e: Entry) => {
    e.run++;
    e.state = "stopped";
    e.error = null;
    e.stderr = [];
    e.tools = [];
  };

  const addServer = (view: McpServerView, state: McpServerState = "stopped"): Entry => {
    const e: Entry = { view, state: "stopped", error: null, stderr: [], tools: [], run: 0 };
    entries.push(e);
    if (state === "running" || state === "failed") settle(e);
    return e;
  };

  if (mcpFlags.has("demo")) {
    const day = 86_400_000;
    const base = { always_ask: false, enabled: true, always_allow: false, always_allow_tools: [] as string[] };
    addServer(
      {
        ...base,
        id: "mcp-github",
        name: "github",
        transport: { kind: "http", url: "https://api.githubcopilot.com/mcp/", header_keys: ["Authorization"] },
        always_allow_tools: ["list_issues"],
        updated_at: Date.now() - 5 * day,
      },
      "running",
    );
    addServer(
      {
        ...base,
        id: "mcp-filesystem",
        name: "filesystem",
        transport: { kind: "stdio", command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "C:\\Users\\kc\\Documents"], env_keys: [] },
        always_ask: true,
        updated_at: Date.now() - 2 * day,
      },
      "running",
    );
    const postgres = addServer({
      ...base,
      id: "mcp-postgres",
      name: "postgres",
      transport: { kind: "stdio", command: "uvx", args: ["postgres-mcp", "--access-mode=restricted"], env_keys: ["DATABASE_URI"] },
      updated_at: Date.now() - day,
    });
    postgres.state = "starting";
    setTimeout(() => {
      if (postgres.state !== "starting") return;
      settle(postgres);
      publish(postgres);
    }, 2500);
    addServer(
      {
        ...base,
        id: "mcp-broken",
        name: "report-builder",
        transport: { kind: "stdio", command: "node", args: ["C:\\tools\\fail-server.js", "--stdio"], env_keys: ["REPORT_DIR", "LOG_LEVEL"] },
        updated_at: Date.now() - 4 * day,
      },
      "failed",
    );
    addServer({
      ...base,
      id: "mcp-notes",
      name: "notes",
      transport: { kind: "stdio", command: "notes-mcp", args: [], env_keys: [] },
      enabled: false,
      updated_at: Date.now() - 12 * day,
    });
  }

  const cloneView = (v: McpServerView): McpServerView => structuredClone(v);

  /** What a saved server's transport looks like to the page: names only, never values. */
  const transportView = (input: McpServerInput["transport"], old: McpServerView | undefined): McpTransportView => {
    const secretKeys = (rows: { key: string; value: string | null }[], saved: string[]) => {
      for (const r of rows) {
        if (r.value === null && !saved.includes(r.key)) fail("invalid_input", `there is no saved value for ${r.key}`, input.kind === "stdio" ? "env" : "headers");
      }
      return rows.map((r) => r.key);
    };
    if (input.kind === "stdio") {
      if (!input.command.trim()) fail("invalid_input", "the command is empty", "command");
      const saved = old?.transport.kind === "stdio" ? old.transport.env_keys : [];
      return { kind: "stdio", command: input.command, args: input.args, env_keys: secretKeys(input.env, saved) };
    }
    const url = (() => {
      try {
        return new URL(input.url);
      } catch {
        return fail("invalid_input", "the URL is not valid", "url");
      }
    })();
    const local = /^(localhost|127\.|10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|\[::1\])/.test(url.hostname);
    if (url.protocol !== "https:" && !(url.protocol === "http:" && local)) fail("invalid_input", "the URL must use https unless it is a local address", "url");
    const saved = old?.transport.kind === "http" ? old.transport.header_keys : [];
    return { kind: "http", url: input.url, header_keys: secretKeys(input.headers, saved) };
  };

  // ── MCP import and export (AI-33) ──
  interface Parsed {
    name: string;
    transport: McpTransportView;
  }
  const parseImport = (json: string): { servers: Parsed[]; skipped: { name: string; reason: string }[] } => {
    let top: unknown;
    try {
      top = JSON.parse(json.replace(/^\uFEFF/, ""));
    } catch (e) {
      const at = /position (\d+)/.exec(e instanceof Error ? e.message : "");
      const before = at ? json.slice(0, Number(at[1])).split("\n") : [];
      return fail("invalid_input", at ? `the text is not valid JSON (line ${before.length}, column ${before[before.length - 1].length + 1})` : "the text is not valid JSON");
    }
    const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
    if (!isObject(top)) return fail("invalid_input", "the text must be a JSON object");
    const looksLikeServer = (v: unknown) => isObject(v) && ("command" in v || "url" in v || "type" in v);
    let list: unknown = top.mcpServers ?? top.servers;
    if (list === undefined && Object.keys(top).length > 0 && Object.values(top).every(looksLikeServer)) list = top;
    if (!isObject(list)) {
      return fail("invalid_input", 'no MCP servers found: expected an "mcpServers" object (Claude Desktop, Claude Code, Cursor) or a "servers" object (VS Code)');
    }
    const servers: Parsed[] = [];
    const skipped: { name: string; reason: string }[] = [];
    for (const [rawName, v] of Object.entries(list)) {
      const name = rawName.trim();
      if (!name) skipped.push({ name: rawName, reason: "the server has no name" });
      else if (!isObject(v)) skipped.push({ name, reason: "the entry is not an object" });
      else if (v.type === "sse") skipped.push({ name, reason: 'the "sse" transport is not supported; only stdio and Streamable HTTP servers can be imported' });
      else if (typeof v.command === "string" && v.command.trim()) {
        servers.push({
          name,
          transport: {
            kind: "stdio",
            command: v.command.trim(),
            args: Array.isArray(v.args) ? v.args.map(String) : [],
            env_keys: isObject(v.env) ? Object.keys(v.env) : [],
          },
        });
      } else if (typeof (v.url ?? v.httpUrl ?? v.serverUrl) === "string") {
        servers.push({
          name,
          transport: { kind: "http", url: String(v.url ?? v.httpUrl ?? v.serverUrl), header_keys: isObject(v.headers) ? Object.keys(v.headers) : [] },
        });
      } else skipped.push({ name, reason: "the server has neither a command nor a URL" });
    }
    return { servers, skipped };
  };

  const placeholder = (key: string) => `<${key.toUpperCase().replace(/[^A-Z0-9]+/g, "_")}>`;

  return {
    skills_list: async () => [...skills.values()].map(skillView),
    skill_get: async (sid) => structuredClone(skills.get(sid) ?? fail("not_found", "no such skill")),
    skill_save: async (input) => {
      await wait(200);
      const issues = skillIssues(input.name, input.description, input.body, input.files);
      if (issues.length) fail("invalid_input", `the skill is not valid: ${issues[0].kind}`, issueField(issues[0]));
      if (nameTaken(input.name, input.id)) fail("invalid_input", "another skill has this name", "name");
      const old = input.id ? skills.get(input.id) : undefined;
      if (input.id && !old) fail("not_found", "no such skill");
      const skillId = old?.skill.id ?? id("sk");
      const detail: SkillDetail = {
        skill: { id: skillId, name: input.name, description: input.description, enabled: input.enabled, files: input.files.map((f) => f.path), updated_at: Date.now() },
        body: input.body,
        files: structuredClone(input.files),
        frontmatter_keys: old?.frontmatter_keys ?? [],
      };
      skills.set(skillId, detail);
      return skillView(detail);
    },
    skill_delete: async (sid) => {
      skills.delete(sid);
    },
    skill_set_enabled: async (sid, enabled) => {
      const d = skills.get(sid) ?? fail("not_found", "no such skill");
      d.skill.enabled = enabled;
    },
    skill_import_preview: async (path) => {
      await wait(350);
      return structuredClone(previewFor(path));
    },
    skill_import: async (path, replaceId, rename) => {
      await wait(300);
      const p = previewFor(path);
      if (p.issues.length || p.name === null || p.description === null || p.body === null) fail("invalid_input", "the import has issues", "path");
      const name = rename ?? p.name;
      if (!NAME_RULE.test(name)) fail("invalid_input", "the new name is not valid", "rename");
      if (replaceId && !skills.has(replaceId)) fail("not_found", "no such skill");
      if (nameTaken(name, replaceId)) fail("invalid_input", "another skill has this name", "name");
      const skillId = replaceId ?? id("sk");
      const detail: SkillDetail = {
        skill: { id: skillId, name, description: p.description, enabled: skills.get(skillId)?.skill.enabled ?? true, files: p.files.map((f) => f.path), updated_at: Date.now() },
        body: p.body,
        files: structuredClone(p.files),
        frontmatter_keys: p.frontmatter_keys,
      };
      skills.set(skillId, detail);
      return skillView(detail);
    },
    skill_export: async (sid) => {
      await wait(300);
      if (!skills.has(sid)) fail("not_found", "no such skill");
    },

    mcp_servers_list: async () => entries.map((e) => cloneView(e.view)),
    mcp_server_save: async (input) => {
      await wait(200);
      const old = input.id ? entry(input.id) : undefined;
      if (!input.name.trim()) fail("invalid_input", "the name is empty", "name");
      const transport = transportView(input.transport, old?.view);
      if (old) {
        const rows = input.transport.kind === "stdio" ? input.transport.env : input.transport.headers;
        const changed = JSON.stringify(old.view.transport) !== JSON.stringify(transport) || rows.some((r) => r.value !== null);
        old.view = { ...old.view, name: input.name.trim(), transport, always_ask: input.always_ask, updated_at: Date.now() };
        // A running server whose configuration changed is stopped; it starts again when a conversation needs it.
        if (changed && old.state !== "stopped") {
          stop(old);
          publish(old);
        }
        return cloneView(old.view);
      }
      const view: McpServerView = {
        id: id("mcp"),
        name: input.name.trim(),
        transport,
        always_ask: input.always_ask,
        enabled: true,
        always_allow: false,
        always_allow_tools: [],
        updated_at: Date.now(),
      };
      addServer(view);
      return cloneView(view);
    },
    mcp_server_delete: async (sid) => {
      const i = entries.findIndex((e) => e.view.id === sid);
      if (i >= 0) {
        stop(entries[i]);
        entries.splice(i, 1);
      }
    },
    mcp_server_set_enabled: async (sid, enabled) => {
      const e = entry(sid);
      e.view = { ...e.view, enabled };
      if (!enabled && e.state !== "stopped") {
        stop(e);
        publish(e);
      }
    },
    mcp_server_status: async (sid) => statusOf(entry(sid)),
    mcp_server_start: async (sid) => {
      const e = entry(sid);
      if (!e.view.enabled) fail("invalid_input", "the server is off on this device", "enabled");
      const run = ++e.run;
      e.state = "starting";
      e.error = null;
      e.stderr = [];
      e.tools = [];
      publish(e);
      await wait(1000);
      if (e.run !== run) return statusOf(e);
      settle(e);
      publish(e);
      return statusOf(e);
    },
    mcp_server_stop: async (sid) => {
      const e = entry(sid);
      stop(e);
      publish(e);
    },
    mcp_set_always_allow: async (sid, tool, allow) => {
      const e = entry(sid);
      if (tool === null) e.view = { ...e.view, always_allow: allow };
      else {
        const rest = e.view.always_allow_tools.filter((x) => x !== tool);
        e.view = { ...e.view, always_allow_tools: allow ? [...rest, tool] : rest };
      }
      if (e.state === "running") publish(e);
    },
    mcp_tool_info: async (name): Promise<McpToolInfo | null> => {
      for (const e of entries) {
        if (e.state !== "running") continue;
        const def = e.tools.find((d) => toolName(e.view.name, d.tool) === name);
        if (def) return { server_id: e.view.id, server_name: e.view.name, always_ask: e.view.always_ask, tool: toolView(e, def) };
      }
      return null;
    },
    mcp_import_preview: async (json): Promise<McpImportPreview> => {
      await wait(150);
      const { servers, skipped } = parseImport(json);
      return {
        servers: servers.map((s) => ({ ...s, exists: entries.some((e) => e.view.name.toLowerCase() === s.name.toLowerCase()) })),
        skipped,
      };
    },
    mcp_import: async (json) => {
      await wait(250);
      const { servers } = parseImport(json);
      return servers.map((s) => {
        const view: McpServerView = {
          id: id("mcp"),
          name: s.name,
          transport: s.transport,
          always_ask: false,
          enabled: s.transport.kind === "http",
          always_allow: false,
          always_allow_tools: [],
          updated_at: Date.now(),
        };
        addServer(view);
        return cloneView(view);
      });
    },
    mcp_export: async () => {
      const mcpServers: Record<string, unknown> = {};
      for (const { view } of entries) {
        const t = view.transport;
        mcpServers[view.name] =
          t.kind === "stdio"
            ? { command: t.command, args: t.args, ...(t.env_keys.length ? { env: Object.fromEntries(t.env_keys.map((k) => [k, placeholder(k)])) } : {}) }
            : { type: "http", url: t.url, ...(t.header_keys.length ? { headers: Object.fromEntries(t.header_keys.map((k) => [k, placeholder(k)])) } : {}) };
      }
      return JSON.stringify({ mcpServers }, null, 2);
    },
  };
}
