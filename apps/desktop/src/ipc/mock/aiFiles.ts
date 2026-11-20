import { fileRevertNote, parseMessage, type Note } from "@/lib/ai/attachments";
import { localName } from "@/lib/ai/place";
import { formatTarget } from "@/session/quickConnect";
import type { HatobaApi } from "../api";
import type { AiEntryView, AiFileChange, AiFileRevert, AiToolCall, AiToolStatus } from "../types";
import type { MemoryPlace } from "./aiMemory";
import { CUT_SHORT, DISK_COMMAND, MAINTENANCE, type AiMockCtx, type Conv } from "./aiShared";

type AiFilesApi = Pick<HatobaApi, "ai_file_preview" | "ai_file_changes" | "ai_file_change_diff" | "ai_file_revert">;

/** AI-41: what each edit or write that ran changed, by conversation, like Rust's snapshots. */
interface Snapshot {
  change: AiFileChange;
  tool: string;
  before: string | null;
  after: string;
  /** What the diff of the last revert showed the file holding: Restore anyway writes only over that. */
  shown?: string | null;
}

/** Rust keeps 32 MB; the mock keeps just over what "bigfile" writes, so it drops the changes before it. */
const SNAPSHOT_CAP = 15_800;

const FILE_DISCONNECTED =
  "The terminal tab is disconnected, so the tool did not run. Ask the user to reconnect the tab.";

const filePath = (p: string) =>
  p === "~"
    ? "/home/deploy"
    : p.startsWith("~/")
      ? `/home/deploy/${p.slice(2)}`
      : p.startsWith("/")
        ? p
        : `/home/deploy/${p}`;

const crlfOf = (text: string) => {
  const crlf = text.split("\r\n").length - 1;
  return crlf > text.split("\n").length - 1 - crlf;
};

const toCrlf = (text: string) => text.replace(/\r\n/g, "\n").replace(/\n/g, "\r\n");

const fileLines = (text: string) =>
  text === ""
    ? []
    : text
        .replace(/\n$/, "")
        .split("\n")
        .map((l) => l.replace(/\r$/, ""));

/** The file after an edit or a write, or the error the model gets, as `hatoba_ai::files` computes them. */
function fileAfter(
  name: string,
  args: Record<string, unknown>,
  before: string | null,
): { text: string } | { error: string } {
  const path = String(args.path ?? "");
  if (name === "write_file") {
    const content = String(args.content ?? "");
    return { text: before !== null && crlfOf(before) ? toCrlf(content) : content };
  }
  if (before === null) return { error: `${path} does not exist. Use write_file to create it.` };
  const crlf = crlfOf(before);
  const old = crlf ? toCrlf(String(args.old_string ?? "")) : String(args.old_string ?? "");
  const neu = crlf ? toCrlf(String(args.new_string ?? "")) : String(args.new_string ?? "");
  if (!old) return { error: "old_string is empty. To create a file or replace all of its content, use write_file." };
  const count = before.split(old).length - 1;
  if (count === 0)
    return {
      error: `old_string was not found in ${path}, so nothing was changed. It must match the file exactly, including spaces, tabs and line breaks: read the file with read_file and copy the text without the line numbers.`,
    };
  if (count > 1 && args.replace_all !== true)
    return {
      error: `old_string appears ${count} times in ${path}, so nothing was changed. Include more of the surrounding lines to make it unique, or set replace_all to replace every occurrence.`,
    };
  return { text: args.replace_all === true ? before.split(old).join(neu) : before.replace(old, () => neu) };
}

