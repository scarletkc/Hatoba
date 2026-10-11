import { useEffect, useId, useRef, useState } from "react";
import { Button, TextField } from "@/components/controls";
import { Dialog, toast } from "@/components/overlay";
import { errorMessage } from "@/app/errors";
import { Field } from "@/features/keys/Field";
import { MIN_PASSWORD_LENGTH } from "@/features/onboarding/PasswordStrength";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import s from "./Dialogs.module.css";

export function ChangePasswordDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const currentId = useId();
  const nextId = useId();
  const confirmId = useId();
  const formId = useId();
  const currentRef = useRef<HTMLInputElement>(null);
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ field: "current" | "next" | "confirm"; message: string } | null>(null);

  useEffect(() => {
    if (error?.field === "current") currentRef.current?.focus();
  }, [error]);

  const submit = async () => {
    if (busy) return;
    if (next.length < MIN_PASSWORD_LENGTH) return setError({ field: "next", message: t("sync.password.short") });
    if (next !== confirmation) return setError({ field: "confirm", message: t("sync.password.mismatch") });
    setBusy(true);
    setError(null);
    try {
      await api.vault_change_password(current, next);
      toast(t("sync.password.done"), "success");
      onClose();
    } catch (e) {
      const wrong = (e as { code?: string })?.code === "wrong_password";
      setError({ field: wrong ? "current" : "next", message: errorMessage(t, e) });
      setBusy(false);
    }
  };

  const clear = <T,>(set: (v: T) => void) => (v: T) => {
    set(v);
    setError(null);
  };

  return (
    <Dialog
      title={t("sync.password.title")}
      icon="lock-key"
      body={t("sync.password.body")}
      onClose={busy ? undefined : onClose}
      actions={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={formId} busy={busy} disabled={!current || !next || !confirmation}>
            {t("btn.save")}
          </Button>
        </>
      }
    >
      <form
        id={formId}
        className={s.form}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Field label={t("sync.password.current")} htmlFor={currentId} error={error?.field === "current" ? error.message : undefined}>
          <TextField
            id={currentId}
            ref={currentRef}
            secret
            autoFocus
            value={current}
            invalid={error?.field === "current"}
            autoComplete="current-password"
            onChange={(e) => clear(setCurrent)(e.target.value)}
          />
        </Field>
        <Field label={t("sync.password.new")} htmlFor={nextId} error={error?.field === "next" ? error.message : undefined}>
          <TextField
            id={nextId}
            secret
            value={next}
            invalid={error?.field === "next"}
            autoComplete="new-password"
            onChange={(e) => clear(setNext)(e.target.value)}
          />
        </Field>
        <Field label={t("sync.password.confirm")} htmlFor={confirmId} error={error?.field === "confirm" ? error.message : undefined}>
          <TextField
            id={confirmId}
            secret
            value={confirmation}
            invalid={error?.field === "confirm"}
            autoComplete="new-password"
            onChange={(e) => clear(setConfirmation)(e.target.value)}
          />
        </Field>
      </form>
    </Dialog>
  );
}
