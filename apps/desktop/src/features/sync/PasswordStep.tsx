import { useEffect, useId, useRef, useState } from "react";
import { Button, Spinner, TextField } from "@/components/controls";
import { Callout } from "@/components/layout";
import { toast } from "@/components/overlay";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { Field } from "@/features/keys/Field";
import { useT } from "@/i18n";
import { refreshSyncStatus } from "./syncUtils";
import { WizardFrame, WizardTitle } from "./WizardFrame";
import s from "./Wizard.module.css";

/**
 * Step 3. The vault already exists locally (created at first launch), so this asks for the existing
 * master password to authorise the upload; it does not set a new one. `submit` initialises the
 * remote: `sync_configure`, or `deploy_setup` after an in-app deployment.
 */
export function PasswordStep({ submit, onBack }: { submit: (password: string) => Promise<void>; onBack: () => void }) {
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
            {itemCount > 0 ? t("sync.uploading", { n: itemCount }) : t("sync.initializing")}
          </>
        )
      }
      footer={
        <>
          <Button onClick={onBack} disabled={busy}>
            {t("btn.back")}
          </Button>
          <Button variant="primary" busy={busy} disabled={!password} onClick={() => void finish()}>
            {t("sync.finish")}
          </Button>
        </>
      }
    >
      <WizardTitle title={t("sync.w3.title")} body={t("sync.w3.body")} />
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
          onKeyDown={(e) => e.key === "Enter" && void finish()}
        />
      </Field>
      <Callout icon="info" iconColor="var(--orange)" title={t("sync.loss.title")}>
        <span className={s.lossBody}>{t("sync.loss.body")}</span>
      </Callout>
    </WizardFrame>
  );
}