function exec(command: string): { status: number; stdout: string; stderr: string } {
  if (/nonexistent|fail/.test(command))
    return {
      status: 5,
      stdout: "",
      stderr: "Failed to restart nonexistent.service: Unit nonexistent.service not found.",
    };
  if (/\bdf\b/.test(command))
    return {
      status: 0,
      stdout: "Filesystem      Size  Used Avail Use% Mounted on\n/dev/nvme0n1p1   49G   23G   24G  48% /",
      stderr: "",
    };
  if (command === DISK_COMMAND.powershell)
    return {
      status: 0,
      stdout:
        "Name   Used (GB)  Free (GB)\n----   ---------  ---------\nC         211.40     264.13\nD          88.02     843.51",
      stderr: "",
    };
  if (command === DISK_COMMAND.cmd)
    return {
      status: 0,
      stdout: "              14 File(s)        482,113 bytes\n              21 Dir(s)  283,604,348,928 bytes free",
      stderr: "",
    };
  if (/uptime/.test(command))
    return { status: 0, stdout: " 09:41:12 up 41 days,  3:12,  1 user,  load average: 0.21, 0.18, 0.12", stderr: "" };
  if (/sleep/.test(command)) return { status: 0, stdout: "done", stderr: "" };
  // `?ai=showcase`
  if (/nginx\/error\.log/.test(command))
    return {
      status: 0,
      stdout: [41, 43, 46]
        .map(
          (s) =>
            `2026/10/08 09:38:${s} [error] 812#812: *4821${s % 10} connect() failed (111: Connection refused) while connecting to upstream, client: 10.0.0.4, server: api.example.net, request: "GET /v1/hosts HTTP/1.1", upstream: "http://127.0.0.1:8081/v1/hosts"`,
        )
        .join("\n"),
      stderr: "",
    };
  if (/\bss -ltnp\b/.test(command))
    return {
      status: 0,
      stdout: `LISTEN 0      4096      127.0.0.1:8080      0.0.0.0:*    users:(("api",pid=48211,fd=9))`,
      stderr: "",
    };
  if (/nginx -t/.test(command))
    return {
      status: 0,
      stdout: "",
      stderr:
        "nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\nnginx: configuration file /etc/nginx/nginx.conf test is successful",
    };
  return { status: 0, stdout: `(demo output of \`${command}\`)`, stderr: "" };
}

