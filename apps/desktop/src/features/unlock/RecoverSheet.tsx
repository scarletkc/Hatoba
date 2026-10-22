import { useState, type FormEvent } from "react";
import { enterUnlocked } from "@/app/boot";
import { errorMessage } from "@/app/errors";
import { Button, TextField } from "@/components/controls";
import { FooterSpacer, Sheet, SheetHeader } from "@/components/overlay";
import { ErrorText, HintText } from "@/components/patterns";
import { NewPasswordFields, useNewPassword } from "@/features/onboarding/PasswordStrength";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import s from "./RecoverSheet.module.css";

/** "ab12 cd34" → "AB12-CD34": accepts dashes and spaces, uppercases, regroups by four. */
function formatCode(raw: string): string {
  const clean = raw.toUpperCase().replace(/[^0-9A-Z]/g, "").slice(0, 40);
  return clean.match(/.{1,4}/g)?.join("-") ?? "";
}

/** VAULT-06: reset the master password with the recovery code. */
export function RecoverSheet({ onClose }: { onClose: () => void }) {
  const t = useT();
  const pw = useNewPassword();
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [codeError, setCodeError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const codeOk = code.replace(/-/g, "").length >= 16;
  const canSubmit = codeOk && pw.valid && !busy;

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setCodeError(null);
    setError(null);
    try {
      await api.vault_recover(code, pw.password);
      pw.setPassword("");
      pw.setConfirm("");
      await enterUnlocked();
    } catch (err) {
      if (toAppError(err).code === "wrong_recovery_code") setCodeError(errorMessage(t, err));
      else setError(errorMessage(t, err));
      setBusy(false);
    }
  };

  return (
    <Sheet
      onClose={busy ? undefined : onClose}
      closeOnBackdrop={false}
      // Opened from the lock screen, where every other dialog stays hidden.
      whileLocked
      footer={
        <>
          <FooterSpacer />
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" busy={busy} disabled={!canSubmit} onClick={() => void submit()}>
            {t("vault.recover.submit")}
          </Button>
        </>
      }
    >
      <SheetHeader title={t("vault.recover.title")} subtitle={t("vault.recover.subtitle")} />
      <form className={s.form} onSubmit={(e) => void submit(e)}>
        <div className={s.field}>
          <label className={s.label} htmlFor="recovery-code">
            {t("vault.recover.code")}
          </label>
          <TextField
            id="recovery-code"
            large
            mono
            autoFocus
            autoComplete="off"
            invalid={!!codeError}
            placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX"
            value={code}
            onChange={(e) => {
              setCode(formatCode(e.target.value));
              setCodeError(null);
            }}
          />
          {codeError ? <ErrorText>{codeError}</ErrorText> : <HintText>{t("vault.recover.codeHint")}</HintText>}
        </div>
        <NewPasswordFields
          state={pw}
          disabled={busy}
          passwordLabel={t("vault.recover.newPassword")}
          confirmLabel={t("vault.field.confirm")}
        />
        {error && <ErrorText>{error}</ErrorText>}
        {/* Lets Enter submit from any field. */}
        <button type="submit" hidden />
      </form>
    </Sheet>
  );
}
