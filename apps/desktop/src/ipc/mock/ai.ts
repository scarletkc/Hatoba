import { noteBlock, parseMessage, splitPermissionNote } from "@/lib/ai/attachments";
import { forkCopies } from "@/lib/ai/fork";
import { recordedTarget } from "@/lib/ai/place";
import { parseQuickConnect } from "@/session/quickConnect";
import type { AiEntryView, AiSearchHit, AiToolResultInput } from "../types";
import { createAiFiles } from "./aiFiles";
import { createAiJudge } from "./aiJudge";
import { createAiModel } from "./aiModel";
import { createAiSeed } from "./aiSeed";
import {
  chunks,
  createAiShared,
  placeOf,
  snippetAround,
  titleOf,
  type AiApi,
  type AiMockDeps,
  type Conv,
} from "./aiShared";
import { createAiTurns } from "./aiTurns";

export type { AiMockDeps };

/**
 * Mock of the AI conversations and turns (§13) for `pnpm dev`. A scripted fake model streams
 * reasoning and then Markdown with timers, and calls tools by keyword in the user's message (English,
 * Chinese or Japanese). Without a connected terminal tab it calls only the tools that need none (AI-09):
 *   "disk"            run_command `df -h /`          "fail"     run_command that exits with 5
 *                     (on a local PowerShell or Command Prompt tab, one in its syntax that only reads, §13.5)
 *   "screen"          read_terminal                  "type" / "send"   send_input `uptime` + Enter
 *   "search"          web_search                     "fetch" / "url"   fetch_url ("whole": the whole page)
 *   "mcp"          a tool of the first running (or startable) MCP server the conversation uses,
 *                     from the extensions mock (`?mcp=demo`), preferring a server named in the message
 *                     ("mcp filesystem"); without one, a tool no server offers
 *   "unknown"         a tool that does not exist     "sleep"    run_command `sleep 8`, to see Stop
 *   "hidden"          run_command with several lines, a tab, an escape sequence and a right-to-left
 *                     override, to see how the approval card shows them (AI-17)
 *   "read file"       read_file of an nginx site        "edit"     edit_file of its proxy block
 *   "listen"          edit_file with replace_all         "nomatch"  edit_file whose old_string is missing
 *   "crlf"            edit_file of a CRLF file           "write"    write_file of a new file
 *   "bigfile"         write_file of a 1,200-line file, to see the diff paged
 *   "unread"          edit_file of a file the conversation has not read, which is refused
 *   "motd"            write_file over an existing file   (AI-38…40: the cards show diffs; the files are
 *                     shared by every tab, and a run changes them for the next card)
 *   "cutshort"        write_file whose write fails halfway, leaving the file cut short
 *   "grep"            grep of the proxy lines under /etc   "glob"     glob of the nginx sites (AI-48, AI-49)
 *   "rmfile"          run_command `rm /etc/motd`     "symlink"  run_command that puts a link in the place
 *                     of the file "write" creates, so reverting "motd" offers to create the file again and
 *                     reverting "write" stops at the link (AI-41)
 * An edit or write that ran, or failed after it may have begun, can be reverted from its block while the
 * conversation is idle (AI-41), and the next message notes it. "listen" then "edit" changes one file twice:
 * reverting the first then shows the diff, says the later call is in the way, and offers Restore Anyway.
 * The mock keeps few snapshots, so "bigfile" after another change drops the older one.
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
 *   autocompact every turn of a conversation that has an answer compacts it first (AI-22)
 *   effortfail every request that carries a thinking level is refused for it and goes again
 *              without it, so the panel says the level was not used (AI-05)
 *   updates    every response thinks for 3 s with no reasoning text, like a Claude model that writes
 *              progress updates (AI-06): the status says Thinking, and no Reasoning block shows
 *   showcase   in a connected tab, any message gets the scripted nginx 502 fix that the README
 *              screenshots and demo video show: two commands, a fix, `curl` typed into the shell, a summary
 *   noprovider (Settings → AI mock) no provider is configured
 *   nojudge    (Settings → AI mock) no judge is chosen, so Auto mode is unavailable (AI-47)
 *   judgeslow  the judge of Auto mode takes 4 seconds, to show a call being checked
 *   judgefail  the judge of Auto mode times out, so every card says it could not judge
 * In Auto mode (AI-47) the mock judge runs commands of reading words, and asks about `rm`, `restart`,
 * `install`, `curl`, redirection and the like unless the message names what the call works on.
 * A side question (AI-42) gets a short scripted answer, streamed slowly enough to stop: about the call whose
 * approval card waits, or else about the conversation. It stores nothing, and the turn goes on.
 * History search (AI-24) matches titles and the text of user, assistant and summary entries.
 * A fork (AI-43) copies a conversation into a new one, like Rust: Fork from Here on a message or under a
 * reply, or Fork in History.
 * Like Rust, a message that moves the conversation to another host (AI-09) or goes to another model
 * than the last reply (AI-05) is stored with Hatoba's note before it, which the panel shows as a
 * divider: open a conversation from history in a tab on another host, or pick another model.
 * Nothing here is secure; it never runs inside the Tauri app.
 */
