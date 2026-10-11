import { useEffect, useId, useRef, useState } from "react";
import { Button, Spinner, TextField } from "@/components/controls";
import { toast } from "@/components/overlay";
import { useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import { errorMessage } from "@/app/errors";
import { Field } from "@/features/keys/Field";
import { useT } from "@/i18n";
import { isImeEvent } from "@/lib/ime";
import { refreshSyncStatus } from "./syncUtils";
import { WizardFrame, WizardTitle } from "./WizardFrame";

/**
 * Step 3. The vault already exists locally (created at first launch), so this asks for the existing
 * master password to authorise the upload; it does not set a new one. `submit` initialises the
 * remote: `sync_configure`, or `deploy_setup` after an in-app deployment.
 *
 * With `join` the remote already holds a vault, so this asks for that vault's master password and
 * `submit` merges this device into it (`sync_join` or `deploy_join`, flow C).
 */
export function PasswordStep({ submit, join, onBack }: { submit: (password: string) => Promise<void>; join: boolean; onBack: () => void }) {
  const t = useT();
  const fieldId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const itemCount = useVaultData((st) => st.hosts.length + st.keys.length + st.groups.length);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const finish = async () => {
    if (!password || busy) return;
    setBusy(true);
    setError(null);
    try {
      await submit(password);
      await refreshSyncStatus();
      // Joining another vault turns off biometric unlock, which held the old vault key.
      if (join) await useApp.getState().refreshVault();
      toast(t("sync.enabled"), "success");
    } catch (e) {
      setError(errorMessage(t, e));
      setBusy(false);
      inputRef.current?.focus();
    }
  };

  return (
    <WizardFrame
      step={3}
      footerLead={
        busy && (
          <>
            <Spinner />
            {join ? t("sync.join.busy") : itemCount > 0 ? t("sync.uploading", { n: itemCount }) : t("sync.initializing")}
          </>
        )
      }
      footer={
        <>
          <Button onClick={onBack} disabled={busy}>
            {t("btn.back")}
          </Button>
          <Button variant="primary" busy={busy} disabled={!password} onClick={() => void finish()}>
            {t(join ? "sync.join.submit" : "sync.finish")}
          </Button>
        </>
      }
    >
      <WizardTitle title={t(join ? "sync.join.title" : "sync.w3.title")} body={t(join ? "sync.join.body" : "sync.w3.body")} />
      <Field label={t("sync.masterPassword")} htmlFor={fieldId} error={error}>
        <TextField
          id={fieldId}
          ref={inputRef}
          large
          secret
          value={password}
          disabled={busy}
          invalid={!!error}
          autoComplete="current-password"
          onChange={(e) => {
            setPassword(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && void finish()}
        />
      </Field>
    </WizardFrame>
  );
}
