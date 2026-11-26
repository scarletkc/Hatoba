import type { TabSource } from "@/app/tabs";
import { noteBlock, notedPermission, parseMessage, screenStateNote, type Note } from "@/lib/ai/attachments";
import { effectiveEffort, modelEfforts } from "@/lib/ai/effort";
import { localName, movedFrom, moves, recordedTarget } from "@/lib/ai/place";
import { formatTarget, sameTarget } from "@/session/quickConnect";
import type {
  AiEffort,
  AiEntryView,
  AiToolCall,
  AiToolStatus,
  AiTurnContext,
  AiTurnEndReason,
  AiTurnEvent,
  AiUsage,
} from "../types";
import {
  chunks,
  lastUser,
  type AiMockCtx,
  type Assistant,
  type Conv,
  type Move,
  type Plan,
  type Recorded,
  type ToolEntry,
  type Turn,
} from "./aiShared";

export interface AiTurnsDeps {
  plan(c: Conv, turn: Turn): Promise<Plan>;
  pendingReverts(convId: string, earlier: AiEntryView[]): Note[];
}

export function createAiTurns(ctx: AiMockCtx, { plan, pendingReverts }: AiTurnsDeps) {
  const {
    deps,
    flags,
    sayZh,
    convs,
    turns,
    compacting,
    replyTerminal,
    replyShell,
    replyServers,
    cameFrom,
    interrupted,
    verdicts,
    newId,
    fail,
    unanswered,
    store,
    sleep,
  } = ctx;
  let errorShown = false;

  /** §13.1 step 5: every call without a result gets a cancelled one. Returns the results it stored. */
  function cancelOpenCalls(c: Conv): AiEntryView[] {
    return unanswered(c).map((call) => {
      const entry: AiEntryView = {
        role: "tool",
        entry_id: newId(),
        created_at: Date.now(),
        tool_call_id: call.id,
        status: "cancelled",
        content: "The call was cancelled before it ran.",
        auto: null,
      };
      verdicts.delete(call.id);
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

  /**
   * Like Rust's stop: what a response being streamed has shown is stored as a stopped reply, and it,
   * the cancelled results and `turn_ended { stopped }` go to the running turn's channel. Returns the
   * entries it stored.
   */
  function stopTurn(convId: string): AiEntryView[] {
    const turn = turns.get(convId);
    const c = convs.get(convId);
    if (turn) turn.timers.forEach((t) => clearTimeout(t));
    const stored: AiEntryView[] = [];
    const part = turn?.streamed;
    if (turn) turn.streamed = null;
    if (c && part && (part.text.trim() || part.calls.length > 0)) {
      const entry: Assistant = {
        role: "assistant",
        entry_id: newId(),
        created_at: Date.now(),
        provider_id: part.provider_id,
        model_id: part.model_id,
        text: part.text,
        reasoning: null,
        tool_calls: part.calls,
        finish: part.calls.length > 0 ? "tool_calls" : "stop",
        usage: null,
      };
      replyTerminal.set(entry.entry_id, turn.context.tab);
      recordShell(entry.entry_id, turn.context);
      if (part.servers) replyServers.set(entry.entry_id, part.servers);
      interrupted.add(entry.entry_id);
      store(c, entry);
      stored.push(entry);
    }
    if (c) stored.push(...cancelOpenCalls(c));
    if (turn) endTurn(turn, "stopped");
    return stored;
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
      if (e.role === "assistant" && e.usage)
        return e.usage.input_tokens + e.usage.output_tokens + Math.ceil(JSON.stringify(ctx.slice(i + 1)).length / 4);
    }
    return Math.ceil(JSON.stringify(ctx).length / 4);
  }

  /** AI-05: the level a request of the turn carries: the chosen one, fitted to the levels the model offers. */
  async function sentEffort(turn: Turn): Promise<AiEffort | null> {
    const providers = await deps.providers();
    const model = providers
      .find((p) => p.id === turn.context.provider_id)
      ?.models.find((m) => m.id === turn.context.model_id);
    return effectiveEffort(turn.context.effort, modelEfforts(model));
  }

  /** AI-22: whether the context, with `extra` tokens of a new message, would pass 90% of the context window. */
  async function compactionDue(c: Conv, turn: Turn, extra = 0): Promise<boolean> {
    if (turn.compacted) return false;
    const providers = await deps.providers();
    const window =
      providers.find((p) => p.id === turn.context.provider_id)?.models.find((m) => m.id === turn.context.model_id)
        ?.context_window ?? null;
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

  /** Like Rust's `store_summary` (AI-16): a summary starts with the note of the mode the model was told of. */
  function summaryText(c: Conv, summary: string): string {
    const mode = notedPermission(c.entries);
    return mode ? `${noteBlock({ kind: "permission_mode", mode })}\n\n${summary}` : summary;
  }

  async function compactNow(c: Conv, turn: Turn): Promise<boolean> {
    turn.compacted = true;
    compacting.add(c.view.id);
    const slept = await sleep(turn, 1400);
    compacting.delete(c.view.id);
    if (!slept) return false;
    const entry: AiEntryView = {
      role: "summary",
      entry_id: newId(),
      created_at: Date.now(),
      text: summaryText(
        c,
        sayZh(
          `**Summary of the earlier conversation (compacted automatically)**: ${c.entries.length} entries. The user and the assistant went over this host’s state; the user’s latest request is: “${lastUser(c).text}”`,
          `**之前的对话摘要（自动压缩）**：共 ${c.entries.length} 条记录。用户和助手讨论了这台主机的状态；用户最新的请求是：“${lastUser(c).text}”`,
          `**之前的對話摘要（自動壓縮）**：共 ${c.entries.length} 條記錄。使用者和助理討論了這台主機的狀態；使用者最新的請求是：「${lastUser(c).text}」`,
        ),
      ),
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
      turn.emit({
        kind: "error",
        status: 529,
        message: "Overloaded: the service is temporarily overloaded, please try again later.",
      });
      endTurn(turn, "error");
      return;
    }
    const p = await plan(c, turn);
    const servers = deferredIn(c.entries) ? null : await offeredServers(turn.context);
    if (turn.ended) return;
    const part = (turn.streamed = {
      provider_id: turn.context.provider_id,
      model_id: turn.context.model_id,
      text: "",
      calls: [] as AiToolCall[],
      servers,
    });
    // Like Rust, a thinking block says the model thinks before any reasoning text comes.
    const silent = flags.has("updates");
    const reasoning = silent ? "" : p.reasoning;
    if (silent || reasoning) turn.emit({ kind: "thinking" });
    if (silent && !(await sleep(turn, 3000))) return;
    for (const piece of chunks(reasoning)) {
      if (!(await sleep(turn, 22))) return;
      turn.emit({ kind: "reasoning", delta: piece });
    }
    for (const piece of chunks(p.text)) {
      if (!(await sleep(turn, 26))) return;
      part.text += piece;
      turn.emit({ kind: "text", delta: piece });
    }
    for (const x of p.calls) {
      if (!(await sleep(turn, 120))) return;
      part.calls.push(x);
      turn.emit({ kind: "tool_call", id: x.id, name: x.name, arguments: x.arguments });
    }
    const usage: AiUsage = {
      input_tokens: estimate(c) + 2400,
      output_tokens: Math.ceil((p.text.length + reasoning.length) / 4) + 20,
      estimated: false,
    };
    turn.emit({ kind: "usage", ...usage });
    if (!(await sleep(turn, 80))) return;
    const entry: Assistant = {
      role: "assistant",
      entry_id: newId(),
      created_at: Date.now(),
      provider_id: turn.context.provider_id,
      model_id: turn.context.model_id,
      text: p.text,
      reasoning: reasoning || null,
      tool_calls: p.calls,
      finish: p.finish,
      usage,
    };
    // Like Rust's `AssistantEntry.terminal` (AI-09), `shell` (§7.5) and `mcp_servers` (AI-30), which the view does not carry.
    replyTerminal.set(entry.entry_id, turn.context.tab);
    recordShell(entry.entry_id, turn.context);
    if (servers) replyServers.set(entry.entry_id, servers);
    turn.streamed = null;
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
    if (!providers.some((p) => p.id === context.provider_id && p.models.some((m) => m.id === context.model_id)))
      fail("invalid_input", "the model is not configured");
  }

  /**
   * Starts a turn on a new channel. A running turn stops first, as in Rust: its channel gets the
   * cancelled results and `turn_ended`, and the new channel gets the cancelled results too, since
   * the panel stopped listening to the old one.
   */
  function startTurn(c: Conv, context: AiTurnContext, onEvent: (ev: AiTurnEvent) => void, go = true): Turn {
    const cancelled = stopTurn(c.view.id);
    c.runningUntil = 0;
    const turn: Turn = {
      convId: c.view.id,
      emit: onEvent,
      context,
      ended: false,
      timers: new Set(),
      sleepers: new Set(),
      compacted: false,
      streamed: null,
    };
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
   * level, the move its note tells of, and where the entries before it came from, `recorded` (like Rust).
   */
  async function storeMessage(
    c: Conv,
    turn: Turn,
    text: string,
    context: AiTurnContext,
    move: Move | null,
    recorded: Recorded | null,
  ): Promise<AiEntryView> {
    if (!(await compactBefore(c, turn, text))) {
      if (turns.get(c.view.id) === turn) turns.delete(c.view.id);
      fail("cancelled", "the message was stopped before it was sent");
    }
    // AI-05: the conversation keeps the level of its last message.
    if (c.view.effort !== context.effort) c.view = { ...c.view, effort: context.effort, updated_at: Date.now() };
    if (move) {
      c.view = {
        ...c.view,
        host_id: move.host_id,
        quick_target: move.quick_target,
        local: move.local,
        origin: null,
        updated_at: Date.now(),
      };
    }
    const user_entry: AiEntryView = { role: "user", entry_id: newId(), created_at: Date.now(), text };
    if (recorded) cameFrom.set(user_entry.entry_id, recorded);
    c.entries.push(user_entry);
    begin(turn);
    return user_entry;
  }

  /**
   * Like Rust's `notes_before` (§13.1, AI-05, AI-09): an `interrupted` note when the newest reply is one
   * the user stopped, unless a message without a reply noted it already, a `host_change` note when the entries before the message
   * came from another host than the one the conversation is on with it (`move`'s, or its own when `move` is
   * `null`; `cameFrom` names the other, `null` without a move), a
   * `terminal_change` note when the message has the terminal tools and `terminalBefore` says there was
   * none, or the other way round, a `shell_change` note when it has a local tab's tools on another shell than
   * `shellBefore` says and notes no move, and a `model_change` note when the newest reply came from another
   * model, unless a message without a reply noted that switch already, then the screen (`screenNote`), also
   * before a conversation's first message. Returned as the text that goes before the message.
   */
  async function notesBefore(
    c: Conv,
    earlier: AiEntryView[],
    move: Move | null,
    cameFrom: string | null,
    context: AiTurnContext,
  ): Promise<string> {
    const notes: Note[] = [];
    const screen = await screenNote(c, move, context);
    // Like Rust (AI-16): the mode, unless it is the one the model was last told of; the first message always names it.
    const permission: Note[] =
      notedPermission(earlier) === context.permission_mode
        ? []
        : [{ kind: "permission_mode", mode: context.permission_mode }];
    if (earlier.length === 0) return permission.map((n) => `${noteBlock(n)}\n\n`).join("") + screen;
    for (const e of [...earlier].reverse()) {
      if (e.role === "assistant") {
        if (interrupted.has(e.entry_id)) notes.push({ kind: "interrupted" });
        break;
      }
      if (
        e.role === "summary" ||
        (e.role === "user" && parseMessage(e.text).notes.some((n) => n.kind === "interrupted"))
      )
        break;
    }
    const hosts = (await deps.hosts?.()) ?? [];
    const place = move ?? c.view;
    const to =
      hosts.find((h) => h.id === place.host_id)?.name ??
      (place.quick_target ? formatTarget(place.quick_target) : place.local ? localName(place.local) : undefined);
    const moved = cameFrom !== null && to !== undefined && cameFrom !== to;
    if (moved) notes.push({ kind: "host_change", from: cameFrom, to });
    if (terminalBefore(earlier) === !context.tab)
      notes.push({ kind: "terminal_change", to: context.tab ? "attached" : "detached" });
    const shellFrom = shellBefore(earlier);
    const shellTo = tabShell(context);
    if (!moved && shellFrom && shellTo && shellFrom !== shellTo)
      notes.push({ kind: "shell_change", from: shellFrom, to: shellTo });
    const providers = await deps.providers();
    const label = (providerId: string, modelId: string) => {
      const name = (
        providers.find((p) => p.id === providerId)?.models.find((m) => m.id === modelId)?.name ?? ""
      ).trim();
      return name && name !== modelId ? `${name} (${modelId})` : modelId;
    };
    const now = label(context.provider_id, context.model_id);
    for (const e of [...earlier].reverse()) {
      if (e.role === "assistant") {
        if (e.model_id !== context.model_id)
          notes.push({ kind: "model_change", from: label(e.provider_id, e.model_id), to: now });
        break;
      }
      if (e.role === "user" && parseMessage(e.text).notes.some((n) => n.kind === "model_change" && n.to === now)) break;
    }
    notes.push(...permission, ...pendingReverts(c.view.id, earlier));
    return notes.map((n) => `${noteBlock(n)}\n\n`).join("") + screen;
  }

  /**
   * Like Rust's `screen_note` (AI-46): the `terminal_state` note of a message the panel sent with the screen
   * of a connected tab, while **Send Terminal Screen** is on, on the host the conversation is on with `move`.
   */
  async function screenNote(c: Conv, move: Move | null, context: AiTurnContext): Promise<string> {
    if (!context.tab || !context.screen) return "";
    const hosts = (await deps.hosts?.()) ?? [];
    const place = move ?? c.view;
    const host =
      hosts.find((h) => h.id === place.host_id)?.name ??
      (place.quick_target ? formatTarget(place.quick_target) : place.local ? localName(place.local) : "");
    const note = screenStateNote(host, context.screen);
    return note ? `${noteBlock(note)}\n\n` : "";
  }

  /**
   * AI-45, for the demo: a message with "deferred" in it lists the tools of the running MCP servers in an
   * `mcp_tools` note, as Rust does once a conversation's MCP tools take a tenth of the context window.
   */
  async function deferredList(context: AiTurnContext, text: string): Promise<string> {
    if (!text.toLowerCase().includes("deferred") || !deps.mcp) return "";
    const lines: string[] = [];
    let tools = 0;
    for (const s of (await deps.mcp.servers()).filter(
      (x) => x.enabled && !context.disabled_mcp_servers.includes(x.id),
    )) {
      const status = await deps.mcp.status(s.id);
      if (status.state !== "running" || status.tools.length === 0) continue;
      tools += status.tools.length;
      lines.push(
        `- ${s.name} (${status.tools.length === 1 ? "1 tool" : `${status.tools.length} tools`}): ${status.tools.map((x) => x.name).join(", ")}`,
      );
    }
    const body =
      tools === 0
        ? "No MCP tools are available from this message on."
        : `The MCP tools you can call with call_mcp_tool from this message on, by server. This list replaces any earlier one. search_tools returns a tool's description and input schema, and finds the tools of a server that is listed by count only.\n${lines.join("\n")}`;
    return `${noteBlock({ kind: "mcp_tools", servers: lines.length, tools, body })}\n\n`;
  }

  /** Like Rust's `mcp_deferred` (AI-45): a conversation is deferred once a message listed its MCP tools. */
  function deferredIn(entries: AiEntryView[]): boolean {
    return entries.some((e) => e.role === "user" && parseMessage(e.text).notes.some((n) => n.kind === "mcp_tools"));
  }

  /** Like Rust's `Offer::server_names` (AI-30): the running servers whose tools the requests offer, by name. */
  async function offeredServers(context: AiTurnContext): Promise<string[]> {
    if (!deps.mcp) return [];
    const names: string[] = [];
    const servers = (await deps.mcp.servers())
      .filter((x) => x.enabled && !context.disabled_mcp_servers.includes(x.id))
      .sort((a, b) => a.name.localeCompare(b.name));
    for (const s of servers) {
      const status = await deps.mcp.status(s.id);
      if (status.state === "running" && status.tools.length > 0 && !names.includes(s.name)) names.push(s.name);
    }
    return names;
  }

  /**
   * Like Rust's `mcp_change_note` (AI-30): an `mcp_change` note when the message's requests offer the MCP tools
   * of other servers than the model was last told of, unless the conversation is deferred or `listed` lists its
   * tools.
   */
  async function mcpChange(earlier: AiEntryView[], context: AiTurnContext, listed: string): Promise<string> {
    if (listed || deferredIn(earlier)) return "";
    const before = mcpServersBefore(earlier);
    if (!before) return "";
    const now = await offeredServers(context);
    const available = now.filter((n) => !before.includes(n));
    const unavailable = before.filter((n) => !now.includes(n));
    if (available.length === 0 && unavailable.length === 0) return "";
    return `${noteBlock({ kind: "mcp_change", available, unavailable })}\n\n`;
  }

  /**
   * Like Rust's `mcp_servers_before` (AI-30): the servers the newest reply's request offered, as the
   * `mcp_change` notes after it changed them; undefined before the first reply or when unknown.
   */
  function mcpServersBefore(earlier: AiEntryView[]): string[] | undefined {
    const notes: { available: string[]; unavailable: string[] }[] = [];
    for (const e of [...earlier].reverse()) {
      if (e.role === "assistant") {
        const recorded = replyServers.get(e.entry_id);
        if (!recorded) return undefined;
        return notes
          .reverse()
          .reduce(
            (servers, n) => [
              ...servers.filter((x) => !n.unavailable.includes(x)),
              ...n.available.filter((x) => !servers.includes(x)),
            ],
            recorded,
          );
      }
      if (e.role !== "user") continue;
      const note = parseMessage(e.text).notes.find((n) => n.kind === "mcp_change");
      if (note?.kind === "mcp_change") notes.push(note);
    }
    return undefined;
  }

  /** Like Rust's `tab_shell` (§7.5): the shell of a local tab whose terminal tools the request offers. */
  function tabShell(context: AiTurnContext): string | null {
    if (!context.tab || context.host_id || context.target) return null;
    return context.local?.shell ?? null;
  }

  function recordShell(entryId: string, context: AiTurnContext) {
    const shell = tabShell(context);
    if (shell) replyShell.set(entryId, shell);
  }

  /**
   * Like Rust's `shell_before` (§7.5): the shell of the newest reply that had the terminal tools, or what a
   * `shell_change` note says on a newer message without a reply; null after a `host_change` note or a summary.
   */
  function shellBefore(earlier: AiEntryView[]): string | null {
    for (const e of [...earlier].reverse()) {
      if (e.role === "assistant") {
        const terminal = replyTerminal.get(e.entry_id);
        if (terminal === undefined) return null;
        if (terminal) return replyShell.get(e.entry_id) ?? null;
        continue;
      }
      if (e.role === "summary") return null;
      if (e.role !== "user") continue;
      const notes = parseMessage(e.text).notes;
      const shell = notes.find((n) => n.kind === "shell_change");
      if (shell?.kind === "shell_change") return shell.to;
      if (notes.some((n) => n.kind === "host_change")) return null;
    }
    return null;
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
    const local = context.local;
    const tab: TabSource | null = context.host_id
      ? { hostId: context.host_id, target: null }
      : context.target
        ? { hostId: null, target: context.target }
        : local
          ? { hostId: null, target: null, local: { id: "", name: local.shell ?? "", kind: local.kind ?? "unix" } }
          : null;
    if (!tab || !moves(c.view, tab, local?.device)) return null;
    const hosts = (await deps.hosts?.()) ?? [];
    return {
      from: movedFrom(c.view, tab, hosts, local?.device)?.from ?? null,
      host_id: tab.hostId,
      quick_target: tab.target && recordedTarget(tab.target),
      local: tab.local && local ? local.device : null,
    };
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

  /** `judged`: Auto mode decided the call (AI-47), so the result keeps the judge's verdict; otherwise it is dropped. */
  function storeResult(
    c: Conv,
    callId: string,
    status: AiToolStatus,
    content: string,
    edited: string | null,
    judged: boolean,
  ): ToolEntry {
    const text =
      edited && (status === "ok" || status === "error")
        ? `The user edited the arguments before the call ran; it ran with ${edited}\n\n${content}`
        : content;
    const auto = judged ? (verdicts.get(callId) ?? null) : null;
    verdicts.delete(callId);
    const entry: ToolEntry = {
      role: "tool",
      entry_id: newId(),
      created_at: Date.now(),
      tool_call_id: callId,
      status,
      content: text,
      auto,
    };
    store(c, entry);
    maybeContinue(c);
    return entry;
  }

  return {
    stopTurn,
    contextOf,
    summaryText,
    checkModel,
    startTurn,
    storeMessage,
    notesBefore,
    deferredList,
    mcpChange,
    moveOf,
    namesServer,
    storeResult,
  };
}
