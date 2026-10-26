import { useId, useMemo, useState } from "react";
import { Button, Chip, ErrorText, HintText, Icon, LinkButton, isImeEvent } from "@hatoba/ui";
import { useT } from "@/i18n";
import type { AiToolCall, McpToolInfo } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { decideCall, type Decision } from "../actions/runner";
import { askSideQuestion } from "../actions/sideQuestions";
import { useAi } from "../store";
import { parseSendKey, readTerminalInput } from "../terminal/sendKeys";
import {
  allowanceScope,
  parseArgs,
  prettyArgs,
  proxyArguments,
  proxyCall,
  shownCall,
  toolKind,
  toolLabel,
  type Scope,
} from "../tools";
import type { AutoAsk } from "../turn";
import { AutoReason } from "./AutoVerdict";
import { argsFrom, callKeys, draftFrom, type Draft } from "./editDraft";
import { FileChange } from "./FileChange";
import { useFilePreview } from "./filePreview";
import type { HostInfo } from "./hostInfo";
import { KeysField, PressedKeys } from "./KeysField";
import { McpDetails } from "./McpDetails";
import { ShownInput, Workdir } from "./ShownInput";
import { TOOL_ICON } from "./toolIcons";
import s from "../messages/Messages.module.css";

interface AllowLabels {
  here: string;
  hereHint: string;
  /**
   * Null for an MCP server set to Always ask in bypass mode, which asks anyway, and for commands that
   * Always allow does not save (`savablePrefixes`).
   */
  always: string | null;
  alwaysHint: string;
}

/**
 * The labels of a card's Allow for this conversation and Always allow (AI-19, AI-31), which name what they
 * cover; `local` for a call on this computer (§7.5).
 */
function allowLabels(
  t: ReturnType<typeof useT>,
  scope: Scope,
  tool: string,
  mcp: McpToolInfo | null,
  bypass: boolean,
  local: boolean,
): AllowLabels {
  switch (scope.kind) {
    case "command": {
      const prefix = scope.prefixes.join(", ");
      return {
        here: t("ai.approval.allowCommandHere", { prefix }),
        hereHint: t("ai.approval.allowCommandHereHint", { prefix }),
        // Interpreters, deleting and the like are allowed for the conversation only (`savablePrefixes`).
        always: scope.savable
          ? t(local ? "ai.approval.alwaysAllowLocalCommand" : "ai.approval.alwaysAllowCommand", { prefix })
          : null,
        alwaysHint: t(local ? "ai.approval.alwaysAllowLocalCommandHint" : "ai.approval.alwaysAllowCommandHint", {
          prefix,
        }),
      };
    }
    case "domain": {
      const { domain } = scope;
      return {
        here: t("ai.approval.allowDomainHere", { domain }),
        hereHint: t("ai.approval.allowDomainHereHint", { domain }),
        always: t("ai.approval.alwaysAllowDomain", { domain }),
        alwaysHint: t("ai.approval.alwaysAllowDomainHint", { domain }),
      };
    }
    case "tool":
      return {
        here: t("ai.approval.allowHere"),
        hereHint: t("ai.approval.allowHereHint", { tool }),
        always: t(local ? "ai.approval.alwaysAllowLocal" : "ai.approval.alwaysAllowHost"),
        alwaysHint: t(local ? "ai.approval.alwaysAllowLocalHint" : "ai.approval.alwaysAllowHostHint", { tool }),
      };
    case "mcp":
      return {
        here: t("ai.approval.allowHere"),
        hereHint: t("ai.approval.allowHereHint", { tool }),
        always: mcp && !(bypass && mcp.always_ask) ? t("ai.approval.alwaysAllow") : null,
        alwaysHint: t("ai.approval.alwaysAllowHint", { tool }),
      };
  }
}

/**
 * AI-17: the tool, the tab's host, and the full input, with Run, Edit and Reject. An MCP call (AI-31)
 * shows its server, the tool's description and hints in place of the host. Allow for this conversation
 * and Always allow (AI-19) name what they cover: a command's prefixes, a page's domain, or the tool. A
 * `call_mcp_tool` call (AI-45) shows, and edits, the tool it names and that tool's own arguments. In Auto mode
 * (AI-47), it says why it asks.
 */
