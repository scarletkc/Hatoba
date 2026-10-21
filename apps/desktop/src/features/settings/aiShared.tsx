import { useRef, useState } from "react";
import { errorMessage } from "@/app/errors";
import { Button, Icon, LinkButton, StatusDot, TextField } from "@/components/controls";
import { useT, type MessageKey } from "@/i18n";
import { toAppError } from "@/ipc/api";
import type { AiTestResult } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { failureOf, type KeyDraft } from "./aiLogic";
import s from "./aiShared.module.css";

type T = ReturnType<typeof useT>;

/** The technical part of an `ai` failure: the HTTP status and the provider's own message. */
export function aiErrorDetail(t: T, e: unknown): { status: string | null; message: string | null } {
  const err = toAppError(e);
  if (err.code !== "ai") return { status: null, message: null };
  return {
    status: err.http_status ? t("aiSettings.err.status", { status: err.http_status }) : null,
    message: err.detail.trim() || null,
  };
}

/** A sentence for any failure; provider errors carry the status and the provider's message. */
export function aiErrorMessage(t: T, e: unknown): string {
  const err = toAppError(e);
  if (err.code !== "ai") return errorMessage(t, e);
  const { status, message } = aiErrorDetail(t, e);
  return [t("aiSettings.err.provider"), status, message].filter(Boolean).join(" · ");
}

/**
 * An API key field that behaves like a saved host password (HOST-08, AI-01): once a key is saved
 * it shows only **Saved**, and the key can be replaced or cleared but never viewed.
 */
export function SavedKeyField({
  id,
  hasSaved,
  draft,
  onChange,
  placeholder,
  invalid,
  allowClear = true,
  newPlaceholder,
  keepLabel,
  clearedLabel,
  ariaLabel,
  autoFocus,
}: {
  id?: string;
  hasSaved: boolean;
  draft: KeyDraft;
  onChange: (draft: KeyDraft) => void;
  placeholder: string;
  invalid?: boolean;
  /** Whether a saved key can be removed without a replacement. */
  allowClear?: boolean;
  /** Replaces the API key wording where the field holds another secret, such as an environment variable's value. */
  newPlaceholder?: string;
  keepLabel?: string;
  /** Replaces the API key wording of the cleared state. */
  clearedLabel?: string;
  /** Names the field when no label points at it. */
  ariaLabel?: string;
  autoFocus?: boolean;
}) {
  const t = useT();
  const inputRef = useRef<HTMLInputElement>(null);

  if (hasSaved && draft.mode === "keep") {
    return (
      <div className={s.savedRow}>
        <div className={s.savedField}>
          <Icon name="check-circle" fill />
          {t("aiSettings.f.apiKeySaved")}
        </div>
        <Button
          size="sm"
          onClick={() => {
            onChange({ mode: "replace", value: "" });
            requestAnimationFrame(() => inputRef.current?.focus());
          }}
        >
          {t("aiSettings.f.apiKeyReplace")}
        </Button>
        {allowClear && (
          <LinkButton tone="danger" onClick={() => onChange({ mode: "clear", value: "" })}>
            {t("aiSettings.f.apiKeyClear")}
          </LinkButton>
        )}
      </div>
    );
  }

  if (hasSaved && draft.mode === "clear") {
    return (
      <div className={s.savedRow}>
        <div className={s.savedField}>
          <Icon name="trash" />
          {clearedLabel ?? t("aiSettings.f.apiKeyCleared")}
        </div>
        <LinkButton onClick={() => onChange({ mode: "keep", value: "" })}>{t("aiSettings.f.apiKeyUndo")}</LinkButton>
      </div>
    );
  }

  return (
    <div className={s.keyRow}>
      <TextField
        id={id}
        ref={inputRef}
        secret
        autoComplete="new-password"
        autoFocus={autoFocus}
        value={draft.value}
        invalid={invalid}
        aria-label={ariaLabel}
        placeholder={hasSaved ? (newPlaceholder ?? t("aiSettings.f.apiKeyNewPlaceholder")) : placeholder}
        onChange={(e) => onChange({ mode: "replace", value: e.target.value })}
      />
      {hasSaved && <LinkButton onClick={() => onChange({ mode: "keep", value: "" })}>{keepLabel ?? t("aiSettings.f.apiKeyKeep")}</LinkButton>}
    </div>
  );
}

/** What a Test Connection button shows (AI-04). */
export type TestState =
  | { state: "idle" }
  | { state: "running" }
  | { state: "done"; result: AiTestResult; model: string | null }
  | { state: "error"; status: string | null; detail: string | null };

export const IDLE: TestState = { state: "idle" };

/**
 * The Test Connection button and its result: passed, or failed with the failure kind, the HTTP
 * status, and the provider's own message.
 */
export function TestConnection({
  scope,
  state,
  disabled,
  chatgpt,
  onRun,
}: {
  scope: "provider" | "search";
  state: TestState;
  disabled?: boolean;
  /** A ChatGPT provider (AI-52), which signs in instead of having a key. */
  chatgpt?: boolean;
  onRun: () => void;
}) {
  const t = useT();
  const running = state.state === "running";
  // A new test keeps the last result on screen, dimmed, so the sheet doesn't shrink and grow again.
  const [shown, setShown] = useState(state);
  if (!running && state !== shown) setShown(state);
  const result = running ? shown : state;

  let line: string | null = null;
  let ok = false;
  let status: string | null = null;
  let detail: string | null = null;
  if (result.state === "done") {
    const r = result.result;
    ok = r.ok;
    if (r.ok) {
      line =
        scope === "search"
          ? t("aiSettings.search.test.ok")
          : result.model
            ? t("aiSettings.test.okModel", { model: result.model })
            : t("aiSettings.test.okList");
    } else {
      const failure = failureOf(r.failure);
      if (scope === "search") {
        const kind = failure === "unknown_model" ? "other" : failure;
        line = t(`aiSettings.search.test.fail.${kind}` as MessageKey);
      } else if (chatgpt && failure === "auth") {
        line = t("aiSettings.chatgpt.test.auth");
      } else {
        line = t(`aiSettings.test.fail.${failure}` as MessageKey, { model: result.model ?? "" });
      }
      status = r.status ? t("aiSettings.err.status", { status: r.status }) : null;
      detail = r.message?.trim() || null;
    }
  } else if (result.state === "error") {
    line = t("aiSettings.err.provider");
    status = result.status;
    detail = result.detail;
  }

  return (
    <div className={s.test}>
      <div>
        <Button icon="plugs-connected" busy={running} disabled={disabled} onClick={onRun}>
          {t("aiSettings.test")}
        </Button>
      </div>
      {line && (
        <div className={cx(s.testResult, running && s.testStale)} role="status" aria-busy={running}>
          <StatusDot color={ok ? "var(--green)" : "var(--red)"} />
          <div className={s.testText}>
            <span className={ok ? undefined : s.testFail}>
              {line}
              {status && <span className={s.testStatus}>{status}</span>}
            </span>
            {detail && <span className={`${s.testDetail} selectable`}>{detail}</span>}
          </div>
        </div>
      )}
    </div>
  );
}