export function createAiFiles(ctx: AiMockCtx) {
  const { deps, turns, fail, delay, need, sleep, openCall } = ctx;

  /** The file tools' host (AI-38…40): a few files every tab shares. `~/` is the home directory of `deploy`. */
  const remoteFiles = new Map<string, string>([
    [
      "/etc/nginx/sites-available/default",
      [
        "# Default server configuration",
        "#",
        "server {",
        "    listen 80 default_server;",
        "    listen [::]:80 default_server;",
        "",
        "    root /var/www/html;",
        "    index index.html index.htm;",
        "",
        "    server_name _;",
        "",
        "    location / {",
        "        try_files $uri $uri/ =404;",
        "    }",
        "",
        "    location /api/ {",
        "        proxy_pass http://127.0.0.1:8081/;",
        "        proxy_read_timeout 30s;",
        "    }",
        "",
        "    access_log /var/log/nginx/access.log;",
        "    error_log /var/log/nginx/error.log warn;",
        "}",
        "",
      ].join("\n"),
    ],
    ["/etc/motd", "Welcome to prod-api.\nMaintenance window: Sundays 02:00–04:00 UTC.\n"],
    [
      "/srv/app/web.config",
      '<?xml version="1.0"?>\r\n<configuration>\r\n  <appSettings>\r\n    <add key="mode" value="production" />\r\n  </appSettings>\r\n</configuration>\r\n',
    ],
  ]);
  const snapshots = new Map<string, Snapshot[]>();
  /** Like Rust's trim: drops what the oldest changes held until the kept ones fit; true when it dropped any. */
  function trimSnapshots(): boolean {
    const kept = [...snapshots.values()]
      .flat()
      .filter((s) => s.change.state === "written" || s.change.state === "unfinished");
    let total = kept.reduce((n, s) => n + (s.before?.length ?? 0) + s.after.length, 0);
    let dropped = false;
    for (const s of kept) {
      if (total <= SNAPSHOT_CAP) break;
      total -= (s.before?.length ?? 0) + s.after.length;
      s.change = { ...s.change, state: "dropped" };
      dropped = true;
    }
    return dropped;
  }
  /** Files a link was put in the place of ("symlink"), which reads go through and a revert's deletion refuses. */
  const remoteLinks = new Set<string>();
  /** Calls whose snapshot dropped older ones, as `ai_tool_run` reports it. */
  const droppedBy = new Set<string>();
  const fileChangesOf = (convId: string) => (snapshots.get(convId) ?? []).map((s) => ({ ...s.change }));
  /** Like Rust's `pending_reverts`: reverted calls in `earlier` that no user entry after the call notes yet, oldest call first. */
  function pendingReverts(convId: string, earlier: AiEntryView[]): Note[] {
    const at = (s: Snapshot) => earlier.findIndex((e) => e.entry_id === s.change.response_id);
    const order = (s: Snapshot) => {
      const e = earlier[at(s)];
      return e?.role === "assistant" ? e.tool_calls.findIndex((x) => x.id === s.change.tool_call_id) : -1;
    };
    return (
      (snapshots.get(convId) ?? [])
        .filter((s) => s.change.state === "reverted" && at(s) >= 0 && order(s) >= 0)
        // A note is about the call of the newest response before it with that id.
        .filter((s) => {
          const id = s.change.tool_call_id;
          return !earlier.some((e, j) => {
            if (
              j <= at(s) ||
              e.role !== "user" ||
              !parseMessage(e.text).notes.some((n) => n.kind === "file_revert" && n.toolCallId === id)
            )
              return false;
            return (
              earlier
                .slice(0, j)
                .findLastIndex((x) => x.role === "assistant" && x.tool_calls.some((c) => c.id === id)) === at(s)
            );
          });
        })
        .sort((a, b) => at(a) - at(b) || order(a) - order(b))
        .map((s) => fileRevertNote(s.change.path, s.change.tool_call_id, s.tool, s.change.created))
    );
  }

  /** What each approval card read, like Rust's: the call writes only over it (AI-39). */
  const fileBases = new Map<string, string | null>();
  /** What the model has seen of each file, by conversation and resolved path, like Rust's (§13.4). */
  const seenFiles = new Map<string, string>();
  const seenKey = (convId: string, path: string) => `${convId}:${filePath(path)}`;
  const forgetSeen = (convId: string) => {
    for (const key of [...seenFiles.keys()]) if (key.startsWith(`${convId}:`)) seenFiles.delete(key);
  };
  /** Why `name` may not change the file at `path`, which holds `current`; null when the model saw it so. */
  function unseen(convId: string, name: string, path: string, current: string): string | null {
    const seen = seenFiles.get(seenKey(convId, path));
    if (seen === undefined)
      return `${path} has not been read in this conversation, so ${name} did not change it. Call read_file on it, then call ${name} again.`;
    if (seen !== current)
      return `${path} changed since you last read it, by the user or another program, so ${name} did not change it. Call read_file on it again, then call ${name} again with what it holds now.`;
    return null;
  }

  /**
   * AI-48, AI-49: `glob` and `grep` over the mock's files, as `hatoba_ai::search` words their results, as on a
   * host with rg. The glob is read roughly (`**` any directories, `*` and `?` within a name), the pattern as a
   * JS RegExp. A file given as grep's path is searched whatever its glob, as with rg. Claude Code's argument
   * names work too.
   */
  function search(name: string, args: Record<string, unknown>): { status: AiToolStatus; content: string } {
    const root = filePath(String(args.path || "~")).replace(/\/$/, "") || "/";
    const glob = String((name === "glob" ? args.pattern : args.glob) ?? "").trim();
    if (name === "glob" && !glob)
      return {
        status: "error",
        content: "The pattern is empty, so nothing was searched. Give one such as *.conf or **/*.conf.",
      };
    const source = glob
      .replace(/[.+^$()|\\]/g, "\\$&")
      .replace(/\*\*\//g, "\u0000")
      .replace(/\*/g, "[^/]*")
      .replace(/\?/g, "[^/]")
      .replace(/\u0000/g, "(?:.*/)?");
    const globRe = new RegExp(glob.includes("/") ? `^${source}$` : `(?:^|/)${source}$`);
    const file = name === "grep" && remoteFiles.has(root);
    const paths = file
      ? [root]
      : [...remoteFiles.keys()]
          .filter((p) => p.startsWith(`${root}/`) && (!glob || globRe.test(p.slice(root.length + 1))))
          .sort();
    const place = file ? `in ${root}` : `under ${root}`;
    const files = (n: number) => (n === 1 ? "1 file" : `${n} files`);
    const ok = (content: string) => ({ status: "ok" as const, content });
    if (name === "glob")
      return ok(
        paths.length
          ? `${files(paths.length)} under ${root} ${paths.length === 1 ? "matches" : "match"} ${glob}, sorted by path:\n${paths.join("\n")}\n`
          : `No files under ${root} match ${glob}.\n`,
      );
    const arg = (name: string, alias: string) => args[name] ?? args[alias];
    const multiline = Boolean(args.multiline);
    let re: RegExp;
    try {
      re = new RegExp(String(args.pattern ?? ""), `g${arg("ignore_case", "-i") ? "i" : ""}${multiline ? "ms" : ""}`);
    } catch (e) {
      return { status: "error", content: `rg on the host refused the pattern, so nothing was searched: ${String(e)}.` };
    }
    // The numbers of the lines each match spans: one per line unless multiline.
    const matched = (text: string): Set<number> => {
      const lines = fileLines(text);
      if (!multiline) return new Set(lines.flatMap((l, i) => (l.match(re) ? [i] : [])));
      const starts = lines.map((_, i) => lines.slice(0, i).reduce((n, l) => n + l.length + 1, 0));
      const lineAt = (offset: number) => starts.filter((s) => s <= offset).length - 1;
      return new Set(
        [...text.matchAll(re)].flatMap((m) => {
          const first = lineAt(m.index ?? 0);
          const last = lineAt((m.index ?? 0) + Math.max(m[0].length - 1, 0));
          return Array.from({ length: last - first + 1 }, (_, k) => first + k);
        }),
      );
    };
    const hits = paths
      .map((p) => ({ p, lines: fileLines(remoteFiles.get(p) ?? ""), hit: matched(remoteFiles.get(p) ?? "") }))
      .filter((f) => f.hit.size);
    const matching = hits.reduce((n, f) => n + f.hit.size, 0);
    if (!hits.length) return ok(`No lines match the pattern ${place}.\n`);
    if (args.output_mode === "content") {
      const context = Number(arg("context", "-C") ?? 0) || 0;
      const before = Number(arg("before_context", "-B") ?? context) || 0;
      const after = Number(arg("after_context", "-A") ?? context) || 0;
      const numbers = arg("line_numbers", "-n") !== false;
      const out = hits.flatMap(({ p, lines, hit }) => {
        const shown = new Set(
          [...hit].flatMap((i) => Array.from({ length: before + after + 1 }, (_, k) => i - before + k)),
        );
        return lines.flatMap((l, i) => {
          if (!shown.has(i)) return [];
          const separator = hit.has(i) ? ":" : "-";
          return [numbers ? `${p}${separator}${i + 1}${separator}${l}` : `${p}${separator}${l}`];
        });
      });
      return ok(`${matching} matching lines in ${files(hits.length)} ${place}, sorted by path:\n${out.join("\n")}\n`);
    }
    if (args.output_mode === "count")
      return ok(
        `${matching} matching lines in ${files(hits.length)} ${place}, sorted by path:\n${hits.map(({ p, hit }) => `${p}:${hit.size}`).join("\n")}\n`,
      );
    return ok(
      `${files(hits.length)} with a matching line ${place}, sorted by path:\n${hits.map((h) => h.p).join("\n")}\n`,
    );
  }

  async function runTool(
    c: Conv,
    x: AiToolCall,
    sessionId: string | null,
    edited: string | null,
  ): Promise<{ status: AiToolStatus; content: string }> {
    let args: Record<string, unknown>;
    try {
      args = JSON.parse(edited ?? x.arguments) as Record<string, unknown>;
    } catch {
      return { status: "error", content: "The arguments are not valid JSON." };
    }
    const turn = turns.get(c.view.id);
    const wait = (ms: number) => (turn ? sleep(turn, ms) : delay(ms).then(() => true));
    switch (x.name) {
      case "run_command": {
        if (!sessionId) return { status: "error", content: "The terminal tab is not connected." };
        const command = String(args.command ?? "");
        // AI-41: files changed behind a call's back, for its revert to find.
        if (command === "rm /etc/motd") remoteFiles.delete("/etc/motd");
        if (command.startsWith("ln -sfn ") && remoteFiles.has(MAINTENANCE)) remoteLinks.add(MAINTENANCE);
        const secs = Number(/sleep\s+(\d+)/.exec(command)?.[1] ?? 0);
        await wait(secs ? Math.min(secs, 15) * 1000 : 1100);
        const r = exec(command);
        return {
          status: "ok",
          content: `exit status: ${r.status}\n--- stdout ---\n${r.stdout}\n--- stderr ---\n${r.stderr}`,
        };
      }
      case "glob":
      case "grep": {
        if (!sessionId) return { status: "error", content: FILE_DISCONNECTED };
        await wait(500);
        return search(x.name, args);
      }
      case "read_file": {
        if (!sessionId) return { status: "error", content: FILE_DISCONNECTED };
        await wait(500);
        const path = String(args.path ?? "");
        const text = remoteFiles.get(filePath(path));
        if (text === undefined) return { status: "error", content: `${path} does not exist.` };
        seenFiles.set(seenKey(c.view.id, path), text);
        const lines = fileLines(text);
        const from = Math.max(1, Number(args.offset ?? 1) || 1);
        const shown = lines.slice(from - 1, args.limit ? from - 1 + Number(args.limit) : undefined);
        const crlf = crlfOf(text) ? ", CRLF line breaks" : "";
        const head =
          shown.length === lines.length
            ? `${path}: ${lines.length} lines${crlf}`
            : `${path}: lines ${from}–${from + shown.length - 1} of ${lines.length}${crlf}`;
        return {
          status: "ok",
          content: `${head}\n${shown.map((l, i) => `${String(from + i).padStart(6)}\t${l}`).join("\n")}\n`,
        };
      }
      case "edit_file":
      case "write_file": {
        if (!sessionId) return { status: "error", content: FILE_DISCONNECTED };
        await wait(600);
        const path = String(args.path ?? "");
        const key = filePath(path);
        const current = remoteFiles.get(key) ?? null;
        const seen = fileBases.has(x.id) ? fileBases.get(x.id)! : current;
        fileBases.delete(x.id);
        if (seen !== current)
          return {
            status: "error",
            content: `${path} changed on the host after Hatoba read it for the approval card, so nothing was written. Read it again with read_file before you change it.`,
          };
        const refused = current === null ? null : unseen(c.view.id, x.name, path, current);
        if (refused) return { status: "error", content: refused };
        const after = fileAfter(x.name, args, current);
        if ("error" in after) return { status: "error", content: after.error };
        // Like Rust: the snapshot is kept before the write begins, unfinished until it ends.
        const response = c.entries.find((e) => e.role === "assistant" && e.tool_calls.some((t) => t.id === x.id));
        let snapshot: Snapshot | null = null;
        if (response) {
          const kept = (snapshots.get(c.view.id) ?? []).filter((s) => s.change.tool_call_id !== x.id);
          const host = ((await deps.hosts?.()) ?? []).find((h) => h.id === c.view.host_id);
          const server = host
            ? formatTarget(host)
            : c.view.quick_target
              ? formatTarget(c.view.quick_target)
              : c.view.local
                ? "local"
                : "root@localhost:22";
          const change: AiFileChange = {
            response_id: response.entry_id,
            tool_call_id: x.id,
            server,
            path: key,
            created: current === null,
            state: "unfinished",
          };
          snapshot = { change, tool: x.name, before: current, after: after.text };
          snapshots.set(c.view.id, [...kept, snapshot]);
          if (trimSnapshots()) droppedBy.add(x.id);
        }
        if (key === CUT_SHORT) {
          remoteFiles.set(key, after.text.slice(0, Math.ceil(after.text.length / 2)));
          return {
            status: "error",
            content: `Writing ${path} failed: the connection closed. The file may be unchanged, cut short or empty, so read it with read_file before you change it.`,
          };
        }
        remoteFiles.set(key, after.text);
        seenFiles.set(seenKey(c.view.id, path), after.text);
        if (snapshot?.change.state === "unfinished") snapshot.change = { ...snapshot.change, state: "written" };
        const count = fileLines(after.text).length;
        if (x.name === "edit_file")
          return { status: "ok", content: `Edited ${path}: replaced 1 occurrence of old_string.\n` };
        return {
          status: "ok",
          content:
            current === null
              ? `Created ${path} (${count} lines).\n`
              : `Replaced the content of ${path} (${count} lines).\n`,
        };
      }
      case "web_search":
        await wait(700);
        return {
          status: "ok",
          content: [
            `1. nginx 502 Bad Gateway: upstream prematurely closed connection\n   https://serverfault.com/questions/nginx-502-upstream\n   The upstream closed the connection before sending a full response; check its logs and timeouts.`,
            `2. Module ngx_http_upstream_module\n   https://nginx.org/en/docs/http/ngx_http_upstream_module.html\n   Directives for groups of upstream servers: keepalive, max_fails, fail_timeout.`,
            `3. Debugging 502 errors behind a reverse proxy\n   https://example.com/blog/debugging-502\n   A checklist: is the upstream listening, does it crash, are the proxy timeouts too short.`,
          ].join("\n\n"),
        };
      case "fetch_url":
        await wait(900);
        return {
          status: "ok",
          content: `URL: ${String(args.url ?? "")} | Content type: text/html | ${args.full_page === true ? "Whole page" : "Main content"} | Characters 0-230 of 230\nTitle: Module ngx_http_upstream_module\n${args.full_page === true ? "" : "Navigation, sidebars and footers are left out. For the whole page, call fetch_url with full_page true.\n"}\n# Module ngx_http_upstream_module\n\nThe ngx_http_upstream_module module is used to define groups of servers that can be referenced by the proxy_pass directive.\n\n## keepalive\n\nActivates the cache for connections to upstream servers.`,
        };
      case "memory": {
        await wait(150);
        if (!deps.memory) return { status: "error", content: "Memory is not available." };
        // Like Rust's `memory_place`: a saved host that exists, or a computer's local terminal.
        const host = ((await deps.hosts?.()) ?? []).find((h) => h.id === c.view.host_id);
        const local = !c.view.host_id && !c.view.quick_target ? c.view.local : null;
        const place: MemoryPlace | null = host
          ? { scope: `host:${host.id}`, name: host.name }
          : local
            ? { scope: `local:${local}`, name: localName(local) }
            : null;
        const text = (key: string) => String(args[key] ?? "");
        return deps.memory(
          { action: text("action"), scope: text("scope"), name: text("name"), content: text("content") },
          place,
        );
      }
      case "read_skill":
        await wait(200);
        if (args.name === "hatoba")
          return {
            status: "ok",
            content:
              "# Troubleshooting connections (demo excerpt)\n\n## ETIMEDOUT (timeout)\n\nThe host did not answer within 15 seconds: it is off, a firewall drops the packets, or the address or port is wrong.\n\n## ECONNREFUSED (refused)\n\nThe host answered but nothing listens on the port: the SSH server is stopped or listens elsewhere.",
          };
        return { status: "error", content: `There is no enabled skill named "${String(args.name ?? "")}".` };
      case "search_tools": {
        // AI-45: the running servers' tools whose names or descriptions have a word of the query.
        await wait(300);
        const words = String(args.query ?? "")
          .toLowerCase()
          .split(/\s+/)
          .filter(Boolean);
        const found: string[] = [];
        const servers = (await deps.mcp?.servers().catch(() => [])) ?? [];
        for (const sv of servers.filter((x) => x.enabled && !turn?.context.disabled_mcp_servers.includes(x.id))) {
          const status = await deps.mcp!.status(sv.id).catch(() => null);
          for (const tool of status?.state === "running" ? status.tools : []) {
            const text = `${tool.name} ${tool.description}`.toLowerCase();
            if (words.some((w) => text.includes(w.replace(/s$/, ""))))
              found.push(
                `## ${tool.name}\nServer: ${sv.name}\nDescription: ${tool.description || "(none)"}\nInput schema: {"type":"object","properties":{"repo":{"type":"string"}}}`,
              );
          }
        }
        return {
          status: "ok",
          content:
            found.length === 0
              ? `No MCP tool matches "${String(args.query ?? "")}".`
              : `${found.length} matches (demo).\n\n${found.slice(0, 5).join("\n\n")}`,
        };
      }
      default: {
        // An MCP tool (AI-30): the server's text content, or an error result when no running server offers it.
        await wait(700);
        // AI-45: `call_mcp_tool` runs the tool it names with that tool's arguments.
        if (x.name === "call_mcp_tool") {
          const named = typeof args.tool === "string" ? args.tool : "";
          // Like Rust: an object, or a string that holds one.
          let inner: unknown = args.arguments;
          if (typeof inner === "string") {
            try {
              inner = JSON.parse(inner);
            } catch {
              inner = null;
            }
          }
          args = (inner && typeof inner === "object" && !Array.isArray(inner) ? inner : {}) as Record<string, unknown>;
          x = { ...x, name: named };
        }
        const info = (await deps.mcp?.toolInfo(c.view.id, x.name).catch(() => null)) ?? null;
        if (!info || turn?.context.disabled_mcp_servers.includes(info.server_id))
          return { status: "error", content: `No running MCP server offers the tool ${x.name}.` };
        return {
          status: "ok",
          content: `${info.server_name} / ${info.tool.tool} (demo result)\n\n${JSON.stringify(
            {
              arguments: args,
              items: [
                { title: "nginx 502 after deploy", state: "open" },
                { title: "upstream timeouts", state: "closed" },
              ],
            },
            null,
            2,
          )}`,
        };
      }
    }
  }

  const api: AiFilesApi = {
    ai_file_preview: async (id, callId, sessionId, edited) => {
      const c = need(id);
      const x = openCall(c, callId);
      let args: Record<string, unknown>;
      try {
        args = JSON.parse(edited ?? x.arguments) as Record<string, unknown>;
      } catch {
        return { path: "", before: null, after: null, error: "The arguments are not valid JSON.", crlf: false };
      }
      const path = String(args.path ?? "");
      const failed = (error: string) => ({ path, before: null, after: null, error, crlf: false });
      if (x.name !== "edit_file" && x.name !== "write_file") return failed(`${x.name} shows no diff.`);
      // Like Rust: the file is read once per call, and the run writes only over what was read.
      if (!fileBases.has(callId)) {
        if (!sessionId) return failed(FILE_DISCONNECTED);
        await delay(450);
        fileBases.set(callId, remoteFiles.get(filePath(path)) ?? null);
      }
      const before = fileBases.get(callId) ?? null;
      const refused = before === null ? null : unseen(id, x.name, path, before);
      if (refused) return failed(refused);
      const after = fileAfter(x.name, args, before);
      if ("error" in after) return failed(after.error);
      const crlf = before !== null && crlfOf(before);
      const shown = (text: string) => (crlf ? text.replace(/\r\n/g, "\n") : text);
      return { path, before: before === null ? null : shown(before), after: shown(after.text), error: null, crlf };
    },
    ai_file_changes: async (id) => {
      need(id);
      return fileChangesOf(id);
    },
    ai_file_change_diff: async (id, responseId, callId) => {
      need(id);
      const snapshot = (snapshots.get(id) ?? []).find(
        (s) => s.change.response_id === responseId && s.change.tool_call_id === callId,
      );
      if (!snapshot) return fail("not_found", "file change not found");
      if (snapshot.change.state !== "written" && snapshot.change.state !== "unfinished") return null;
      const shown = (text: string) => text.replace(/\r\n/g, "\n");
      const crlf = snapshot.before !== null && crlfOf(snapshot.before);
      return {
        path: snapshot.change.path,
        before: snapshot.before === null ? null : shown(snapshot.before),
        after: shown(snapshot.after),
        error: null,
        crlf,
      };
    },
    ai_file_revert: async (id, responseId, callId, sessionId, restore) => {
      const c = need(id);
      if (turns.has(id) || c.runningUntil > Date.now())
        fail("invalid_input", "The conversation is busy; revert the change once it is idle.", {
          field: "conversation_id",
        });
      const list = snapshots.get(id) ?? [];
      const snapshot = list.find((s) => s.change.response_id === responseId && s.change.tool_call_id === callId);
      if (!snapshot) return fail("not_found", "file change not found");
      if (snapshot.change.state === "reverted")
        fail("invalid_input", "the change was reverted already", { field: "tool_call_id" });
      if (snapshot.change.state === "dropped")
        fail("invalid_input", "the change can no longer be reverted: the snapshot limit dropped it", {
          field: "tool_call_id",
        });
      if (!sessionId) return { status: "failed", reason: "disconnected", detail: null } satisfies AiFileRevert;
      await delay(500);
      const path = snapshot.change.path;
      const current = remoteFiles.get(path) ?? null;
      const shown = (text: string) => text.replace(/\r\n/g, "\n");
      const reverted = (wrote: boolean) => {
        snapshot.change = { ...snapshot.change, state: "reverted" };
        delete snapshot.shown;
        // The model reads the file again before it changes it.
        seenFiles.delete(`${id}:${path}`);
        return { status: "reverted", wrote } satisfies AiFileRevert;
      };
      // Like Rust: (b) the file holds what it held before, or the file the call created is gone.
      if (current === snapshot.before) return reverted(false);
      // (a) It holds what the call wrote; or with Restore anyway, what the diff showed.
      if (current === snapshot.after || (restore && snapshot.shown !== undefined && current === snapshot.shown)) {
        // Like Rust's delete script: a link in the place of the file the call created is not deleted.
        if (snapshot.before === null && remoteLinks.has(path))
          return { status: "failed", reason: "symbolic_link", detail: null } satisfies AiFileRevert;
        if (snapshot.before === null) remoteFiles.delete(path);
        else remoteFiles.set(path, snapshot.before);
        return reverted(true);
      }
      // (c) Anything else: the diff to what it held before.
      snapshot.shown = current;
      const at = list.indexOf(snapshot);
      const later = list
        .slice(at + 1)
        .some((s) => s.change.path === path && (s.change.state === "written" || s.change.state === "unfinished"));
      return {
        status: "changed",
        path,
        current: current === null ? null : shown(current),
        before: snapshot.before === null ? null : shown(snapshot.before),
        later,
      } satisfies AiFileRevert;
    },
  };

  return {
    api,
    runTool,
    fileChangesOf,
    pendingReverts,
    forgetSeen,
    clearFileBase: (callId: string) => fileBases.delete(callId),
    consumeDropped: (callId: string) => droppedBy.delete(callId),
    deleteSnapshots: (convId: string) => snapshots.delete(convId),
    keepSnapshots: (convId: string, entries: AiEntryView[]) => {
      snapshots.set(
        convId,
        (snapshots.get(convId) ?? []).filter((s) => entries.some((e) => e.entry_id === s.change.response_id)),
      );
    },
  };
}