export function createAiMock(deps: AiMockDeps): AiApi {
  const ctx = createAiShared(deps);
  const {
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
    sides,
    newId,
    fail,
    delay,
    need,
    view,
    answered,
    openCall,
  } = ctx;

  const { sideAnswer, plan } = createAiModel(ctx);
  const files = createAiFiles(ctx);
  const {
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
  } = createAiTurns(ctx, { plan, pendingReverts: files.pendingReverts });
  const judgeApi = createAiJudge(ctx);
  const { seed, settleRemote } = createAiSeed(ctx);

  if (flags.has("history") || flags.has("running")) seed();

  return {
    ...judgeApi,
    ...files.api,
    ai_conversations_list: async () => {
      await delay(150);
      return [...convs.values()].map(view);
    },
    ai_conversation_get: async (id) => {
      await delay(120);
      const c = need(id);
      settleRemote(c);
      return {
        conversation: view(c),
        entries: c.entries.map((e) => ({ ...e })),
        running: turns.has(id) || c.runningUntil > Date.now(),
        file_changes: files.fileChangesOf(id),
      };
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
      files.deleteSnapshots(id);
    },
    ai_conversation_fork: async (id, entryId, title) => {
      await delay(120);
      const c = need(id);
      if (turns.has(id) || compacting.has(id) || c.runningUntil > Date.now())
        fail("invalid_input", "The conversation can be forked once its turn or compaction has ended.", {
          field: "conversation_id",
        });
      const name = [...title.trim()].slice(0, 200).join("");
      if (!name) fail("invalid_input", "the title is empty", { field: "title" });
      const at = entryId === null ? c.entries.length - 1 : c.entries.findIndex((e) => e.entry_id === entryId);
      if (entryId !== null && at < 0) fail("not_found", "message not found");
      if (at < 0) fail("invalid_input", "the conversation has no messages", { field: "conversation_id" });
      const copies = entryId === null ? [...c.entries.keys()] : forkCopies(c.entries, at);
      if (!copies)
        return fail(
          "invalid_input",
          "A conversation can be forked from one of your messages or from a finished reply.",
          { field: "entry_id" },
        );
      // New, increasing ids; what Rust keeps in an entry's JSON goes with the copy.
      const copied = copies.map((i) => {
        const e = c.entries[i];
        const entry_id = newId();
        const terminal = replyTerminal.get(e.entry_id);
        if (terminal !== undefined) replyTerminal.set(entry_id, terminal);
        const shell = replyShell.get(e.entry_id);
        if (shell !== undefined) replyShell.set(entry_id, shell);
        const servers = replyServers.get(e.entry_id);
        if (servers) replyServers.set(entry_id, servers);
        if (interrupted.has(e.entry_id)) interrupted.add(entry_id);
        const recorded = cameFrom.get(e.entry_id);
        if (recorded) cameFrom.set(entry_id, recorded);
        return { ...e, entry_id };
      });
      // The copy of the entry the original's context starts at, or the newest copied summary.
      let start = -1;
      const origin = c.view.context_start;
      if (origin) {
        const from = c.entries.findIndex((e) => e.entry_id >= origin);
        start = from >= 0 && from <= at ? from : c.entries.slice(0, at + 1).findLastIndex((e) => e.role === "summary");
      }
      // Like Rust's `fork_place`: what the first later message recorded, or the name its move's note gives, or the
      // original's place.
      let place = placeOf(c.view);
      for (const e of c.entries.slice(at + 1)) {
        if (e.role !== "user") continue;
        const recorded = cameFrom.get(e.entry_id);
        if (recorded) {
          place = recorded;
          break;
        }
        const moved = parseMessage(e.text).notes.find((n) => n.kind === "host_change");
        if (moved?.kind !== "host_change") continue;
        const target = /\s/.test(moved.from) ? null : parseQuickConnect(moved.from);
        const named = ((await deps.hosts?.()) ?? []).filter((h) => moved.from && h.name === moved.from);
        const host_id = named.length === 1 ? named[0].id : null;
        const quick_target =
          !host_id && target?.ok && target.target.username
            ? recordedTarget({ ...target.target, username: target.target.username })
            : null;
        const local =
          !host_id && !quick_target && moved.from.endsWith(" (local terminal)")
            ? moved.from.slice(0, -" (local terminal)".length)
            : null;
        place = { host_id, quick_target, local, origin: host_id || quick_target || local ? null : moved.from };
        break;
      }
      const now = Date.now();
      const fork: Conv = {
        view: {
          id: newId(),
          title: name,
          ...place,
          pinned: false,
          context_start: copied[start]?.entry_id ?? null,
          effort: c.view.effort,
          created_at: now,
          updated_at: now,
        },
        entries: copied,
        runningUntil: 0,
      };
      convs.set(fork.view.id, fork);
      return view(fork);
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
        // Like Rust: the new conversation's id goes out before the message waits for the MCP servers (AI-45).
        onEvent({ kind: "opened", conversation_id: id });
        c = {
          view: {
            id,
            title: titleOf(input.text),
            host_id: input.context.host_id,
            quick_target: !input.context.host_id && input.context.target ? recordedTarget(input.context.target) : null,
            local: !input.context.host_id && !input.context.target ? (input.context.local?.device ?? null) : null,
            origin: null,
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
      const listed = await deferredList(input.context, input.text);
      const notes =
        (await notesBefore(c, earlier, move, move?.from ?? null, input.context)) +
        listed +
        (await mcpChange(earlier, input.context, listed));
      const recorded = input.conversation_id
        ? placeOf(c.view)
        : { host_id: null, quick_target: null, origin: null, local: null };
      const user_entry = await storeMessage(c, turn, notes + input.text, input.context, move, recorded);
      return { conversation: view(c), user_entry };
    },
    ai_retry: async (id, context, onEvent) => {
      await delay(80);
      const c = need(id);
      await checkModel(context);
      // Like Rust: a conversation that ends with the model's answer has nothing to retry.
      const last = [...contextOf(c)].reverse().find((e) => e.role !== "tool");
      if (!last || (last.role === "assistant" && last.tool_calls.length === 0))
        fail("invalid_input", "nothing to retry: the conversation ends with the model's answer", {
          field: "conversation_id",
        });
      if (c.view.effort !== context.effort) c.view = { ...c.view, effort: context.effort, updated_at: Date.now() };
      startTurn(c, context, onEvent);
    },
    ai_tool_result: async (id, callId, result: AiToolResultInput) => {
      const c = need(id);
      openCall(c, callId);
      // Like Rust: a rejection stores the user's reason; the adapter tells the model the call was rejected.
      const content = result.status === "rejected" ? result.content.trim() : result.content;
      files.clearFileBase(callId);
      return storeResult(c, callId, result.status, content, result.edited_arguments, result.judged ?? false);
    },
    ai_tool_run: async (id, callId, sessionId, edited, judged) => {
      const c = need(id);
      const x = openCall(c, callId);
      const r = await files.runTool(c, x, sessionId, edited);
      // A stop stored a cancelled result while the tool ran.
      const dropped = files.consumeDropped(callId);
      if (answered(c).has(callId)) fail("cancelled", "the tool call was stopped");
      return { entry: storeResult(c, callId, r.status, r.content, edited, judged), dropped_file_changes: dropped };
    },
    ai_session_server: async (sessionId) => (sessionId ? (deps.sessionServer?.(sessionId) ?? null) : null),
    ai_stop: async (id) => {
      const c = convs.get(id);
      if (c?.runningUntil) c.runningUntil = 0;
      stopTurn(id);
    },
    ai_compact: async (id) => {
      const c = need(id);
      if (turns.has(id)) fail("invalid_input", "a turn is running");
      compacting.add(id);
      await delay(1500);
      compacting.delete(id);
      // The summary no longer holds the files the model read (AI-38…40).
      files.forgetSeen(id);
      const entry: AiEntryView = {
        role: "summary",
        entry_id: newId(),
        created_at: Date.now(),
        text: summaryText(
          c,
          sayZh(
            `**Summary of the earlier conversation**: ${c.entries.length} entries. The user and the assistant went over this host’s state and finished the checks.`,
            `**之前的对话摘要**：共 ${c.entries.length} 条记录。用户和助手讨论了这台主机的状态，并完成了相关检查。`,
            `**之前的對話摘要**：共 ${c.entries.length} 條記錄。使用者和助理討論了這台主機的狀態，並完成了相關檢查。`,
          ),
        ),
      };
      c.entries.push(entry);
      c.view = { ...c.view, context_start: entry.entry_id, updated_at: Date.now() };
      return entry;
    },
    ai_side_question: async (input, onEvent) => {
      const id = input.conversation_id;
      const c = need(id);
      // Like Rust: a newer side question of the conversation stops the one before it, even one that is refused.
      sides.get(id)?.stop();
      let stopped = false;
      const wakers = new Set<() => void>();
      const stop = () => {
        stopped = true;
        wakers.forEach((wake) => wake());
      };
      const mine = { question: input.question_id, stop };
      sides.set(id, mine);
      const pause = (ms: number) =>
        new Promise<boolean>((resolve) => {
          const wake = () => resolve(false);
          wakers.add(wake);
          window.setTimeout(() => {
            wakers.delete(wake);
            resolve(!stopped);
          }, ms);
        });
      const gone = () => fail("cancelled", "the side question was stopped");
      try {
        if (!parseMessage(input.text).typed.trim()) fail("invalid_input", "The question is empty.", { field: "text" });
        if (compacting.has(id))
          fail("invalid_input", "the conversation is being compacted", { field: "conversation_id" });
        await checkModel(input.context);
        if (!(await pause(500))) gone();
        for (const piece of chunks(
          sayZh(
            "Looking at what the conversation shows so far…",
            "先看看对话里已有的内容……",
            "先看看對話裡已有的內容……",
          ),
        )) {
          if (!(await pause(20))) gone();
          onEvent({ kind: "reasoning", delta: piece });
        }
        const text = sideAnswer(c, input);
        for (const piece of chunks(text)) {
          if (!(await pause(28))) gone();
          onEvent({ kind: "text", delta: piece });
        }
        return { text, finish: "stop" };
      } finally {
        if (sides.get(id) === mine) sides.delete(id);
      }
    },
    ai_side_stop: async (id, question) => {
      const side = sides.get(id);
      if (side?.question === question) side.stop();
    },
    ai_search: async (query) => {
      await delay(200);
      const q = query.trim().toLowerCase();
      if (!q) return [];
      // Like Rust, without the screen a message showed (AI-46), whose text is the host's, or the
      // permission mode note (§13.5).
      const withoutNotes = (text: string) =>
        parseMessage(text)
          .notes.filter((n) => n.kind === "terminal_state" || n.kind === "permission_mode")
          .reduce((t, n) => t.replace(`${noteBlock(n)}\n\n`, ""), text);
      const textOf = (e: AiEntryView) =>
        e.role === "user"
          ? withoutNotes(e.text)
          : e.role === "summary"
            ? splitPermissionNote(e.text).summary
            : e.role === "assistant"
              ? e.text
              : "";
      const hits: AiSearchHit[] = [];
      for (const c of [...convs.values()].sort((a, b) => view(b).last_activity - view(a).last_activity)) {
        const entry = c.entries.find((e) => textOf(e).toLowerCase().includes(q));
        if (entry)
          hits.push({ conversation_id: c.view.id, entry_id: entry.entry_id, snippet: snippetAround(textOf(entry), q) });
        else if (c.view.title.toLowerCase().includes(q))
          hits.push({ conversation_id: c.view.id, entry_id: null, snippet: c.view.title });
      }
      return hits;
    },
    ai_edit_resend: async (id, entryId, text, context, onEvent) => {
      await delay(120);
      const c = need(id);
      const i = c.entries.findIndex((e) => e.entry_id === entryId);
      if (i < 0) fail("not_found", "entry not found");
      if (c.entries[i].role !== "user")
        fail("invalid_input", "only the user's messages can be edited", { field: "entry_id" });
      if (!text.trim()) fail("invalid_input", "the message is empty", { field: "text" });
      await checkModel(context);
      files.forgetSeen(id);
      // The stopped turn's cancelled results go with everything else after the message.
      stopTurn(id);
      // Like Rust: the earliest move the deleted messages noted still tells where the earlier screens came from.
      const moved = c.entries
        .slice(i)
        .flatMap((e) => (e.role === "user" ? parseMessage(e.text).notes : []))
        .find((n) => n.kind === "host_change");
      let noted: string | null = moved?.kind === "host_change" ? moved.from : null;
      // Like Rust: the message records what the edited one did.
      const recorded = cameFrom.get(entryId) ?? null;
      c.entries = c.entries.slice(0, i);
      // Like Rust: the snapshots of the calls the deleted responses made go with them (AI-41).
      files.keepSnapshots(id, c.entries);
      if (c.view.context_start && !c.entries.some((e) => e.entry_id === c.view.context_start)) {
        const summary = [...c.entries].reverse().find((e) => e.role === "summary");
        c.view = { ...c.view, context_start: summary?.entry_id ?? null };
      }
      c.view = { ...c.view, updated_at: Date.now() };
      const turn = startTurn(c, context, onEvent, false);
      const move = await moveOf(c, context);
      // Like Rust: the kept move is dropped when its note names the server the message goes from under another name.
      if (noted !== null && (await namesServer(c, move, noted))) noted = null;
      else if (noted === null) noted = move?.from ?? null;
      const listed = await deferredList(context, text);
      const notes =
        (await notesBefore(c, [...c.entries], move, noted, context)) +
        listed +
        (await mcpChange([...c.entries], context, listed));
      const user_entry = await storeMessage(c, turn, notes + text, context, move, recorded);
      return { conversation: view(c), user_entry };
    },
    // The browser reads dropped files itself (File API); there are no paths to read here.
    ai_read_dropped_files: async (paths) =>
      paths.map((p) => ({ status: "refused", name: p.split(/[\\/]/).pop() ?? p, reason: "unreadable" })),
  };
}