export function ApprovalCard({
  slotId,
  call,
  host,
  mcp,
  auto,
}: {
  slotId: string;
  call: AiToolCall;
  host: HostInfo | null;
  mcp: McpToolInfo | null;
  auto: AutoAsk | null;
}) {
  const t = useT();
  const shown = useMemo(() => shownCall(call), [call]);
  const proxied = proxyCall(call.name, call.arguments) !== null;
  const kind = toolKind(shown.name);
  const args = useMemo(() => parseArgs(shown.arguments) ?? {}, [shown.arguments]);
  const [mode, setMode] = useState<"view" | "edit" | "reject">("view");
  const [draft, setDraft] = useState<Draft>(() => draftFrom(kind, args, shown.arguments));
  const [reason, setReason] = useState("");
  const [invalid, setInvalid] = useState(false);
  const keysId = useId();
  const pressed = kind === "send_input" ? callKeys(args) : [];
  // AI-42: a side question about the call, which neither runs nor rejects it.
  const [question, setQuestion] = useState<string | null>(null);
  const label = mcp ? t("ai.tool.mcp", { server: mcp.server_name, tool: mcp.tool.tool }) : toolLabel(t, shown.name);
  const bypass = useAi((st) => st.slots[slotId]?.mode === "bypass");
  // AI-39, AI-40: an edit or write shows its diff, which follows the text changed in Edit.
  const fileChange = kind === "edit_file" || kind === "write_file";
  const editedFile = useMemo(() => {
    if (!fileChange || mode !== "edit") return null;
    const next = argsFrom(kind, args, draft);
    return "json" in next && next.json !== JSON.stringify(args) ? next.json : null;
  }, [fileChange, mode, kind, args, draft]);
  const file = useFilePreview(slotId, call.id, editedFile, fileChange);

  // AI-39, AI-40: an edit or write runs only once the card has shown its diff, which Rust then writes
  // over. When the card showed why the call cannot apply, Run answers the model with that instead, so a
  // file that comes back meanwhile is not written without a diff; without any answer, it waits.
  const holdRun = fileChange && (file.loading || !file.preview);
  const refusal = fileChange ? (file.preview?.error ?? undefined) : undefined;
  // AI-19: what Allow for this conversation and Always allow cover, for the input that would run; nothing
  // when the card shows why an edit or write cannot apply, since it showed no change.
  let allow: AllowLabels | null = null;
  if (refusal === undefined) {
    const next = mode === "edit" ? argsFrom(kind, args, draft) : null;
    const scope = allowanceScope(
      kind,
      next && "json" in next ? (parseArgs(next.json) ?? args) : args,
      mcp,
      host?.dialect,
    );
    allow = scope && allowLabels(t, scope, label, mcp, bypass, !!host?.local);
  }
  const run = (allow?: Extract<Decision, { kind: "run" }>["allow"]) => {
    if (holdRun) return;
    if (mode !== "edit") return decideCall(slotId, { kind: "run", edited: null, allow, refusal });
    const next = argsFrom(kind, args, draft);
    if ("error" in next) return setInvalid(true);
    const same = next.json === JSON.stringify(args);
    const edited = proxied ? proxyArguments(call.arguments, next.json) : next.json;
    decideCall(slotId, { kind: "run", edited: same ? null : edited, allow, refusal });
  };
  const reject = () => decideCall(slotId, { kind: "reject", reason: reason.trim() });
  const ask = () => {
    if (question !== null && askSideQuestion(slotId, question, false)) setQuestion(null);
  };
  /** Edit and Reject close the box of a side question. */
  const switchMode = (next: "edit" | "reject") => {
    setMode(next);
    setQuestion(null);
  };
  const str = (v: unknown) => (typeof v === "string" ? v : v === undefined || v === null ? "" : String(v));

  return (
    <div className={cx(s.tool, s.approval)} role="group" aria-label={t("ai.approval.title", { tool: label })}>
      <div className={s.approvalHead}>
        <Icon name={TOOL_ICON[kind]} size={14} className={s.toolIcon} />
        <span className={s.toolName}>{label}</span>
        <span className={s.gap} />
        <Chip tone="warning">{t("ai.call.approval")}</Chip>
      </div>
      {kind === "mcp"
        ? mcp && (
            <div className={s.approvalHost}>
              <Icon name="plug" size={12} />
              <span>{t("ai.approval.server")}</span>
              <span className={s.hostName}>{mcp.server_name}</span>
            </div>
          )
        : host && (
            <div className={s.approvalHost}>
              <Icon name={host.local ? "desktop" : "hard-drives"} size={12} />
              <span className={s.hostName}>{host.name}</span>
              <span className={s.hostTarget}>{host.target}</span>
            </div>
          )}
      {auto && <AutoReason ask={auto} />}
      {mcp?.always_ask && bypass && <HintText>{t("ai.approval.alwaysAsk")}</HintText>}
      {mcp && mode !== "edit" && <McpDetails mcp={mcp} />}

      {mode === "edit" ? (
        <div
          className={s.editForm}
          onKeyDown={(e) => {
            // Esc leaves the form instead of stopping the turn.
            if (e.key === "Escape" && !e.defaultPrevented) {
              e.stopPropagation();
              setMode("view");
            }
          }}
        >
          {kind === "fetch_url" ? (
            <input
              className={s.input}
              value={draft.main}
              aria-label={t("ai.approval.url")}
              spellCheck={false}
              onChange={(e) => setDraft({ ...draft, main: e.target.value })}
              autoFocus
            />
          ) : (
            <>
              {fileChange && (
                <div className={s.label}>
                  {kind === "edit_file" ? t("ai.approval.newString") : t("ai.approval.content")}
                </div>
              )}
              <textarea
                className={cx(s.input, s.textarea)}
                value={draft.main}
                aria-label={
                  kind === "send_input"
                    ? t("ai.approval.text")
                    : kind === "run_command"
                      ? t("ai.approval.command")
                      : kind === "edit_file"
                        ? t("ai.approval.newString")
                        : kind === "write_file"
                          ? t("ai.approval.content")
                          : t("ai.call.input")
                }
                spellCheck={false}
                rows={Math.min(fileChange ? 14 : 8, Math.max(2, draft.main.split("\n").length))}
                onChange={(e) => {
                  setInvalid(false);
                  setDraft({ ...draft, main: e.target.value });
                }}
                autoFocus
              />
            </>
          )}
          {kind === "send_input" && readTerminalInput(draft.main, str(args.text)) !== draft.main && (
            <HintText>{t("ai.approval.terminalEscapes")}</HintText>
          )}
          {invalid && kind !== "send_input" && <ErrorText>{t("ai.approval.invalidJson")}</ErrorText>}
          {kind === "run_command" && (
            <label className={s.inline}>
              {t("ai.approval.workdir")}
              <input
                className={cx(s.input, s.dirInput)}
                value={draft.dir}
                aria-label={t("ai.approval.workdir")}
                placeholder={t("ai.approval.homeDir")}
                spellCheck={false}
                onChange={(e) => setDraft({ ...draft, dir: e.target.value })}
              />
            </label>
          )}
          {kind === "run_command" && (
            <label className={s.inline}>
              {t("ai.approval.timeout")}
              <input
                className={cx(s.input, s.number)}
                type="number"
                min={1}
                max={600}
                placeholder="120"
                value={draft.timeout}
                onChange={(e) => setDraft({ ...draft, timeout: e.target.value })}
              />
              {t("ai.approval.seconds")}
            </label>
          )}
          {kind === "send_input" && (
            <>
              <div className={cx(s.inline, s.keysRow)}>
                <label htmlFor={keysId}>{t("ai.approval.key")}</label>
                <KeysField id={keysId} keys={draft.keys} onChange={(keys) => setDraft({ ...draft, keys })} />
              </div>
              {draft.keys.some((key) => parseSendKey(key) === null) && (
                <ErrorText>{t("ai.approval.invalidKey")}</ErrorText>
              )}
              <label className={s.inline}>
                {t("ai.approval.wait")}
                <input
                  className={cx(s.input, s.number)}
                  type="number"
                  min={1}
                  max={120}
                  placeholder="10"
                  value={draft.wait}
                  onChange={(e) => setDraft({ ...draft, wait: e.target.value })}
                />
                {t("ai.approval.seconds")}
              </label>
            </>
          )}
          {fileChange && <FileChange state={file} replaceAll={args.replace_all === true} />}
        </div>
      ) : (
        <div className={s.approvalInput}>
          {kind === "run_command" && (
            <>
              {str(args.workdir) && <Workdir path={str(args.workdir)} />}
              <ShownInput text={str(args.command)} command />
              <HintText>{t("ai.approval.timeoutValue", { s: str(args.timeout_seconds) || 120 })}</HintText>
            </>
          )}
          {kind === "send_input" && (
            <>
              <ShownInput text={str(args.text) || " "} command terminal />
              <HintText>
                {pressed.length ? <PressedKeys keys={pressed} /> : t("ai.approval.noKey")}
                {" · "}
                {t("ai.approval.waitValue", { s: str(args.wait_seconds) || 10 })}
              </HintText>
            </>
          )}
          {kind === "fetch_url" && (
            <>
              <ShownInput text={str(args.url)} command />
              {(args.full_page === true || args.offset !== undefined) && (
                <HintText>
                  {[
                    args.full_page === true && t("ai.approval.fullPage"),
                    args.offset !== undefined && t("ai.approval.offset", { n: str(args.offset) }),
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </HintText>
              )}
            </>
          )}
          {fileChange && <FileChange state={file} replaceAll={args.replace_all === true} />}
          {kind !== "run_command" && kind !== "send_input" && kind !== "fetch_url" && !fileChange && (
            <>
              {mcp && <div className={s.label}>{t("ai.approval.arguments")}</div>}
              <ShownInput text={prettyArgs(shown.arguments) || "{}"} />
            </>
          )}
        </div>
      )}

      {question !== null && mode === "view" && (
        <div className={s.rejectRow}>
          <input
            className={s.input}
            value={question}
            aria-label={t("ai.side.question")}
            onChange={(e) => setQuestion(e.target.value)}
            onFocus={(e) => e.currentTarget.select()}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !isImeEvent(e)) {
                e.preventDefault();
                ask();
              } else if (e.key === "Escape") {
                e.stopPropagation();
                setQuestion(null);
              }
            }}
            autoFocus
          />
          <div className={s.actions}>
            <Button size="sm" onClick={() => setQuestion(null)}>
              {t("btn.cancel")}
            </Button>
            <Button size="sm" variant="primary" icon="chat-teardrop-dots" disabled={!question.trim()} onClick={ask}>
              {t("ai.side.send")}
            </Button>
          </div>
        </div>
      )}

      {mode === "reject" ? (
        <div className={s.rejectRow}>
          <input
            className={s.input}
            value={reason}
            placeholder={t("ai.approval.reasonPlaceholder")}
            aria-label={t("ai.approval.reason")}
            onChange={(e) => setReason(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !isImeEvent(e)) {
                e.preventDefault();
                reject();
              } else if (e.key === "Escape") {
                e.stopPropagation();
                setMode("view");
              }
            }}
            autoFocus
          />
          <div className={s.actions}>
            <Button size="sm" onClick={() => setMode("view")}>
              {t("btn.cancel")}
            </Button>
            <Button size="sm" variant="danger" onClick={reject}>
              {t("ai.approval.reject")}
            </Button>
          </div>
        </div>
      ) : (
        <div className={s.actions}>
          <LinkButton tone="danger" onClick={() => switchMode("reject")}>
            {t("ai.approval.reject")}
          </LinkButton>
          {/* The model would see the call as stored, not as edited here, so not while editing. */}
          {question === null && mode === "view" && (
            <LinkButton
              tone="muted"
              icon="chat-teardrop-dots"
              title={t("ai.side.askCardHint")}
              onClick={() => setQuestion(t("ai.side.suggest"))}
            >
              {t("ai.side.askCard")}
            </LinkButton>
          )}
          <span className={s.gap} />
          {mode === "edit" ? (
            <Button size="sm" onClick={() => setMode("view")}>
              {t("btn.cancel")}
            </Button>
          ) : (
            <Button size="sm" icon="pencil-simple" onClick={() => switchMode("edit")}>
              {t("btn.edit")}
            </Button>
          )}
          {/* An edit or write runs once its card has an answer (a diff or why it cannot apply); so does Allow below. */}
          <Button size="sm" variant="primary" icon="play" disabled={holdRun} onClick={() => run()}>
            {t("ai.approval.run")}
          </Button>
        </div>
      )}
      {mode !== "reject" && allow && (
        <div className={s.allowRow}>
          <LinkButton
            tone="muted"
            icon="chat-circle-dots"
            title={allow.hereHint}
            disabled={holdRun}
            onClick={() => run("conversation")}
          >
            {allow.here}
          </LinkButton>
          {/* Always allow covers manual mode; an MCP server set to Always ask still asks in bypass mode. */}
          {allow.always && (
            <LinkButton
              tone="muted"
              icon="check-circle"
              title={allow.alwaysHint}
              disabled={holdRun}
              onClick={() => run("always")}
            >
              {allow.always}
            </LinkButton>
          )}
        </div>
      )}
    </div>
  );
}
