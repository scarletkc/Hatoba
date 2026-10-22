import { useCallback, useState, type FormEvent } from "react";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { Button, TextField } from "@/components/controls";
import { Dialog, FooterSpacer, Sheet, SheetHeader, toast } from "@/components/overlay";
import { NewPasswordFields, useNewPassword } from "@/features/onboarding/PasswordStrength";
import { RecoveryCodeSection } from "@/features/onboarding/RecoveryCode";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import { cx } from "@/lib/cx";
import s from "./dialogs.module.css";

/** Asks for the master password before a sensitive action (Windows Hello or Touch ID, new recovery code). */
export function VerifyPasswordDialog({
  body,
  onVerify,
  onClose,
}: {
  body: string;
  /** Runs the action with the entered password; throw to show the error and keep the dialog open. */
  onVerify: (password: string) => Promise<void>;
  onClose: () => void;
}) {
  const t = useT();
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (!password || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onVerify(password);
      setPassword("");
      onClose();
    } catch (err) {
      setError(errorMessage(t, err));
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={t("settings.verify.title")}
      body={body}
      icon="lock-key"
      onClose={busy ? undefined : onClose}
      actions={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" busy={busy} disabled={!password} onClick={() => void submit()}>
            {t("settings.verify.submit")}
          </Button>
        </>
      }
    >
      <form className={s.verify} onSubmit={(e) => void submit(e)}>
        <TextField
          large
          secret
          autoFocus
          autoComplete="current-password"
          aria-label={t("vault.field.master")}
          invalid={!!error}
          readOnly={busy}
          value={password}
          onChange={(e) => {
            setPassword(e.target.value);
            setError(null);
          }}
        />
        <div className={cx(s.error, s.errorSlot)} role="alert">
          {error}
        </div>
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}

/** VAULT-05: change the master password. With sync on, it says that the other devices sign in again. */
export function ChangePasswordDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const syncOn = useApp((st) => st.sync?.kind !== undefined && st.sync.kind !== "none");
  const pw = useNewPassword();
  const [current, setCurrent] = useState("");
  const [busy, setBusy] = useState(false);
  const [currentError, setCurrentError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const canSubmit = !!current && pw.valid && !busy;

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setCurrentError(null);
    setError(null);
    try {
      await api.vault_change_password(current, pw.password);
      toast(t("settings.change.done"), "success");
      onClose();
    } catch (err) {
      if (toAppError(err).code === "wrong_password") setCurrentError(errorMessage(t, err));
      else setError(errorMessage(t, err));
      setBusy(false);
    }
  };

  return (
    <Sheet
      width={480}
      onClose={busy ? undefined : onClose}
      closeOnBackdrop={false}
      footer={
        <>
          <FooterSpacer />
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" busy={busy} disabled={!canSubmit} onClick={() => void submit()}>
            {t("settings.change.submit")}
          </Button>
        </>
      }
    >
      <SheetHeader title={t("settings.change.title")} subtitle={syncOn ? t("sync.password.body") : undefined} />
      <form className={s.form} onSubmit={(e) => void submit(e)}>
        <div className={s.field}>
          <label className={s.label} htmlFor="current-password">
            {t("settings.change.current")}
          </label>
          <TextField
            id="current-password"
            large
            secret
            autoFocus
            autoComplete="current-password"
            invalid={!!currentError}
            readOnly={busy}
            value={current}
            onChange={(e) => {
              setCurrent(e.target.value);
              setCurrentError(null);
            }}
          />
          {currentError && (
            <div className={s.error} role="alert">
              {currentError}
            </div>
          )}
        </div>
        <NewPasswordFields
          state={pw}
          disabled={busy}
          passwordLabel={t("settings.change.new")}
          confirmLabel={t("vault.field.confirm")}
        />
        {error && (
          <div className={s.error} role="alert">
            {error}
          </div>
        )}
        <button type="submit" hidden />
      </form>
    </Sheet>
  );
}

