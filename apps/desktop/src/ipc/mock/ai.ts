import type { TabSource } from "@/app/tabs";
import { noteBlock, parseMessage, type Note } from "@/features/ai/attachments";
import { effectiveEffort, modelEfforts } from "@/features/ai/effort";
import { movedFrom, moves, recordedTarget } from "@/features/ai/place";
import { formatTarget, sameTarget } from "@/features/hosts/quickConnect";
import { detectLocale } from "@/i18n";
import type { HatobaApi } from "../api";
import type {
  AiConversationView,
  AiEffort,
  AiEntryView,
  AiFinish,
  AiSearchHit,
  AiToolCall,
  AiToolResultInput,
  AiToolStatus,
  AiTurnContext,
  AiTurnEndReason,
  AiTurnEvent,
  AiUsage,
  AppError,
  HostView,
  QuickTarget,
} from "../types";

type AiApi = Pick<
  HatobaApi,
  | "ai_conversations_list"
  | "ai_conversation_get"
  | "ai_conversation_rename"
  | "ai_conversation_pin"
  | "ai_conversation_delete"
  | "ai_send"
  | "ai_retry"
  | "ai_tool_result"
  | "ai_tool_run"
  | "ai_file_preview"
  | "ai_stop"
  | "ai_compact"
  | "ai_search"
  | "ai_edit_resend"
  | "ai_read_dropped_files"
>;

/** What the conversation mock reads from the settings and MCP mocks. */
export interface AiMockDeps {
  providers: HatobaApi["ai_providers_list"];
  settings: HatobaApi["ai_settings_get"];
  /** The hosts mock's list, whose names the notes of a move to another host carry (AI-09). */
  hosts?: () => Promise<HostView[]>;
  /** The MCP servers of the extensions mock, whose tools the fake model calls. */
  mcp?: {
    servers: HatobaApi["mcp_servers_list"];
    status: HatobaApi["mcp_server_status"];
    start: HatobaApi["mcp_server_start"];
    toolInfo: HatobaApi["mcp_tool_info"];
  };
}

type Assistant = Extract<AiEntryView, { role: "assistant" }>;
type ToolEntry = Extract<AiEntryView, { role: "tool" }>;

interface Conv {
  view: Omit<AiConversationView, "last_activity">;
  entries: AiEntryView[];
  /** `?ai=running`: a turn "runs in Rust" until then, started by an earlier page. */
  runningUntil: number;
}

/** Like Rust's `Place` (AI-09): where a message moves its conversation, and where from (`moveOf`). */
interface Move {
  from: string | null;
  host_id: string | null;
  quick_target: QuickTarget | null;
}

interface Turn {
  convId: string;
  emit: (ev: AiTurnEvent) => void;
  context: AiTurnContext;
  ended: boolean;
  timers: Set<number>;
  /** Pending `sleep`s, which end (with false) when the turn ends. */
  sleepers: Set<() => void>;
  /** AI-22 ran once in this turn. */
  compacted: boolean;
}

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/**
 * Mock of the AI conversations and turns (§13) for `pnpm dev`. A scripted fake model streams
 * reasoning and then Markdown with timers, and calls tools by keyword in the user's message (English,
 * Chinese or Japanese). Without a connected terminal tab it calls only the tools that need none (AI-09):
 *   "disk"            run_command `df -h /`          "fail"     run_command that exits with 5
 *   "screen"          read_terminal                  "type" / "send"   send_input `uptime` + Enter
 *   "search"          web_search                     "fetch" / "url"   fetch_url
 *   "mcp"             a tool of the first running (or startable) MCP server the conversation uses,
 *                     from the extensions mock (`?mcp=demo`), preferring a server named in the message
 *                     ("mcp filesystem"); without one, a tool no server offers
 *   "unknown"         a tool that does not exist     "sleep"    run_command `sleep 8`, to see Stop
 *   "hidden"          run_command with several lines, a tab, an escape sequence and a right-to-left
 *                     override, to see how the approval card shows them (AI-17)
 *   "read file"       read_file of an nginx site        "edit"     edit_file of its proxy block
 *   "listen"          edit_file with replace_all         "nomatch"  edit_file whose old_string is missing
 *   "crlf"            edit_file of a CRLF file           "write"    write_file of a new file
 *   "bigfile"         write_file of a 1,200-line file, to see the diff paged
 *   "motd"            write_file over an existing file   (AI-38…40: the cards show diffs; the files are
 *                     shared by every tab, and a run changes them for the next card)
 * It waits for every result, then answers from them. Other messages get a Markdown sample.
 * Keywords come from the typed text. A message without one gets an answer about what it carries:
 * pasted text and files (AI-35) with their size, else the terminal selection (AI-10).
 * Before a request that would pass 90% of the model's context window, the turn compacts the
 * conversation first (AI-22): a `summary` entry arrives and the context starts there.
 * `?ai=` demo values (comma-separated, shared with the Settings → AI mock):
 *   history    prefilled history: a pinned conversation, a compacted one near the context limit,
 *              one with tool calls in every state, and one on a host with no context window
 *   running    history as above, and the newest conversation has a turn "running in Rust" (started
 *              by an earlier page) for 20 s; open it from History to see it, or Stop it
 *   error      the first request fails with HTTP 529 and the provider's message; Retry succeeds
 *   refused    every response is declined by the model
 *   length     every response is cut off at the output limit
 *   limit      the model keeps calling read_terminal, so the turn pauses at the tool call limit
 *   autocompact every turn of a conversation that has an answer compacts it first (AI-22)
 *   effortfail every request that carries a thinking level is refused for it and goes again
 *              without it, so the panel says the level was not used (AI-05)
 *   showcase   in a connected tab, any message gets the scripted nginx 502 fix that the README
 *              screenshots and demo video show: two commands, a fix, `curl` typed into the shell, a summary
 *   noprovider (Settings → AI mock) no provider is configured
 * History search (AI-24) matches titles and the text of user, assistant and summary entries.
 * Like Rust, a message that moves the conversation to another host (AI-09) or goes to another model
 * than the last reply (AI-05) is stored with Hatoba's note before it, which the panel shows as a
 * divider: open a conversation from history in a tab on another host, or pick another model.
 * Nothing here is secure; it never runs inside the Tauri app.
 */
