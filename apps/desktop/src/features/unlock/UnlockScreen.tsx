import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { enterUnlocked } from "@/app/boot";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { WindowChrome } from "@/app/TitleBar";
import { AppLogo, Icon, Spinner } from "@/components/controls";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import { cx } from "@/lib/cx";
import { RecoverSheet } from "./RecoverSheet";
import s from "./UnlockScreen.module.css";

/** Full-window lock screen (design §06, VAULT-03, SEC-06, SEC-07, SEC-11). */
export function UnlockScreen() {
  const t = useT();
  const vault = useApp((st) => st.vault);
  const lockReason = useApp((st) => st.lockReason);
  const platform = useApp((st) => st.info.platform);
  const sessions = useTabs((st) => st.tabs.filter((tab) => tab.status === "connected").length);

  // SEC-11: the field takes the PIN while it can unlock, unless the user picked the master password.
  const [usePassword, setUsePassword] = useState(false);
  const pinMode = !!vault?.pin_unlock && !usePassword;
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shake, setShake] = useState(false);
  const [retryAt, setRetryAt] = useState<number | null>(vault?.retry_at ?? null);
  const [now, setNow] = useState(() => Date.now());
  const [recovering, setRecovering] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  // The password shown goes back to dots when the window loses focus, so it is not left on screen
  // for whoever comes to the computer next.
  useEffect(() => {
    const hide = () => setRevealed(false);
    window.addEventListener("blur", hide);
    return () => window.removeEventListener("blur", hide);
  }, []);

  // SEC-06: while throttled the field is disabled and a live countdown replaces the error text. The
  // delay is the master password's; the PIN has its own limit.
  const throttled = !pinMode && retryAt !== null && now < retryAt;
  const remaining = retryAt ? Math.max(1, Math.ceil((retryAt - now) / 1000)) : 0;

  useEffect(() => {
    if (retryAt === null) return;
    setNow(Date.now());
    const timer = window.setInterval(() => {
      const n = Date.now();
      setNow(n);
      if (n >= retryAt) {
        window.clearInterval(timer);
        setRetryAt(null);
      }
    }, 250);
    return () => window.clearInterval(timer);
  }, [retryAt]);

  useEffect(() => {
    if (!throttled && !recovering) input.current?.focus();
  }, [throttled, recovering]);

  // The wrong password stays in the field, which is disabled with its Show button for the delay.
  useEffect(() => {
    if (throttled) setRevealed(false);
  }, [throttled]);

  // The backend may already be throttling when the screen appears (e.g. after an idle lock).
  useEffect(() => {
    if (vault?.retry_at) setRetryAt(vault.retry_at);
  }, [vault?.retry_at]);

  const switchMode = (toPassword: boolean) => {
    setUsePassword(toPassword);
    setPassword("");
    setError(null);
    setRevealed(false);
    requestAnimationFrame(() => input.current?.focus());
  };

  const fail = useCallback(
    async (e: unknown) => {
      const err = toAppError(e);
      if (err.code === "cancelled") return;
      // The PIN no longer unlocks (too many wrong ones, or the app restarted): the field asks for
      // the master password, under the reason.
      if (err.code === "pin_unavailable" || (err.code === "wrong_pin" && !err.attempts_left)) {
        setUsePassword(true);
        setPassword("");
        void useApp.getState().refreshVault().catch(() => null);
      }
      if (err.code === "throttled" && err.retry_at) setRetryAt(err.retry_at);
      else if (err.code === "wrong_password") {
        const v = await useApp.getState().refreshVault().catch(() => null);
        if (v?.retry_at) setRetryAt(v.retry_at);
      }
      setError(errorMessage(t, e));
      if (err.code === "wrong_password" || err.code === "throttled" || err.code === "wrong_pin") setShake(true);
      requestAnimationFrame(() => {
        input.current?.focus();
        input.current?.select();
      });
    },
    [t],
  );

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!password || busy || throttled) return;
    setBusy(true);
    setError(null);
    try {
      await (pinMode ? api.vault_unlock_pin(password) : api.vault_unlock(password));
      setPassword("");
      await enterUnlocked();
    } catch (err) {
      setBusy(false);
      await fail(err);
    }
  };

  const unlockBiometric = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.vault_unlock_biometric();
      await enterUnlocked();
    } catch (err) {
      setBusy(false);
      await fail(err);
    }
  };

  // SEC-07: Windows Hello asks on its own, once per lock, when the user comes back to the window: at
  // once when the app starts in front, when the window gets the focus, or (after an idle or sleep
  // lock) at the first pointer movement in it. After a manual lock only the focus counts, so the
  // window has to be left first; otherwise a face would unlock it again at once.
  const prompted = useRef(false);
  const autoPrompt = useRef(() => {});
  autoPrompt.current = () => {
    if (prompted.current || busy || recovering || !document.hasFocus() || document.visibilityState !== "visible") return;
    prompted.current = true;
    void unlockBiometric();
  };
  const biometricOn = !!vault?.biometric_enabled;
  useEffect(() => {
    if (!biometricOn) return;
    const prompt = () => autoPrompt.current();
    const onPointer = lockReason === "manual" ? null : prompt;
    if (lockReason === null) prompt();
    window.addEventListener("focus", prompt);
    if (onPointer) {
      window.addEventListener("pointermove", onPointer);
      window.addEventListener("pointerdown", onPointer);
    }
    return () => {
      window.removeEventListener("focus", prompt);
      if (onPointer) {
        window.removeEventListener("pointermove", onPointer);
        window.removeEventListener("pointerdown", onPointer);
      }
    };
  }, [biometricOn, lockReason]);

  const message = throttled ? t("err.throttled", { s: remaining }) : error;
  const biometricLabel = platform === "macos" ? t("vault.unlock.touchId") : t("vault.unlock.hello");

  return (
    <div className={s.root}>
      <WindowChrome />
      <div className={s.center}>
        <AppLogo className={s.logo} />
        <h1 className={s.title}>{t("vault.unlock.title")}</h1>
        <div className={s.subtitle}>{t(pinMode ? "vault.unlock.subtitlePin" : "vault.unlock.subtitle")}</div>

        <form className={s.form} onSubmit={(e) => void submit(e)}>
          <div
            className={cx(s.field, message && s.fieldError, shake && s.shake, revealed && s.fieldRevealed)}
            onAnimationEnd={() => setShake(false)}
          >
            <input
              ref={input}
              type={revealed ? "text" : "password"}
              autoFocus
              autoComplete={pinMode ? "off" : "current-password"}
              aria-label={t(pinMode ? "vault.unlock.pinField" : "vault.unlock.field")}
              aria-invalid={!!error || undefined}
              spellCheck={false}
              autoCorrect="off"
              autoCapitalize="off"
              disabled={throttled}
              readOnly={busy}
              value={password}
              onChange={(e) => {
                setPassword(e.target.value);
                if (error) setError(null);
              }}
            />
            {/* Keeps the focus in the field, so typing and Enter go on from where they were. */}
            <button
              type="button"
              className={s.reveal}
              aria-label={revealed ? t("field.hide") : t("field.show")}
              title={revealed ? t("field.hide") : t("field.show")}
              disabled={throttled}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => setRevealed((v) => !v)}
            >
              <Icon name={revealed ? "eye-slash" : "eye"} />
            </button>
            <button type="submit" className={s.go} aria-label={t("vault.unlock.submit")} disabled={!password || busy || throttled}>
              {busy ? <Spinner size={13} /> : <Icon name="arrow-right" />}
            </button>
          </div>
          <div className={s.message} role="alert" aria-live="assertive">
            {message}
          </div>

          {vault?.biometric_enabled && (
            <>
              <div className={s.divider}>{t("vault.unlock.or")}</div>
              <button type="button" className={s.biometric} disabled={busy} onClick={() => void unlockBiometric()}>
                <Icon name="fingerprint" />
                {biometricLabel}
              </button>
            </>
          )}
        </form>

        <div className={s.links}>
          {pinMode ? (
            <button type="button" className={s.forgot} onClick={() => switchMode(true)}>
              {t("vault.unlock.usePassword")}
            </button>
          ) : (
            <>
              {vault?.pin_unlock && (
                <button type="button" className={s.forgot} onClick={() => switchMode(false)}>
                  {t("vault.unlock.usePin")}
                </button>
              )}
              <button type="button" className={s.forgot} onClick={() => setRecovering(true)}>
                {t("vault.unlock.forgot")}
              </button>
            </>
          )}
        </div>
      </div>

      <div className={s.foot}>
        {sessions > 0 && (
          <div className={s.sessions}>
            <span className={s.sessionDot} />
            {t("lock.sessionsKept", { n: sessions })}
          </div>
        )}
        <div>{vault && vault.sync_kind !== "none" ? t("vault.unlock.footer") : t("vault.unlock.footerOff")}</div>
      </div>

      {recovering && <RecoverSheet onClose={() => setRecovering(false)} />}
    </div>
  );
}
