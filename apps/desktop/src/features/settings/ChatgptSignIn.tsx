import { useEffect, useRef, useState } from "react";
import { Button, Spinner } from "@/components/controls";
import { Dialog, toast } from "@/components/overlay";
import { openExternal } from "@/features/sync/external";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AiChatgptSignIn, AiChatgptStatus, AiProviderView } from "@/ipc/types";
import { aiErrorMessage } from "./aiShared";
import s from "./AiPane.module.css";

/** ChatGPT's settings page for the usage of the plan by apps (AI-52). */
export const CHATGPT_USAGE_URL = "https://chatgpt.com/settings/usage";

/** The row hint of a ChatGPT provider's status, or null when it is signed in. */
export function chatgptRowKey(status: AiChatgptStatus) {
  switch (status) {
    case "signed_in":
      return null;
    case "signed_out":
      return "aiSettings.chatgpt.row.signed_out" as const;
    case "ended":
      return "aiSettings.chatgpt.row.ended" as const;
    case "plan_disabled":
      return "aiSettings.chatgpt.row.plan_disabled" as const;
  }
}

type Phase = { state: "waiting" } | { state: "failed"; message: string } | { state: "welcome"; provider: AiProviderView };

/**
 * AI-52: Continue with ChatGPT for a new account. The browser opens for the sign-in while this
 * dialog waits; the first sign-in of an account ends with the note that requests now use the ChatGPT
 * plan, with a link to manage its usage.
 */
export function ChatgptSignInDialog({ onDone, onClose }: { onDone: (provider: AiProviderView) => void; onClose: () => void }) {
  const t = useT();
  const [phase, setPhase] = useState<Phase>({ state: "waiting" });
  const [attempt, setAttempt] = useState(0);
  // Read through refs, so new functions on each render (from the parent, and `t`) do not restart the
  // sign-in, which would open the browser again.
  const done = useRef(onDone);
  done.current = onDone;
  const close = useRef(onClose);
  close.current = onClose;
  const tr = useRef(t);
  tr.current = t;
  /** The sign-in of the current attempt: StrictMode runs the effect twice, and the second run waits for
   * the same sign-in instead of opening the browser again. */
  const pending = useRef<{ attempt: number; result: Promise<AiChatgptSignIn> } | null>(null);

  useEffect(() => {
    // Per run: a run that ended (StrictMode's first, or the dialog closed) settles unseen, apart from a
    // provider the sign-in added.
    let live = true;
    if (pending.current?.attempt !== attempt) pending.current = { attempt, result: api.ai_chatgpt_sign_in(null) };
    const signIn = pending.current.result;
    void (async () => {
      try {
        const result = await signIn;
        // The provider exists now, also when the dialog closed first (Cancel after the browser came back).
        done.current(result.provider);
        if (!live) return;
        if (result.provider.chatgpt?.status === "plan_disabled") {
          toast(tr.current("aiSettings.chatgpt.status.plan_disabled"), "error", 6000);
          close.current();
        } else if (result.first_sign_in) {
          setPhase({ state: "welcome", provider: result.provider });
        } else {
          toast(tr.current("aiSettings.chatgpt.signedIn"), "success");
          close.current();
        }
      } catch (e) {
        if (!live) return;
        if (toAppError(e).code === "cancelled") close.current();
        else setPhase({ state: "failed", message: aiErrorMessage(tr.current, e) });
      }
    })();
    return () => {
      live = false;
    };
  }, [attempt]);

  const cancel = () => {
    void api.ai_chatgpt_sign_in_cancel();
    onClose();
  };

  if (phase.state === "welcome") {
    return (
      <Dialog
        title={t("aiSettings.chatgpt.welcome.title")}
        body={t("aiSettings.chatgpt.welcome.body")}
        icon="sparkle"
        onClose={onClose}
        actions={
          <>
            <Button onClick={() => void openExternal(CHATGPT_USAGE_URL)}>{t("aiSettings.chatgpt.manageUsage")}</Button>
            <Button variant="primary" autoFocus onClick={onClose}>
              {t("aiSettings.chatgpt.welcome.ok")}
            </Button>
          </>
        }
      />
    );
  }
  if (phase.state === "failed") {
    return (
      <Dialog
        title={t("aiSettings.chatgpt.failed.title")}
        body={<span className="selectable">{phase.message}</span>}
        icon="warning-circle"
        iconColor="var(--red)"
        onClose={onClose}
        actions={
          <>
            <Button onClick={onClose}>{t("btn.close")}</Button>
            <Button
              variant="primary"
              autoFocus
              onClick={() => {
                setPhase({ state: "waiting" });
                setAttempt((n) => n + 1);
              }}
            >
              {t("btn.retry")}
            </Button>
          </>
        }
      />
    );
  }
  return (
    <Dialog
      title={t("aiSettings.chatgpt.waiting.title")}
      body={
        <span className={s.waiting}>
          <Spinner />
          <span>{t("aiSettings.chatgpt.waiting.body")}</span>
        </span>
      }
      onClose={cancel}
      actions={<Button onClick={cancel}>{t("btn.cancel")}</Button>}
    />
  );
}