export function createAiMock(deps: AiMockDeps): AiApi {
  const flags = new Set((new URLSearchParams(location.search).get("ai") ?? "").split(",").map((x) => x.trim()));
  const zh = detectLocale() === "zh-CN";
  const convs = new Map<string, Conv>();
  const turns = new Map<string, Turn>();
  /** By reply entry id: whether its request offered the terminal tools (AI-09). */
  const replyTerminal = new Map<string, boolean>();
  let errorShown = false;
  let lastMs = 0;
  let seq = 0;

  /** Increasing ids, like the UUIDv7 entry ids Rust makes. */
  const newId = (ms = Date.now()) => {
    lastMs = Math.max(ms, lastMs + 1);
    return `${lastMs.toString(16).padStart(12, "0")}-7${(++seq).toString(16).padStart(5, "0")}`;
  };
  const fail = (code: AppError["code"], detail: string, extra: Partial<AppError> = {}): never => {
    throw { code, detail, ...extra } satisfies AppError;
  };
  const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

  function need(id: string): Conv {
    return convs.get(id) ?? fail("not_found", "conversation not found");
  }

  function view(c: Conv): AiConversationView {
    const last = c.entries.reduce((m, e) => Math.max(m, e.created_at), 0);
    return { ...c.view, last_activity: Math.max(c.view.updated_at, last) };
  }

  function answered(c: Conv): Set<string> {
    return new Set(c.entries.flatMap((e) => (e.role === "tool" ? [e.tool_call_id] : [])));
  }

  function unanswered(c: Conv): AiToolCall[] {
    const done = answered(c);
    return c.entries.flatMap((e) => (e.role === "assistant" ? e.tool_calls.filter((x) => !done.has(x.id)) : []));
  }

  function store(c: Conv, entry: AiEntryView) {
    c.entries.push(entry);
    turns.get(c.view.id)?.emit({ kind: "entry", entry });
  }

  /** §13.1 step 5: every call without a result gets a cancelled one. Returns the results it stored. */
  function cancelOpenCalls(c: Conv): AiEntryView[] {
    return unanswered(c).map((call) => {
      const entry: AiEntryView = { role: "tool", entry_id: newId(), created_at: Date.now(), tool_call_id: call.id, status: "cancelled", content: "The call was cancelled before it ran." };
      store(c, entry);
      return entry;
    });
  }

  function endTurn(turn: Turn, reason: AiTurnEndReason) {
    if (turn.ended) return;
    turn.ended = true;
    turn.timers.forEach((t) => clearTimeout(t));
    turn.sleepers.forEach((wake) => wake());
    turn.sleepers.clear();
    turn.emit({ kind: "turn_ended", reason });
    if (turns.get(turn.convId) === turn) turns.delete(turn.convId);
  }

  /** Like Rust's stop: the cancelled results and `turn_ended { stopped }` go to the running turn's channel. */
  function stopTurn(convId: string): AiEntryView[] {
    const turn = turns.get(convId);
    const c = convs.get(convId);
    if (turn) turn.timers.forEach((t) => clearTimeout(t));
    const cancelled = c ? cancelOpenCalls(c) : [];
    if (turn) endTurn(turn, "stopped");
    return cancelled;
  }

  /** Resolves false once the turn has ended, also when it ends while sleeping. */
  function sleep(turn: Turn, ms: number): Promise<boolean> {
    return new Promise((resolve) => {
      if (turn.ended) return resolve(false);
      const wake = () => resolve(false);
      const t = window.setTimeout(() => {
        turn.timers.delete(t);
        turn.sleepers.delete(wake);
        resolve(!turn.ended);
      }, ms);
      turn.timers.add(t);
      turn.sleepers.add(wake);
    });
  }

  function chunks(text: string): string[] {
    const out: string[] = [];
    for (let i = 0; i < text.length; ) {
      const n = 3 + Math.floor(Math.random() * 7);
      out.push(text.slice(i, i + n));
      i += n;
    }
    return out;
  }

  const call = (name: string, args: Record<string, unknown>): AiToolCall => ({ id: `call_${(++seq).toString(36)}`, name, arguments: JSON.stringify(args) });

  // ───────────── the fake model ─────────────

  interface Plan {
    reasoning: string;
    text: string;
    calls: AiToolCall[];
    finish: AiFinish;
  }

  function lastUser(c: Conv): { text: string; index: number } {
    for (let i = c.entries.length - 1; i >= 0; i--) {
      const e = c.entries[i];
      if (e.role === "user") return { text: e.text, index: i };
    }
    return { text: "", index: -1 };
  }

  /**
   * A call of a tool the conversation's MCP servers offer: a running server's, or one started for it
   * (as Rust starts a `stdio` server when a conversation first needs its tools, AI-32). A tool that
   * asks is preferred, to show the approval card. Without one, a name no server offers.
   */
  async function mcpCall(turn: Turn, words: string): Promise<AiToolCall> {
    const args = { query: "nginx 502 bad gateway", limit: 5 };
    const mcp = deps.mcp;
    if (mcp) {
      try {
        // A server named in the message goes first ("mcp filesystem").
        const named = (name: string) => (words.includes(name.toLowerCase()) ? 0 : 1);
        const servers = (await mcp.servers())
          .filter((s) => s.enabled && !turn.context.disabled_mcp_servers.includes(s.id))
          .sort((a, b) => named(a.name) - named(b.name));
        for (const s of servers) {
          let status = await mcp.status(s.id);
          if (status.state === "stopped") status = await mcp.start(s.id);
          const tool = status.tools.find((x) => !x.always_allow) ?? status.tools[0];
          if (status.state === "running" && tool) return call(tool.name, args);
        }
      } catch {
        // fall through to a tool no server offers
      }
    }
    return call("mcp__github__list_issues", { repo: "scarletkc/Hatoba", state: "open" });
  }

  /** `?ai=showcase`: step `step` (the replies so far in this turn) of the scripted nginx 502 fix. */
  function showcase(step: number): Plan {
    if (step === 0)
      return {
        reasoning: zh
          ? "502 说明 nginx 连不上上游。先看 nginx 的错误日志，再看 API 实际监听的端口。"
          : "A 502 means nginx could not get a response from the upstream. Check nginx’s error log, then the port the API actually listens on.",
        text: zh ? "我先看 nginx 的错误日志和 API 监听的端口。" : "Let me check nginx’s error log and the port the API listens on.",
        calls: [
          call("run_command", { command: "sudo tail -n 3 /var/log/nginx/error.log", timeout_seconds: 30 }),
          call("run_command", { command: "sudo ss -ltnp | grep api", timeout_seconds: 30 }),
        ],
        finish: "tool_calls",
      };
    if (step === 1)
      return {
        reasoning: zh ? "nginx 连的是 8081，API 监听的是 8080。改 upstream，测试配置后重新加载。" : "nginx connects to 8081, but the API listens on 8080. Fix the upstream, test the config, then reload.",
        text: zh
          ? "找到了：nginx 把请求转发到 `127.0.0.1:8081`，但 `api` 监听的是 `127.0.0.1:8080`。我把 upstream 改成 8080，测试配置后重新加载 nginx。"
          : "Found it: nginx forwards to `127.0.0.1:8081`, but `api` listens on `127.0.0.1:8080`. I’ll point the upstream at 8080, test the config, and reload nginx.",
        calls: [
          call("run_command", {
            command: "sudo sed -i 's/127.0.0.1:8081/127.0.0.1:8080/' /etc/nginx/conf.d/api.conf && sudo nginx -t && sudo systemctl reload nginx",
            timeout_seconds: 30,
          }),
        ],
        finish: "tool_calls",
      };
    if (step === 2)
      return {
        reasoning: "",
        text: zh ? "nginx 已重新加载。我在你的终端里验证一下。" : "nginx reloaded. I’ll verify it from your shell.",
        calls: [call("send_input", { text: "curl -sI http://localhost/v1/hosts", key: "enter", wait_seconds: 3 })],
        finish: "tool_calls",
      };
    return {
      reasoning: "",
      text: zh
        ? "**修好了。** 502 来自端口不一致：\n\n| 检查项 | 之前 | 现在 |\n|---|---|---|\n| nginx upstream | `127.0.0.1:8081` | `127.0.0.1:8080` |\n| `GET /v1/hosts` | 502 Bad Gateway | **200 OK** |\n\n上次部署把 API 改到了 8080 端口，但 `/etc/nginx/conf.d/api.conf` 还指向 8081。可以在部署脚本最后加一个检查，下次就能马上发现：\n\n```bash\ncurl -fsS http://localhost/healthz || exit 1\n```"
        : "**Fixed.** The 502s came from a port mismatch:\n\n| Check | Before | Now |\n|---|---|---|\n| nginx upstream | `127.0.0.1:8081` | `127.0.0.1:8080` |\n| `GET /v1/hosts` | 502 Bad Gateway | **200 OK** |\n\nThe last deploy moved the API to port 8080, but `/etc/nginx/conf.d/api.conf` still pointed at 8081. A check at the end of the deploy script catches this next time:\n\n```bash\ncurl -fsS http://localhost/healthz || exit 1\n```",
      calls: [],
      finish: "stop",
    };
  }

  async function plan(c: Conv, turn: Turn): Promise<Plan> {
    const user = lastUser(c);
    const since = c.entries.slice(user.index + 1);
    const results = since.filter((e): e is ToolEntry => e.role === "tool");
    const callsSoFar = since.reduce((n, e) => n + (e.role === "assistant" ? e.tool_calls.length : 0), 0);
    // AI-10: what is attached to the message is in blocks before the typed text; keywords come from the typed text.
    const { attachments, typed } = parseMessage(user.text);
    const diagnostics = attachments.find((a) => a.kind === "diagnostics");
    const attachment = attachments.find((a) => a.kind === "selection");
    const added = attachments.filter((a) => a.kind === "paste" || a.kind === "file");
    const errorLine = diagnostics?.text.split("\n").find((line) => line.startsWith("Error:")) ?? "";
    const words = typed.toLowerCase();
    const want = (...w: string[]) => w.some((x) => words.includes(x));
    const terminal = turn.context.tab;

    if (flags.has("refused")) return { reasoning: "", text: zh ? "抱歉，我不能帮助完成这个请求。" : "I can’t help with that request.", calls: [], finish: "refused" };
    if (flags.has("length")) {
      const para = zh
        ? "这台主机上运行着多个服务，下面逐一说明它们的配置、日志位置和常见问题的排查方法。"
        : "This host runs several services; here is each one with its configuration, log location and the usual ways to debug it. ";
      return { reasoning: zh ? "需要写一份很长的说明。" : "This needs a long write-up.", text: `## ${zh ? "服务总览" : "Services"}\n\n${para.repeat(14)}`, calls: [], finish: "length" };
    }
    if (flags.has("limit") && terminal && callsSoFar < 40) {
      return {
        reasoning: zh ? "再看一次屏幕，确认部署进度。" : "Check the screen again to follow the deploy.",
        text: callsSoFar === 0 ? (zh ? "我会持续观察屏幕上的部署进度。" : "I’ll keep watching the deploy on the screen.") : "",
        calls: Array.from({ length: 5 }, () => call("read_terminal", { lines: 20 })),
        finish: "tool_calls",
      };
    }

    if (flags.has("showcase") && terminal) return showcase(since.filter((e) => e.role === "assistant").length);

    if (results.length === 0 && callsSoFar === 0) {
      const calls: AiToolCall[] = [];
      // Terminal tools only with a connected tab; a request without one does not offer them.
      const wantsTerminal = want(
        ...["disk", "磁盘", "ディスク", "df", "fail", "失败", "失敗", "sleep", "screen", "屏幕", "画面", "type", "send", "输入", "入力"],
        ...["read file", "读取文件", "ファイルを読", "edit", "编辑", "編集", "write", "写入", "書き込"],
      );
      if (terminal) {
        if (want("disk", "磁盘", "ディスク", "df")) calls.push(call("run_command", { command: "df -h /", timeout_seconds: 30 }));
        if (want("fail", "失败", "失敗")) calls.push(call("run_command", { command: "sudo systemctl restart nonexistent.service" }));
        if (want("sleep")) calls.push(call("run_command", { command: "sleep 8 && echo done", timeout_seconds: 30 }));
        if (want("screen", "屏幕", "画面")) calls.push(call("read_terminal", { lines: 50 }));
        if (want("type", "send", "输入", "入力")) calls.push(call("send_input", { text: "uptime", key: "enter", wait_seconds: 5 }));
        if (want("hidden", "隐藏", "隠し")) calls.push(call("run_command", { command: "cd /srv/app\tmake deploy\nprintf '\x1b[2J'\necho ‮/ fr- mr\n", timeout_seconds: 30 }));
        // AI-38…40: the file tools, on the mock's files.
        if (want("read file", "cat ", "读取文件", "ファイルを読")) calls.push(call("read_file", { path: "/etc/nginx/sites-available/default" }));
        if (want("edit", "编辑", "編集"))
          calls.push(
            call("edit_file", {
              path: "/etc/nginx/sites-available/default",
              old_string: "        proxy_pass http://127.0.0.1:8081/;\n        proxy_read_timeout 30s;",
              new_string: "        proxy_pass http://127.0.0.1:8080/;\n        proxy_read_timeout 60s;\n        proxy_set_header Host $host;",
            }),
          );
        if (want("listen", "端口", "ポート")) calls.push(call("edit_file", { path: "/etc/nginx/sites-available/default", old_string: "80 default_server", new_string: "8080 default_server", replace_all: true }));
        if (want("nomatch", "不匹配", "不一致")) calls.push(call("edit_file", { path: "/etc/nginx/sites-available/default", old_string: "listen 443 ssl;", new_string: "listen 8443 ssl;" }));
        if (want("crlf")) calls.push(call("edit_file", { path: "/srv/app/web.config", old_string: '<add key="mode" value="production" />', new_string: '<add key="mode" value="maintenance" />\n    <add key="banner" value="Back soon" />' }));
        if (want("write", "写入", "書き込"))
          calls.push(call("write_file", { path: "/srv/app/public/maintenance.html", content: "<!doctype html>\n<title>Maintenance</title>\n<h1>We’ll be back soon</h1>\n<p>Scheduled maintenance until 04:00 UTC.</p>\n" }));
        if (want("bigfile"))
          calls.push(call("write_file", { path: "/srv/app/data/seed.csv", content: `id,name\n${Array.from({ length: 1200 }, (_, i) => `${i + 1},host-${i + 1}`).join("\n")}\n` }));
        if (want("motd")) calls.push(call("write_file", { path: "/etc/motd", content: "Welcome to prod-api.\nMaintenance window: Saturdays 01:00–03:00 UTC.\nOn call: #ops\n" }));
      }
      if (want("search", "搜索", "検索")) calls.push(call("web_search", { query: "nginx 502 bad gateway upstream prematurely closed" }));
      if (want("fetch", "url", "网页", "ページ")) calls.push(call("fetch_url", { url: "https://nginx.org/en/docs/http/ngx_http_upstream_module.html" }));
      if (want("mcp")) calls.push(await mcpCall(turn, words));
      if (want("unknown")) calls.push(call("delete_everything", { confirm: true }));
      // Connection diagnostics: read the built-in skill's troubleshooting reference first.
      if (diagnostics && calls.length === 0) calls.push(call("read_skill", { name: "hatoba", path: "references/troubleshooting-connections.md" }));
      if (calls.length > 0)
        return {
          reasoning: zh
            ? "用户想了解主机的状态。先用工具收集信息，再根据结果回答。"
            : "The user wants to know about the host. Gather the facts with tools first, then answer from the results.",
          text: zh ? "我先看一下。" : "Let me check.",
          calls,
          finish: "tool_calls",
        };
      if (wantsTerminal)
        return {
          reasoning: "",
          text: zh
            ? "这个对话没有连接终端标签页，所以我无法运行命令或读取屏幕。请打开一台主机，然后再问我一次。"
            : "No terminal tab is attached to this conversation, so I can’t run commands or read the screen. Open a host and ask me again.",
          calls: [],
          finish: "stop",
        };
    }

    if (results.length > 0 && diagnostics) {
      return {
        reasoning: zh ? "根据诊断信息和故障排查参考给出原因和下一步。" : "Use the diagnostics and the troubleshooting reference to explain the cause and the next steps.",
        text: zh
          ? `诊断信息显示：\`${errorLine}\`。\n\n可能的原因：\n\n1. 主机没有开机，或者 SSH 服务没有运行\n2. 防火墙或安全组挡住了端口\n3. 地址或端口填错了\n\n先确认能 ping 通主机，再检查 **编辑主机** 里的地址和端口。改好之后点 **重试**。`
          : `The diagnostics say \`${errorLine}\`.\n\nLikely causes:\n\n1. The host is down, or its SSH server is not running\n2. A firewall or security group blocks the port\n3. The address or port is wrong\n\nCheck that the host answers at all, then the address and port in **Edit Host**, and choose **Retry** after fixing them.`,
        calls: [],
        finish: "stop",
      };
    }

    if (results.length > 0) {
      const parts = results.map((r) => {
        const head = r.content.split("\n").slice(0, 8).join("\n");
        const label = r.status === "ok" ? "" : ` _(${r.status})_`;
        return `- \`${r.tool_call_id}\`${label}\n\n  \`\`\`\n${head.replace(/^/gm, "  ")}\n  \`\`\``;
      });
      return {
        reasoning: zh ? "根据工具结果整理答案。" : "Summarize what the tools returned.",
        text: zh
          ? `这是我看到的结果：\n\n${parts.join("\n")}\n\n| 项目 | 状态 |\n|---|---|\n| 根分区 | 已用 48%，正常 |\n| 负载 | 0.21，空闲 |\n\n如果还需要我做什么，告诉我。`
          : `Here is what I found:\n\n${parts.join("\n")}\n\n| Item | State |\n|---|---|\n| Root filesystem | 48% used, fine |\n| Load | 0.21, idle |\n\nTell me if you want me to do anything else.`,
        calls: [],
        finish: "stop",
      };
    }

    if (added.length > 0) {
      const list = added.map((a) => `- ${a.kind === "file" ? `\`${a.name}\`` : zh ? "粘贴的文本" : "pasted text"}: ${a.lines} ${zh ? "行" : "line(s)"}, ${[...a.text].length} ${zh ? "个字符" : "characters"}`).join("\n");
      return {
        reasoning: zh ? "用户附上了文本，根据它回答。" : "The user attached text; answer from it.",
        text: zh
          ? `我收到了这些附件：\n\n${list}\n\n内容都完整地到了。告诉我要从里面找什么。`
          : `I received these attachments:\n\n${list}\n\nAll of it arrived whole. Tell me what to look for in it.`,
        calls: [],
        finish: "stop",
      };
    }

    if (attachment) {
      const first = "```\n" + attachment.text.split("\n")[0] + "\n```";
      const cut = attachment.truncated ? (zh ? "（太长，中间部分被省略了）" : " (long, so the middle was left out)") : "";
      return {
        reasoning: zh ? "用户附上了终端里选中的内容，根据它回答。" : "The user attached text selected in the terminal; answer from it.",
        text: zh
          ? `你从 **${attachment.host}** 选中了 ${attachment.lines} 行${cut}。第一行是：\n\n${first}\n\n这看起来是一段正常的终端输出。如果要我进一步检查，告诉我要看什么。`
          : `You selected ${attachment.lines} line(s) on **${attachment.host}**${cut}. The first one is:\n\n${first}\n\nIt looks like ordinary terminal output. Tell me what to look into next.`,
        calls: [],
        finish: "stop",
      };
    }

    return {
      reasoning: zh ? "这是一般性的问题，不需要工具。用 Markdown 简短回答。" : "A general question; no tools needed. Answer briefly in Markdown.",
      text: zh
        ? "我是 Hatoba 的 AI 助手（演示模式）。我可以：\n\n1. **读取屏幕**，解释报错\n2. 运行 `df -h` 之类的**命令**\n3. 在 shell 里**输入**内容\n4. **搜索**和读取网页\n\n```bash\n# 例如\njournalctl -u nginx -n 50 --no-pager\n```\n\n试试问我**磁盘**用量、**屏幕**上的内容、**搜索**某个错误，或者让我**输入**一条命令。文档见 [Hatoba 仓库](https://github.com/scarletkc/Hatoba)。远程图片不会加载：![架构图](https://example.com/diagram.png)\n\n> 原始 HTML 只会显示为文字：<b>不加粗</b>"
        : "I’m Hatoba’s AI assistant (demo mode). I can:\n\n1. **Read the screen** and explain errors\n2. Run **commands** such as `df -h`\n3. **Type** into the shell\n4. **Search** and read the web\n\n```bash\n# for example\njournalctl -u nginx -n 50 --no-pager\n```\n\nTry asking about **disk** usage, what’s on the **screen**, to **search** for an error, or to **type** a command. See the [Hatoba repository](https://github.com/scarletkc/Hatoba). Remote images are never loaded: ![diagram](https://example.com/diagram.png)\n\n> Raw HTML shows as text: <b>not bold</b>",
      calls: [],
      finish: "stop",
    };
  }

  const estimate = (c: Conv) => Math.ceil(c.entries.reduce((n, e) => n + JSON.stringify(e).length, 0) / 4);

  /** The entries from `context_start` on (AI-21). */
  function contextOf(c: Conv): AiEntryView[] {
    const i = c.view.context_start ? c.entries.findIndex((e) => e.entry_id === c.view.context_start) : -1;
    return i < 0 ? c.entries : c.entries.slice(i);
  }

  /** The context's size as the meter counts it: the last usage plus a length estimate for what came after. */
  function contextTokens(c: Conv): number {
    const ctx = contextOf(c);
    for (let i = ctx.length - 1; i >= 0; i--) {
      const e = ctx[i];
      if (e.role === "assistant" && e.usage) return e.usage.input_tokens + e.usage.output_tokens + Math.ceil(JSON.stringify(ctx.slice(i + 1)).length / 4);
    }
    return Math.ceil(JSON.stringify(ctx).length / 4);
  }

  /** AI-05: the level a request of the turn carries: the chosen one, fitted to the levels the model offers. */
  async function sentEffort(turn: Turn): Promise<AiEffort | null> {
    const providers = await deps.providers();
    const model = providers.find((p) => p.id === turn.context.provider_id)?.models.find((m) => m.id === turn.context.model_id);
    return effectiveEffort(turn.context.effort, modelEfforts(model));
  }

  /** AI-22: whether the context, with `extra` tokens of a new message, would pass 90% of the context window. */
  async function compactionDue(c: Conv, turn: Turn, extra = 0): Promise<boolean> {
    if (turn.compacted) return false;
    const providers = await deps.providers();
    const window = providers.find((p) => p.id === turn.context.provider_id)?.models.find((m) => m.id === turn.context.model_id)?.context_window ?? null;
    const ctx = contextOf(c);
    const answered = ctx.some((e) => e.role === "assistant");
    // Like Rust: nothing to compact, or just compacted.
    if (!ctx.some((e) => e.role === "user" || e.role === "assistant") || ctx.at(-1)?.role === "summary") return false;
    return (flags.has("autocompact") && answered) || (!!window && contextTokens(c) + extra > 0.9 * window);
  }

  /**
   * AI-22 before a new message, as Rust does it: the summary is stored before the message (and sent on the
   * turn's channel before `ai_send` returns), so the message stays in the context. False when the turn was
   * stopped meanwhile: the message is then not stored.
   */
  async function compactBefore(c: Conv, turn: Turn, text: string): Promise<boolean> {
    if (!(await compactionDue(c, turn, Math.ceil(text.length / 4)))) return !turn.ended;
    return compactNow(c, turn);
  }

  /** AI-22: before a request that would pass 90% of the context window, compact first. */
  async function autoCompact(c: Conv, turn: Turn): Promise<boolean> {
    if (!(await compactionDue(c, turn))) return true;
    return compactNow(c, turn);
  }

  async function compactNow(c: Conv, turn: Turn): Promise<boolean> {
    turn.compacted = true;
    if (!(await sleep(turn, 1400))) return false;
    const entry: AiEntryView = {
      role: "summary",
      entry_id: newId(),
      created_at: Date.now(),
      text: zh
        ? `**之前的对话摘要（自动压缩）**：共 ${c.entries.length} 条记录。用户和助手讨论了这台主机的状态；用户最新的请求是：“${lastUser(c).text}”`
        : `**Summary of the earlier conversation (compacted automatically)**: ${c.entries.length} entries. The user and the assistant went over this host’s state; the user’s latest request is: “${lastUser(c).text}”`,
    };
    c.view = { ...c.view, context_start: entry.entry_id, updated_at: Date.now() };
    store(c, entry);
    return true;
  }

  async function respond(turn: Turn) {
    const c = convs.get(turn.convId);
    if (!c || turn.ended) return;
    if (!(await autoCompact(c, turn))) return;
    turn.emit({ kind: "request_started" });
    if (!(await sleep(turn, 450))) return;
    if (flags.has("effortfail") && (await sentEffort(turn))) turn.emit({ kind: "effort_ignored" });
    if (flags.has("error") && !errorShown) {
      errorShown = true;
      turn.emit({ kind: "error", status: 529, message: "Overloaded: the service is temporarily overloaded, please try again later." });
      endTurn(turn, "error");
      return;
    }
    const p = await plan(c, turn);
    if (turn.ended) return;
    for (const piece of chunks(p.reasoning)) {
      if (!(await sleep(turn, 22))) return;
      turn.emit({ kind: "reasoning", delta: piece });
    }
    for (const piece of chunks(p.text)) {
      if (!(await sleep(turn, 26))) return;
      turn.emit({ kind: "text", delta: piece });
    }
    for (const x of p.calls) {
      if (!(await sleep(turn, 120))) return;
      turn.emit({ kind: "tool_call", id: x.id, name: x.name, arguments: x.arguments });
    }
    const usage: AiUsage = { input_tokens: estimate(c) + 2400, output_tokens: Math.ceil((p.text.length + p.reasoning.length) / 4) + 20, estimated: false };
    turn.emit({ kind: "usage", ...usage });
    if (!(await sleep(turn, 80))) return;
    const entry: Assistant = {
      role: "assistant",
      entry_id: newId(),
      created_at: Date.now(),
      provider_id: turn.context.provider_id,
      model_id: turn.context.model_id,
      text: p.text,
      reasoning: p.reasoning || null,
      tool_calls: p.calls,
      finish: p.finish,
      usage,
    };
    // Like Rust's `AssistantEntry.terminal` (AI-09), which the view does not carry.
    replyTerminal.set(entry.entry_id, turn.context.tab);
    store(c, entry);
    turn.emit({ kind: "done", finish: p.finish });
    if (p.finish === "tool_calls") return;
    endTurn(turn, p.finish === "stop" ? "completed" : p.finish === "length" ? "length" : "refused");
  }

  /** When the last call of a response has its result, the next request goes out. */
  function maybeContinue(c: Conv) {
    const turn = turns.get(c.view.id);
    if (!turn || turn.ended || unanswered(c).length > 0) return;
    const t = window.setTimeout(() => {
      turn.timers.delete(t);
      void respond(turn);
    }, 250);
    turn.timers.add(t);
  }

  async function checkModel(context: AiTurnContext) {
    const providers = await deps.providers();
    if (!providers.some((p) => p.id === context.provider_id && p.models.some((m) => m.id === context.model_id))) fail("invalid_input", "the model is not configured");
  }

  /**
   * Starts a turn on a new channel. A running turn stops first, as in Rust: its channel gets the
   * cancelled results and `turn_ended`, and the new channel gets the cancelled results too, since
   * the panel stopped listening to the old one.
   */
  function startTurn(c: Conv, context: AiTurnContext, onEvent: (ev: AiTurnEvent) => void, go = true): Turn {
    const cancelled = stopTurn(c.view.id);
    c.runningUntil = 0;
    const turn: Turn = { convId: c.view.id, emit: onEvent, context, ended: false, timers: new Set(), sleepers: new Set(), compacted: false };
    turns.set(c.view.id, turn);
    for (const entry of cancelled) turn.emit({ kind: "entry", entry });
    if (go) begin(turn);
    return turn;
  }

  /** The turn's first request. */
  function begin(turn: Turn) {
    window.setTimeout(() => void respond(turn), 0);
  }

  /**
   * The user's message after a compaction it waited for, unless the turn was stopped meanwhile, with the thinking
   * level and the move its note tells of (like Rust).
   */
  async function storeMessage(c: Conv, turn: Turn, text: string, context: AiTurnContext, move: Move | null): Promise<AiEntryView> {
    if (!(await compactBefore(c, turn, text))) {
      if (turns.get(c.view.id) === turn) turns.delete(c.view.id);
      fail("cancelled", "the message was stopped before it was sent");
    }
    // AI-05: the conversation keeps the level of its last message.
    if (c.view.effort !== context.effort) c.view = { ...c.view, effort: context.effort, updated_at: Date.now() };
    if (move) {
      c.view = { ...c.view, host_id: move.host_id, quick_target: move.quick_target, updated_at: Date.now() };
    }
    const user_entry: AiEntryView = { role: "user", entry_id: newId(), created_at: Date.now(), text };
    c.entries.push(user_entry);
    begin(turn);
    return user_entry;
  }

  /**
   * Like Rust's `notes_before` (AI-05, AI-09): a `host_change` note when the entries before the message
   * came from another host than the one the conversation is on with it (`move`'s, or its own when `move` is
   * `null`; `cameFrom` names the other, `null` without a move), a
   * `terminal_change` note when the message has the terminal tools and `terminalBefore` says there was
   * none, or the other way round, and a `model_change` note when the newest reply came from another
   * model, unless a message without a reply noted that switch already. Returned as the text that goes
   * before the message.
   */
  async function notesBefore(c: Conv, earlier: AiEntryView[], move: Move | null, cameFrom: string | null, context: AiTurnContext): Promise<string> {
    const notes: Note[] = [];
    if (earlier.length === 0) return "";
    const hosts = (await deps.hosts?.()) ?? [];
    const place = move ?? c.view;
    const to = hosts.find((h) => h.id === place.host_id)?.name ?? (place.quick_target ? formatTarget(place.quick_target) : undefined);
    if (cameFrom !== null && to !== undefined && cameFrom !== to) notes.push({ kind: "host_change", from: cameFrom, to });
    if (terminalBefore(earlier) === !context.tab) notes.push({ kind: "terminal_change", to: context.tab ? "attached" : "detached" });
    const providers = await deps.providers();
    const label = (providerId: string, modelId: string) => {
      const name = (providers.find((p) => p.id === providerId)?.models.find((m) => m.id === modelId)?.name ?? "").trim();
      return name && name !== modelId ? `${name} (${modelId})` : modelId;
    };
    const now = label(context.provider_id, context.model_id);
    for (const e of [...earlier].reverse()) {
      if (e.role === "assistant") {
        if (e.model_id !== context.model_id) notes.push({ kind: "model_change", from: label(e.provider_id, e.model_id), to: now });
        break;
      }
      if (e.role === "user" && parseMessage(e.text).notes.some((n) => n.kind === "model_change" && n.to === now)) break;
    }
    return notes.map((n) => `${noteBlock(n)}\n\n`).join("");
  }

  /**
   * Like Rust's `terminal_before` (AI-09): what the newest reply's request offered, or what a
   * `terminal_change` note says on a newer message without a reply; undefined when unknown.
   */
  function terminalBefore(earlier: AiEntryView[]): boolean | undefined {
    for (const e of [...earlier].reverse()) {
      if (e.role === "assistant") return replyTerminal.get(e.entry_id);
      if (e.role !== "user") continue;
      const note = parseMessage(e.text).notes.find((n) => n.kind === "terminal_change");
      if (note?.kind === "terminal_change") return note.to === "attached";
    }
    return undefined;
  }

  /**
   * Like Rust's move in `open_turn` (AI-09), decided by the panel's `moves` and `movedFrom` so the two cannot
   * disagree: the conversation moves to the tab's host, or to its quick-connect target (HOST-12), which it records
   * in place of a host, and stays where it was on the home tab, which sends its own host or target; `null`
   * without a move. `from` is the note's, or `null` when Rust writes none. `storeMessage` stores it with the
   * message.
   */
  async function moveOf(c: Conv, context: AiTurnContext): Promise<Move | null> {
    const tab: TabSource | null = context.host_id ? { hostId: context.host_id, target: null } : context.target ? { hostId: null, target: context.target } : null;
    if (!tab || !moves(c.view, tab)) return null;
    const hosts = (await deps.hosts?.()) ?? [];
    return { from: movedFrom(c.view, tab, hosts)?.from ?? null, host_id: tab.hostId, quick_target: tab.target && recordedTarget(tab.target) };
  }

  /**
   * Like Rust's `names_server` (AI-09): whether a `host_change` note's `from` names the server the conversation is
   * on with `move` (or without it): its target's name, or a saved host of that name with its address, port, and user.
   */
  async function namesServer(c: Conv, move: Move | null, from: string): Promise<boolean> {
    const hosts = (await deps.hosts?.()) ?? [];
    const host = hosts.find((h) => h.id === (move ? move.host_id : c.view.host_id));
    const here = host ? recordedTarget(host) : move ? move.quick_target : c.view.quick_target;
    return !!here && (formatTarget(here) === from || hosts.some((h) => h.name === from && sameTarget(h, here)));
  }

  /** Like Rust: the call must belong to the newest response and have no result yet. */
  function openCall(c: Conv, callId: string): AiToolCall {
    const newest = [...c.entries].reverse().find((e): e is Assistant => e.role === "assistant");
    const x = newest?.tool_calls.find((y) => y.id === callId) ?? fail("not_found", "tool call not found");
    if (answered(c).has(callId)) fail("invalid_input", "the tool call already has a result", { field: "tool_call_id" });
    return x;
  }

  function storeResult(c: Conv, callId: string, status: AiToolStatus, content: string, edited: string | null): ToolEntry {
    const text = edited && (status === "ok" || status === "error") ? `The user edited the arguments before the call ran; it ran with ${edited}\n\n${content}` : content;
    const entry: ToolEntry = { role: "tool", entry_id: newId(), created_at: Date.now(), tool_call_id: callId, status, content: text };
    store(c, entry);
    maybeContinue(c);
    return entry;
  }

  // ───────────── tools that run in "Rust" ─────────────

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
    ["/srv/app/web.config", '<?xml version="1.0"?>\r\n<configuration>\r\n  <appSettings>\r\n    <add key="mode" value="production" />\r\n  </appSettings>\r\n</configuration>\r\n'],
  ]);
  /** What each approval card read, like Rust's: the call writes only over it (AI-39). */
  const fileBases = new Map<string, string | null>();

  const filePath = (p: string) => (p === "~" ? "/home/deploy" : p.startsWith("~/") ? `/home/deploy/${p.slice(2)}` : p.startsWith("/") ? p : `/home/deploy/${p}`);
  const crlfOf = (text: string) => {
    const crlf = text.split("\r\n").length - 1;
    return crlf > text.split("\n").length - 1 - crlf;
  };
  const toCrlf = (text: string) => text.replace(/\r\n/g, "\n").replace(/\n/g, "\r\n");
  const fileLines = (text: string) => (text === "" ? [] : text.replace(/\n$/, "").split("\n").map((l) => l.replace(/\r$/, "")));
  const FILE_DISCONNECTED = "The terminal tab is disconnected, so the tool did not run. Ask the user to reconnect the tab.";

  /** The file after an edit or a write, or the error the model gets, as `hatoba_ai::files` computes them. */
  function fileAfter(name: string, args: Record<string, unknown>, before: string | null): { text: string } | { error: string } {
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
      return { error: `old_string appears ${count} times in ${path}, so nothing was changed. Include more of the surrounding lines to make it unique, or set replace_all to replace every occurrence.` };
    return { text: args.replace_all === true ? before.split(old).join(neu) : before.replace(old, () => neu) };
  }

  function exec(command: string): { status: number; stdout: string; stderr: string } {
    if (/nonexistent|fail/.test(command)) return { status: 5, stdout: "", stderr: "Failed to restart nonexistent.service: Unit nonexistent.service not found." };
    if (/\bdf\b/.test(command))
      return { status: 0, stdout: "Filesystem      Size  Used Avail Use% Mounted on\n/dev/nvme0n1p1   49G   23G   24G  48% /", stderr: "" };
    if (/uptime/.test(command)) return { status: 0, stdout: " 09:41:12 up 41 days,  3:12,  1 user,  load average: 0.21, 0.18, 0.12", stderr: "" };
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
    if (/\bss -ltnp\b/.test(command)) return { status: 0, stdout: `LISTEN 0      4096      127.0.0.1:8080      0.0.0.0:*    users:(("api",pid=48211,fd=9))`, stderr: "" };
    if (/nginx -t/.test(command))
      return { status: 0, stdout: "", stderr: "nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\nnginx: configuration file /etc/nginx/nginx.conf test is successful" };
    return { status: 0, stdout: `(demo output of \`${command}\`)`, stderr: "" };
  }

  async function runTool(c: Conv, x: AiToolCall, sessionId: string | null, edited: string | null): Promise<{ status: AiToolStatus; content: string }> {
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
        const secs = Number(/sleep\s+(\d+)/.exec(command)?.[1] ?? 0);
        await wait(secs ? Math.min(secs, 15) * 1000 : 1100);
        const r = exec(command);
        return { status: "ok", content: `exit status: ${r.status}\n--- stdout ---\n${r.stdout}\n--- stderr ---\n${r.stderr}` };
      }
      case "read_file": {
        if (!sessionId) return { status: "error", content: FILE_DISCONNECTED };
        await wait(500);
        const path = String(args.path ?? "");
        const text = remoteFiles.get(filePath(path));
        if (text === undefined) return { status: "error", content: `${path} does not exist.` };
        const lines = fileLines(text);
        const from = Math.max(1, Number(args.offset ?? 1) || 1);
        const shown = lines.slice(from - 1, args.limit ? from - 1 + Number(args.limit) : undefined);
        const crlf = crlfOf(text) ? ", CRLF line breaks" : "";
        const head = shown.length === lines.length ? `${path}: ${lines.length} lines${crlf}` : `${path}: lines ${from}–${from + shown.length - 1} of ${lines.length}${crlf}`;
        return { status: "ok", content: `${head}\n${shown.map((l, i) => `${String(from + i).padStart(6)}\t${l}`).join("\n")}\n` };
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
        const after = fileAfter(x.name, args, current);
        if ("error" in after) return { status: "error", content: after.error };
        remoteFiles.set(key, after.text);
        const count = fileLines(after.text).length;
        if (x.name === "edit_file") return { status: "ok", content: `Edited ${path}: replaced 1 occurrence of old_string.\n` };
        return { status: "ok", content: current === null ? `Created ${path} (${count} lines).\n` : `Replaced the content of ${path} (${count} lines).\n` };
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
          content: `URL: ${String(args.url ?? "")} (characters 0–1,180 of 1,180, text/html)\n\n# Module ngx_http_upstream_module\n\nThe ngx_http_upstream_module module is used to define groups of servers that can be referenced by the proxy_pass directive.\n\n## keepalive\n\nActivates the cache for connections to upstream servers.`,
        };
      case "read_skill":
        await wait(200);
        if (args.name === "hatoba")
          return {
            status: "ok",
            content: "# Troubleshooting connections (demo excerpt)\n\n## ETIMEDOUT (timeout)\n\nThe host did not answer within 15 seconds: it is off, a firewall drops the packets, or the address or port is wrong.\n\n## ECONNREFUSED (refused)\n\nThe host answered but nothing listens on the port: the SSH server is stopped or listens elsewhere.",
          };
        return { status: "error", content: `There is no enabled skill named "${String(args.name ?? "")}".` };
      default: {
        // An MCP tool (AI-30): the server's text content, or an error result when no running server offers it.
        await wait(700);
        const info = (await deps.mcp?.toolInfo(c.view.id, x.name).catch(() => null)) ?? null;
        if (!info || turn?.context.disabled_mcp_servers.includes(info.server_id)) return { status: "error", content: `No running MCP server offers the tool ${x.name}.` };
        return {
          status: "ok",
          content: `${info.server_name} / ${info.tool.tool} (demo result)\n\n${JSON.stringify({ arguments: args, items: [{ title: "nginx 502 after deploy", state: "open" }, { title: "upstream timeouts", state: "closed" }] }, null, 2)}`,
        };
      }
    }
  }

  // ───────────── demo history ─────────────

  function seed() {
    const now = Date.now();
    const add = (id: string, title: string, host: string | null, ago: number, pinned: boolean, build: (at: (offset: number) => number) => AiEntryView[]) => {
      const created = now - ago;
      const at = (offset: number) => created + offset;
      const c: Conv = {
        // The pinned one was last sent at High (AI-05).
        view: { id, title, host_id: host, quick_target: null, pinned, context_start: null, effort: pinned ? "high" : null, created_at: created, updated_at: created },
        entries: build(at),
        runningUntil: 0,
      };
      convs.set(id, c);
      return c;
    };
    const user = (ms: number, text: string): AiEntryView => ({ role: "user", entry_id: newId(ms), created_at: ms, text });
    const reply = (ms: number, model: [string, string], text: string, opts: Partial<Assistant> = {}): Assistant => {
      const entry: Assistant = {
        role: "assistant",
        entry_id: newId(ms),
        created_at: ms,
        provider_id: model[0],
        model_id: model[1],
        text,
        reasoning: null,
        tool_calls: [],
        finish: "stop",
        usage: { input_tokens: 3200, output_tokens: 240, estimated: false },
        ...opts,
      };
      // Every demo conversation was held in a connected tab of its host.
      replyTerminal.set(entry.entry_id, true);
      return entry;
    };
    const result = (ms: number, callId: string, status: AiToolStatus, content: string): AiEntryView => ({ role: "tool", entry_id: newId(ms), created_at: ms, tool_call_id: callId, status, content });
    const SONNET: [string, string] = ["p-anthropic", "claude-sonnet-5-5"];
    const QWEN: [string, string] = ["p-ollama", "qwen3:8b"];
    const OSS: [string, string] = ["p-ollama", "gpt-oss:20b"];

    add("c-nginx", zh ? "排查 prod-api 上的 nginx 502" : "Debug nginx 502 on prod-api", "h-api-tokyo", 2 * DAY, true, (at) => {
      const logs = call("run_command", { command: "sudo journalctl -u nginx -n 20 --no-pager", timeout_seconds: 30 });
      const screen = call("read_terminal", { lines: 100 });
      return [
        user(at(0), zh ? "网站一直返回 502，帮我看看 nginx" : "The site keeps returning 502. Can you look at nginx?"),
        reply(at(MIN), SONNET, zh ? "我先看 nginx 的日志和当前屏幕。" : "I’ll check the nginx log and the current screen first.", {
          reasoning: zh ? "502 通常是上游服务的问题。先看 nginx 错误日志。" : "A 502 usually means the upstream failed. Start with nginx’s error log.",
          tool_calls: [logs, screen],
          finish: "tool_calls",
        }),
        result(at(2 * MIN), logs.id, "ok", "exit status: 0\n--- stdout ---\nOct 06 09:12:01 prod-api-tokyo nginx[812]: upstream prematurely closed connection while reading response header from upstream, client: 10.0.0.4, upstream: \"http://127.0.0.1:8080/v1/hosts\"\n--- stderr ---\n"),
        result(at(2 * MIN + 1000), screen.id, "ok", "Alternate screen: no. This is the visible screen with 12 line(s) of scrollback above it.\n\ndeploy@prod-api-tokyo:/srv/api$"),
        reply(at(3 * MIN), SONNET, zh
          ? "上游 `api.service`（127.0.0.1:8080）在返回响应头之前关闭了连接。\n\n| 检查项 | 结果 |\n|---|---|\n| nginx | 正常运行 |\n| 上游 | **提前关闭连接** |\n\n建议先看 `journalctl -u api -n 100`，确认服务是否崩溃重启。"
          : "The upstream `api.service` (127.0.0.1:8080) closed the connection before sending headers.\n\n| Check | Result |\n|---|---|\n| nginx | running |\n| upstream | **closed early** |\n\nNext, look at `journalctl -u api -n 100` to see whether the service crashed and restarted.", {
          reasoning: zh ? "日志说明问题在上游。" : "The log points at the upstream.",
          usage: { input_tokens: 5400, output_tokens: 310, estimated: false },
        }),
      ];
    });

    const vacuum = add("c-vacuum", zh ? "Postgres VACUUM 计划" : "Postgres vacuum plan", "h-db-osaka-01", DAY + 3 * HOUR, false, (at) => [
      user(at(0), zh ? "帮我规划一下 db-osaka-01 的 VACUUM" : "Help me plan VACUUM on db-osaka-01"),
      reply(at(MIN), QWEN, zh ? "先确认哪些表膨胀最严重……（很长的分析）" : "First find the most bloated tables… (a long analysis)", { usage: { input_tokens: 21_000, output_tokens: 3_100, estimated: false } }),
      user(at(4 * MIN), zh ? "继续，给出每张表的建议" : "Go on, with advice for each table"),
      reply(at(6 * MIN), QWEN, zh ? "按表给出的建议……（很长）" : "Per-table advice… (long)", { usage: { input_tokens: 29_500, output_tokens: 4_200, estimated: false } }),
    ]);
    const summary: AiEntryView = {
      role: "summary",
      entry_id: newId(vacuum.view.created_at + 8 * MIN),
      created_at: vacuum.view.created_at + 8 * MIN,
      text: zh
        ? "**之前的对话摘要**：用户要为 db-osaka-01 规划 VACUUM。已确认 `events` 和 `sessions` 两张表膨胀最严重，建议在凌晨低峰期对它们运行 `VACUUM (ANALYZE)`，并把 `autovacuum_vacuum_scale_factor` 调到 0.05。"
        : "**Summary of the earlier conversation**: the user is planning VACUUM on db-osaka-01. `events` and `sessions` are the most bloated tables; run `VACUUM (ANALYZE)` on them off-peak and lower `autovacuum_vacuum_scale_factor` to 0.05.",
    };
    vacuum.entries.push(summary);
    vacuum.view.context_start = summary.entry_id;
    const vt = vacuum.view.created_at + 10 * MIN;
    vacuum.entries.push(
      user(vt, zh ? "那 sessions 表现在能直接跑吗？" : "Can I run it on sessions right now?"),
      reply(vt + MIN, QWEN, zh ? "可以，但现在是业务高峰，建议等到 02:00 以后。" : "You can, but it’s peak time now; wait until after 02:00.", {
        usage: { input_tokens: 32_800, output_tokens: 160, estimated: false },
      }),
    );

    add("c-cleanup", zh ? "清理 staging 磁盘空间" : "Clean up disk on staging", "h-staging-worker", 3 * HOUR, false, (at) => {
      const rm = call("run_command", { command: "sudo rm -rf /var/log/journal/*", timeout_seconds: 60 });
      const vacuumLogs = call("run_command", { command: "sudo journalctl --vacuum-size=200M" });
      const type = call("send_input", { text: "docker system prune -af", key: "enter", wait_seconds: 30 });
      const fetch = call("fetch_url", { url: "https://docs.docker.com/engine/manage-resources/pruning/" });
      const mcp = call("mcp__github__create_issue", { repo: "scarletkc/infra", title: "staging disk full" });
      return [
        user(at(0), zh ? "staging-worker-01 的磁盘快满了" : "staging-worker-01 is nearly out of disk"),
        reply(at(MIN), OSS, zh ? "我来清理日志和 Docker 缓存。" : "I’ll clean up logs and the Docker cache.", {
          tool_calls: [rm, vacuumLogs, type, fetch, mcp],
          finish: "tool_calls",
          usage: null,
        }),
        result(at(2 * MIN), rm.id, "rejected", "don't delete the journal by hand"),
        result(at(3 * MIN), vacuumLogs.id, "ok", "The user edited the arguments before the call ran; it ran with {\"command\":\"sudo journalctl --vacuum-size=500M\"}\n\nexit status: 0\n--- stdout ---\nVacuuming done, freed 1.2G of archived journals from /var/log/journal.\n--- stderr ---\n"),
        result(at(4 * MIN), type.id, "error", "The terminal tab disconnected while waiting. Output before that:\n\ndocker system prune -af\nDeleted Containers:"),
        result(at(5 * MIN), fetch.id, "cancelled", "The call was cancelled before it ran."),
        result(at(5 * MIN), mcp.id, "error", "No running MCP server offers the tool mcp__github__create_issue."),
        reply(at(6 * MIN), OSS, zh ? "日志已压缩到 500M，释放了 1.2G。Docker 清理在终端断开时中断了，重新连接后可以再试。" : "The journal is down to 500M, which freed 1.2G. The Docker prune stopped when the terminal disconnected; try again after reconnecting.", {
          usage: { input_tokens: 2100, output_tokens: 90, estimated: true },
        }),
      ];
    });

    // On a quick connection (HOST-12), which the conversation records by its target (AI-09).
    const pi = add("c-pi", zh ? "树莓派的温度" : "Raspberry Pi temperature", null, 5 * HOUR, false, (at) => [
      user(at(0), zh ? "这台树莓派现在多热？" : "How hot is this Pi right now?"),
      reply(at(MIN), OSS, zh ? "`vcgencmd measure_temp` 显示 52.1°C，在正常范围内。" : "`vcgencmd measure_temp` reads 52.1°C, which is within the normal range.", {
        usage: { input_tokens: 1800, output_tokens: 60, estimated: false },
      }),
    ]);
    pi.view.quick_target = { username: "pi", address: "raspberrypi.local", port: 22 };

    const deploy = add("c-deploy", zh ? "盯一下 prod-api 的部署" : "Watch the prod-api deploy", "h-api-tokyo", 20 * MIN, false, (at) => [
      user(at(0), zh ? "部署跑完之后告诉我结果" : "Tell me how the deploy went when it finishes"),
    ]);
    if (flags.has("running")) deploy.runningUntil = now + 20_000;
  }

  if (flags.has("history") || flags.has("running")) seed();

  /** `?ai=running`: the turn an earlier page started finishes. */
  function settleRemote(c: Conv) {
    if (!c.runningUntil || Date.now() < c.runningUntil) return;
    c.runningUntil = 0;
    const entry: Assistant = {
      role: "assistant",
      entry_id: newId(),
      created_at: Date.now(),
      provider_id: "p-anthropic",
      model_id: "claude-sonnet-5-5",
      text: zh ? "部署完成：3 个实例都已健康，`/healthz` 返回 200。" : "The deploy finished: all 3 instances are healthy and `/healthz` returns 200.",
      reasoning: null,
      tool_calls: [],
      finish: "stop",
      usage: { input_tokens: 4100, output_tokens: 40, estimated: false },
    };
    // The earlier page ran the turn in a connected tab.
    replyTerminal.set(entry.entry_id, true);
    c.entries.push(entry);
  }

  return {
    ai_conversations_list: async () => {
      await delay(150);
      return [...convs.values()].map(view);
    },
    ai_conversation_get: async (id) => {
      await delay(120);
      const c = need(id);
      settleRemote(c);
      return { conversation: view(c), entries: c.entries.map((e) => ({ ...e })), running: turns.has(id) || c.runningUntil > Date.now() };
    },
    ai_conversation_rename: async (id, title) => {
      const c = need(id);
      c.view = { ...c.view, title: title.slice(0, 200), updated_at: Date.now() };
      return view(c);
    },
    ai_conversation_pin: async (id, pinned) => {
      const c = need(id);
      c.view = { ...c.view, pinned, updated_at: Date.now() };
      return view(c);
    },
    ai_conversation_delete: async (id) => {
      stopTurn(id);
      convs.delete(id);
    },
    ai_send: async (input, onEvent) => {
      await delay(120);
      if (!input.text.trim()) fail("invalid_input", "the message is empty", { field: "text" });
      await checkModel(input.context);
      let c: Conv;
      if (input.conversation_id) {
        c = need(input.conversation_id);
      } else {
        const id = newId();
        c = {
          view: {
            id,
            title: titleOf(input.text),
            host_id: input.context.host_id,
            quick_target: !input.context.host_id && input.context.target ? recordedTarget(input.context.target) : null,
            pinned: false,
            context_start: null,
            effort: input.context.effort,
            created_at: Date.now(),
            updated_at: Date.now(),
          },
          entries: [],
          runningUntil: 0,
        };
      }
      const turn = startTurn(c, input.context, onEvent, false);
      convs.set(c.view.id, c);
      const earlier = [...c.entries];
      const move = await moveOf(c, input.context);
      const notes = input.conversation_id ? await notesBefore(c, earlier, move, move?.from ?? null, input.context) : "";
      const user_entry = await storeMessage(c, turn, notes + input.text, input.context, move);
      return { conversation: view(c), user_entry };
    },
    ai_retry: async (id, context, onEvent) => {
      await delay(80);
      const c = need(id);
      await checkModel(context);
      // Like Rust: a conversation that ends with the model's answer has nothing to retry.
      const last = [...contextOf(c)].reverse().find((e) => e.role !== "tool");
      if (!last || (last.role === "assistant" && last.tool_calls.length === 0)) fail("invalid_input", "nothing to retry: the conversation ends with the model's answer", { field: "conversation_id" });
      if (c.view.effort !== context.effort) c.view = { ...c.view, effort: context.effort, updated_at: Date.now() };
      startTurn(c, context, onEvent);
    },
    ai_tool_result: async (id, callId, result: AiToolResultInput) => {
      const c = need(id);
      openCall(c, callId);
      // Like Rust: a rejection stores the user's reason; the adapter tells the model the call was rejected.
      const content = result.status === "rejected" ? result.content.trim() : result.content;
      fileBases.delete(callId);
      return storeResult(c, callId, result.status, content, result.edited_arguments);
    },
    ai_tool_run: async (id, callId, sessionId, edited) => {
      const c = need(id);
      const x = openCall(c, callId);
      const r = await runTool(c, x, sessionId, edited);
      // A stop stored a cancelled result while the tool ran.
      if (answered(c).has(callId)) fail("cancelled", "the tool call was stopped");
      return storeResult(c, callId, r.status, r.content, edited);
    },
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
      const after = fileAfter(x.name, args, before);
      if ("error" in after) return failed(after.error);
      const crlf = before !== null && crlfOf(before);
      const shown = (text: string) => (crlf ? text.replace(/\r\n/g, "\n") : text);
      return { path, before: before === null ? null : shown(before), after: shown(after.text), error: null, crlf };
    },
    ai_stop: async (id) => {
      const c = convs.get(id);
      if (c?.runningUntil) c.runningUntil = 0;
      stopTurn(id);
    },
    ai_compact: async (id) => {
      const c = need(id);
      if (turns.has(id)) fail("invalid_input", "a turn is running");
      await delay(1500);
      const entry: AiEntryView = {
        role: "summary",
        entry_id: newId(),
        created_at: Date.now(),
        text: zh
          ? `**之前的对话摘要**：共 ${c.entries.length} 条记录。用户和助手讨论了这台主机的状态，并完成了相关检查。`
          : `**Summary of the earlier conversation**: ${c.entries.length} entries. The user and the assistant went over this host’s state and finished the checks.`,
      };
      c.entries.push(entry);
      c.view = { ...c.view, context_start: entry.entry_id, updated_at: Date.now() };
      return entry;
    },
    ai_search: async (query) => {
      await delay(200);
      const q = query.trim().toLowerCase();
      if (!q) return [];
      const textOf = (e: AiEntryView) => (e.role === "user" || e.role === "assistant" || e.role === "summary" ? e.text : "");
      const hits: AiSearchHit[] = [];
      for (const c of [...convs.values()].sort((a, b) => view(b).last_activity - view(a).last_activity)) {
        const entry = c.entries.find((e) => textOf(e).toLowerCase().includes(q));
        if (entry) hits.push({ conversation_id: c.view.id, entry_id: entry.entry_id, snippet: snippetAround(textOf(entry), q) });
        else if (c.view.title.toLowerCase().includes(q)) hits.push({ conversation_id: c.view.id, entry_id: null, snippet: c.view.title });
      }
      return hits;
    },
    ai_edit_resend: async (id, entryId, text, context, onEvent) => {
      await delay(120);
      const c = need(id);
      const i = c.entries.findIndex((e) => e.entry_id === entryId);
      if (i < 0) fail("not_found", "entry not found");
      if (c.entries[i].role !== "user") fail("invalid_input", "only the user's messages can be edited", { field: "entry_id" });
      if (!text.trim()) fail("invalid_input", "the message is empty", { field: "text" });
      await checkModel(context);
      // The stopped turn's cancelled results go with everything else after the message.
      stopTurn(id);
      // Like Rust: the earliest move the deleted messages noted still tells where the earlier screens came from.
      const moved = c.entries
        .slice(i)
        .flatMap((e) => (e.role === "user" ? parseMessage(e.text).notes : []))
        .find((n) => n.kind === "host_change");
      let cameFrom: string | null = moved?.kind === "host_change" ? moved.from : null;
      c.entries = c.entries.slice(0, i);
      if (c.view.context_start && !c.entries.some((e) => e.entry_id === c.view.context_start)) {
        const summary = [...c.entries].reverse().find((e) => e.role === "summary");
        c.view = { ...c.view, context_start: summary?.entry_id ?? null };
      }
      c.view = { ...c.view, updated_at: Date.now() };
      const turn = startTurn(c, context, onEvent, false);
      const move = await moveOf(c, context);
      // Like Rust: the kept move is dropped when its note names the server the message goes from under another name.
      if (cameFrom !== null && (await namesServer(c, move, cameFrom))) cameFrom = null;
      else if (cameFrom === null) cameFrom = move?.from ?? null;
      const notes = await notesBefore(c, [...c.entries], move, cameFrom, context);
      const user_entry = await storeMessage(c, turn, notes + text, context, move);
      return { conversation: view(c), user_entry };
    },
    // The browser reads dropped files itself (File API); there are no paths to read here.
    ai_read_dropped_files: async (paths) => paths.map((p) => ({ status: "refused", name: p.split(/[\\/]/).pop() ?? p, reason: "unreadable" })),
  };
}

/**
 * Like Rust's `title_of` (AI-23): the first non-empty line typed after the attachment blocks
 * (AI-10), or the first block's kind when nothing was typed, cut to 60 characters.
 */
function titleOf(text: string): string {
  const { attachments, typed } = parseMessage(text);
  const block = attachments[0];
  const fallback = !block
    ? ""
    : block.kind === "diagnostics"
      ? "Connection diagnostics"
      : block.kind === "selection"
        ? "Terminal selection"
        : block.kind === "paste"
          ? "Pasted text"
          : block.name;
  const line = typed.split(/\r?\n/).find((l) => l.trim())?.trim() ?? fallback;
  return [...line].slice(0, 60).join("");
}

/** About 40 characters before the first match of `q` (lowercase) in `text` and 100 after, on one line. */
function snippetAround(text: string, q: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  const at = flat.toLowerCase().indexOf(q);
  const start = Math.max(0, at - 40);
  const end = Math.min(flat.length, Math.max(at, 0) + q.length + 100);
  return `${start > 0 ? "…" : ""}${flat.slice(start, end)}${end < flat.length ? "…" : ""}`;
}