/** SEC-11: a PIN has at least this many characters, of any kind (as the backend checks). */
const MIN_PIN_CHARS = 4;

/** SEC-11: turns on the PIN after the master password. It starts asking for the master password after a restart. */
export function SetPinDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const [password, setPassword] = useState("");
  const [pin, setPin] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const pinLength = [...pin].length;
  const tooShort = pinLength > 0 && pinLength < MIN_PIN_CHARS;
  const mismatch = confirm.length > 0 && confirm !== pin;
  const canSubmit = !!password && pinLength >= MIN_PIN_CHARS && confirm === pin && !busy;

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setPasswordError(null);
    setError(null);
    try {
      await api.pin_enable(password, pin);
      await useApp.getState().refreshVault();
      onClose();
    } catch (err) {
      const code = toAppError(err).code;
      if (code === "wrong_password" || code === "throttled") setPasswordError(errorMessage(t, err));
      else setError(errorMessage(t, err));
      setBusy(false);
    }
  };

  return (
    <Sheet
      width={420}
      onClose={busy ? undefined : onClose}
      closeOnBackdrop={false}
      footer={
        <>
          <FooterSpacer />
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" busy={busy} disabled={!canSubmit} onClick={() => void submit()}>
            {t("settings.pin.submit")}
          </Button>
        </>
      }
    >
      <SheetHeader title={t("settings.pin.title")} subtitle={t("settings.pin.body")} />
      <form className={s.form} onSubmit={(e) => void submit(e)}>
        <div className={s.field}>
          <label className={s.label} htmlFor="pin-master-password">
            {t("vault.field.master")}
          </label>
          <TextField
            id="pin-master-password"
            large
            secret
            autoFocus
            autoComplete="current-password"
            invalid={!!passwordError}
            readOnly={busy}
            value={password}
            onChange={(e) => {
              setPassword(e.target.value);
              setPasswordError(null);
            }}
          />
          {passwordError && (
            <div className={s.error} role="alert">
              {passwordError}
            </div>
          )}
        </div>
        <div className={s.field}>
          <label className={s.label} htmlFor="pin-new">
            {t("settings.pin.pin")}
          </label>
          <TextField
            id="pin-new"
            large
            secret
            autoComplete="off"
            invalid={tooShort}
            readOnly={busy}
            value={pin}
            onChange={(e) => {
              setPin(e.target.value);
              setError(null);
            }}
          />
          {tooShort && <div className={s.error}>{t("settings.pin.tooShort")}</div>}
        </div>
        <div className={s.field}>
          <label className={s.label} htmlFor="pin-confirm">
            {t("settings.pin.confirm")}
          </label>
          <TextField
            id="pin-confirm"
            large
            secret
            autoComplete="off"
            invalid={mismatch}
            readOnly={busy}
            value={confirm}
            onChange={(e) => {
              setConfirm(e.target.value);
              setError(null);
            }}
          />
          {mismatch && <div className={s.error}>{t("settings.pin.mismatch")}</div>}
        </div>
        {error && (
          <div className={s.error} role="alert">
            {error}
          </div>
        )}
        <button type="submit" hidden />
      </form>
    </Sheet>
  );
}

/** Shows a freshly rotated recovery code. The old one is already void, so leaving needs the same confirmation as setup. */
export function NewRecoveryDialog({ code, onClose }: { code: string; onClose: () => void }) {
  const t = useT();
  const [confirmed, setConfirmed] = useState(false);
  const onConfirmed = useCallback((ok: boolean) => setConfirmed(ok), []);
  return (
    <Sheet
      closeOnBackdrop={false}
      footer={
        <>
          <FooterSpacer />
          <Button variant="primary" disabled={!confirmed} onClick={onClose}>
            {t("btn.done")}
          </Button>
        </>
      }
    >
      <SheetHeader title={t("settings.newCode.title")} subtitle={t("settings.newCode.body")} />
      <RecoveryCodeSection code={code} onConfirmedChange={onConfirmed} />
    </Sheet>
  );
}
