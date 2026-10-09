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

/** Full-window lock screen (design §06, VAULT-03, SEC-06). */
export function UnlockScreen() {
  const t = useT();
  const vault = useApp((st) => st.vault);
  const platform = useApp((st) => st.info.platform);
  const sessions = useTabs((st) => st.tabs.filter((tab) => tab.status === "connected").length);

  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [shake, setShake] = useState(false);
  const [retryAt, setRetryAt] = useState<number | null>(vault?.retry_at ?? null);
  const [now, setNow] = useState(() => Date.now());
  const [recovering, setRecovering] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  // SEC-06: while throttled the field is disabled and a live countdown replaces the error text.
  const throttled = retryAt !== null && now < retryAt;
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

  // The backend may already be throttling when the screen appears (e.g. after an idle lock).
  useEffect(() => {
    if (vault?.retry_at) setRetryAt(vault.retry_at);
  }, [vault?.retry_at]);

  const fail = useCallback(
    async (e: unknown) => {
      const err = toAppError(e);
      if (err.code === "cancelled") return;
      if (err.code === "throttled" && err.retry_at) setRetryAt(err.retry_at);
      else if (err.code === "wrong_password") {
        const v = await useApp.getState().refreshVault().catch(() => null);
        if (v?.retry_at) setRetryAt(v.retry_at);
      }
      setError(errorMessage(t, e));
      if (err.code === "wrong_password" || err.code === "throttled") setShake(true);
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
      await api.vault_unlock(password);
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

  const message = throttled ? t("err.throttled", { s: remaining }) : error;
  const biometricLabel = platform === "macos" ? t("vault.unlock.touchId") : t("vault.unlock.hello");

  return (
    <div className={s.root}>
      <WindowChrome />
      <div className={s.center}>
        <AppLogo className={s.logo} />
        <h1 className={s.title}>{t("vault.unlock.title")}</h1>
        <div className={s.subtitle}>{t("vault.unlock.subtitle")}</div>

        <form className={s.form} onSubmit={(e) => void submit(e)}>
          <div
            className={cx(s.field, message && s.fieldError, shake && s.shake)}
            onAnimationEnd={() => setShake(false)}
          >
            <input
              ref={input}
              type="password"
              autoFocus
              autoComplete="current-password"
              aria-label={t("vault.unlock.field")}
              aria-invalid={!!error || undefined}
              spellCheck={false}
              disabled={throttled}
              readOnly={busy}
              value={password}
              onChange={(e) => {
                setPassword(e.target.value);
                if (error) setError(null);
              }}
            />
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

        <button type="button" className={s.forgot} onClick={() => setRecovering(true)}>
          {t("vault.unlock.forgot")}
        </button>
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
