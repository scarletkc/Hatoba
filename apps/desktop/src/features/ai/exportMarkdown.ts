import type { MessageKey, Params } from "@/i18n";
import type { AiConversationView, AiEntryView, AiToolCall } from "@/ipc/types";
import { fenced, quoted } from "./markdownText";
import { parseMessage } from "./selection";
import { exitStatusOf, prettyArgs, toolKind, toolLabel } from "./tools";

type ToolEntry = Extract<AiEntryView, { role: "tool" }>;
type Tr = (key: MessageKey, params?: Params) => string;

export interface ExportSource {
  conversation: AiConversationView;
  entries: AiEntryView[];
  /** The conversation's host, when it still exists. */
  host: { name: string; target: string } | null;
}

export interface ExportFormat {
  t: Tr;
  /** A date and time in the user's language. */
  time: (ms: number) => string;
}

function callStatus(t: Tr, call: AiToolCall, result: ToolEntry | undefined): string {
  if (!result) return t("ai.call.notRun");
  switch (result.status) {
    case "ok": {
      const code = toolKind(call.name) === "run_command" ? exitStatusOf(result.content) : null;
      return code !== null ? t("ai.call.exit", { code }) : t("ai.call.done");
    }
    case "error":
      return t("ai.call.error");
    case "rejected":
      return t("ai.call.rejected");
    case "cancelled":
      return t("ai.call.cancelled");
  }
}

/** A call with its input and result in fenced blocks. A rejected call's result is the user's reason. */
function toolCall(t: Tr, call: AiToolCall, result: ToolEntry | undefined): string {
  const parts = [`**${toolLabel(t, call.name)}** · ${callStatus(t, call, result)}`, `${t("ai.call.input")}:`, fenced(prettyArgs(call.arguments) || "{}", "json")];
  if (result?.status === "rejected") {
    const reason = result.content.trim();
    if (reason) parts.push(`${t("ai.approval.reason")}:`, fenced(reason));
  } else if (result) {
    parts.push(`${t("ai.call.output")}:`, fenced(result.content || t("ai.call.noOutput")));
  }
  return parts.join("\n\n");
}

/**
 * AI-25: the conversation as Markdown. The user's and the assistant's messages under headings,
 * reasoning as a quote, each tool call with its input and result in fenced blocks, and summaries and
 * the start of the context marked.
 */
export function conversationMarkdown({ conversation, entries, host }: ExportSource, { t, time }: ExportFormat): string {
  const results = new Map<string, ToolEntry>();
  for (const e of entries) if (e.role === "tool") results.set(e.tool_call_id, e);

  const meta: string[] = [];
  if (host) meta.push(`- ${t("ai.export.host")}: ${host.name} (\`${host.target}\`)`);
  else if (conversation.host_id) meta.push(`- ${t("ai.export.host")}: ${t("ai.history.hostGone")}`);
  meta.push(`- ${t("ai.export.created")}: ${time(conversation.created_at)}`);

  const blocks = [`# ${conversation.title || t("ai.untitled")}`, meta.join("\n")];
  const contextAt = conversation.context_start ? entries.findIndex((e) => e.entry_id === conversation.context_start) : -1;

  entries.forEach((entry, i) => {
    if (i === contextAt && i > 0) blocks.push("---", `*${t("ai.outsideContext")}*`);
    switch (entry.role) {
      case "user": {
        const { diagnostics, selection, typed } = parseMessage(entry.text);
        blocks.push(`## ${t("ai.export.you")}`);
        // AI-10: what was attached to the message, each as a labelled fenced block.
        if (diagnostics) blocks.push(`**${t("ai.diagnostics", { host: diagnostics.host })}**`, fenced(diagnostics.text));
        if (selection) {
          const label = [`**${t("ai.selection", { n: selection.lines })}**`, selection.host && `\`${selection.host}\``, selection.truncated && t("ai.selection.truncated")];
          blocks.push(label.filter(Boolean).join(" · "), fenced(selection.text));
        }
        blocks.push(typed);
        break;
      }
      case "assistant": {
        blocks.push(`## ${t("ai.export.assistant")} · ${entry.model_id}`);
        if (entry.reasoning?.trim()) blocks.push(quoted(`**${t("ai.reasoning")}**\n\n${entry.reasoning.trim()}`));
        if (entry.text.trim()) blocks.push(entry.text.trim());
        for (const call of entry.tool_calls) blocks.push(toolCall(t, call, results.get(call.id)));
        if (!entry.text.trim() && !entry.reasoning?.trim() && entry.tool_calls.length === 0) blocks.push(`*${t("ai.emptyReply")}*`);
        break;
      }
      case "summary":
        blocks.push(`## ${t("ai.summary")}`, entry.text.trim());
        break;
      case "tool":
        break; // with its call
    }
  });
  return `${blocks.join("\n\n")}\n`;
}

/** A file name for the export: the title without characters file systems refuse, or `fallback`. */
export function exportFileName(title: string, fallback: string): string {
  const base = title
    .replace(/[\\/:*?"<>|\u0000-\u001f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/^\.+/, "")
    .slice(0, 80)
    .replace(/[. ]+$/, "");
  return `${base || fallback}.md`;
}
