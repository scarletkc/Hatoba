/**
 * AI-50, AI-51: what running a `/` command and picking an `@` mention attach to the slot's next message. The
 * built-in commands run in `Composer.tsx`, which has the selector and the menus they open.
 */

import { toast } from "@/components/overlay";
import { t } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AiHostRefusal } from "@/ipc/types";
import { addAttachment, aiErrorMessage, selectionHost } from "./actions";
import { lineCount, pathName } from "./attachments";
import { listingText, promptArguments, resourceTarget, type PromptCommand, type ResourceItem } from "./commands";
import { menuSources, useMcp } from "./mcp";
import { getSlot, slotGeneration } from "./store";

/** The prompt a `/` command names among the running servers the slot's conversation uses, if any. */
export function findPrompt(slotId: string, name: string): PromptCommand | undefined {
  const { servers, status } = useMcp.getState();
  return menuSources(servers ?? [], status, getSlot(slotId).mcpOff).prompts.find((p) => p.command === name);
}

/**
 * Runs the skill or MCP prompt that a `/` command at the start of the box names (AI-50): its `SKILL.md`, or the
 * prompt's messages for the arguments that follow its name, become an attachment of the slot's next message, in place
 * of one the same command attached before. `none`: the command names neither, so the text goes as it was typed.
 * `failed` too when the slot shows another conversation by the time the text arrives: it attaches nothing there, and
 * for an `mcp__…` command that names no prompt of a running server (one still starting, say), which would otherwise
 * go as plain text without its prompt.
 */
export async function attachCommand(slotId: string, name: string, args: string): Promise<"attached" | "none" | "failed"> {
  const generation = slotGeneration(slotId);
  const attach = (attachment: Parameters<typeof addAttachment>[1]): "attached" | "failed" =>
    slotGeneration(slotId) === generation && addAttachment(slotId, attachment, name) ? "attached" : "failed";
  const prompt = findPrompt(slotId, name);
  if (prompt) {
    const names = prompt.prompt.arguments.map((a) => a.name);
    const values = promptArguments(args, names);
    const missing = prompt.prompt.arguments.find((a) => a.required && !values.some((v) => v.name === a.name && v.value));
    if (missing) {
      toast(t("ai.cmd.prompt.missing", { name: missing.name, command: `/${name}` }), "error");
      return "failed";
    }
    try {
      const text = await api.mcp_prompt_get(prompt.serverId, prompt.prompt.name, values);
      return attach({ kind: "mcp_prompt", server: prompt.server, name: prompt.prompt.name, text });
    } catch (e) {
      toast(aiErrorMessage(e), "error");
      return "failed";
    }
  }
  try {
    const text = await api.ai_skill_text(name);
    return attach({ kind: "skill", name, text });
  } catch (e) {
    if (toAppError(e).code !== "not_found") {
      toast(aiErrorMessage(e), "error");
      return "failed";
    }
    if (!name.startsWith("mcp__")) return "none";
    toast(t("ai.cmd.prompt.unavailable", { command: `/${name}` }), "error");
    return "failed";
  }
}

/** Why a path on the tab's host was not attached (`read`) or listed (`list`). */
export function hostRefusal(reason: AiHostRefusal, path: string, op: "read" | "list"): string {
  if (reason === "wrong_kind") return t(op === "read" ? "ai.mention.refused.isFolder" : "ai.mention.refused.notFolder", { path });
  return t(`ai.mention.refused.${reason}`, { path });
}

/** AI-51: attaches the text file at `path` on the host of the tab's session. */
export async function attachHostFile(slotId: string, sessionId: string, path: string): Promise<boolean> {
  try {
    const file = await api.ai_host_read(sessionId, path);
    if (file.status === "refused") {
      toast(hostRefusal(file.reason, path, "read"), "error");
      return false;
    }
    const text = file.text.replace(/\r\n?/g, "\n");
    return addAttachment(slotId, { kind: "host_file", host: selectionHost(slotId), path: file.path, lines: lineCount(text), text });
  } catch (e) {
    toast(aiErrorMessage(e), "error");
    return false;
  }
}

/** AI-51: attaches the entries of the directory at `path` on the host of the tab's session. */
export async function attachHostFolder(slotId: string, sessionId: string, path: string): Promise<boolean> {
  try {
    const dir = await api.ai_host_list(sessionId, path);
    if (dir.status === "refused") {
      toast(hostRefusal(dir.reason, path, "list"), "error");
      return false;
    }
    return addAttachment(slotId, {
      kind: "host_directory",
      host: selectionHost(slotId),
      path: dir.path,
      entries: dir.entries.length,
      truncated: dir.cut,
      text: listingText(dir.entries),
    });
  } catch (e) {
    toast(aiErrorMessage(e), "error");
    return false;
  }
}

/** AI-51: attaches a resource of an MCP server. */
export async function attachResource(slotId: string, item: ResourceItem): Promise<boolean> {
  try {
    const raw = await api.mcp_resource_read(item.serverId, item.resource.uri);
    const text = raw.replace(/\r\n?/g, "\n");
    return addAttachment(slotId, { kind: "mcp_resource", server: item.server, uri: item.resource.uri, lines: lineCount(text), text });
  } catch (e) {
    toast(aiErrorMessage(e), "error");
    return false;
  }
}

/** The name a chip shows for a mention: a file's or folder's last part, or a resource's `server:uri`. */
export function mentionLabel(target: string, resource: ResourceItem | null): string {
  return resource ? resourceTarget(resource) : pathName(target) || target;
}
