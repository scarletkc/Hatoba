import { useCallback, useState } from "react";
import { enterUnlocked } from "@/app/boot";
import { errorMessage } from "@/app/errors";
import { Button } from "@/components/controls";
import { Callout } from "@/components/layout";
import { FooterSpacer } from "@/components/overlay";
import { ErrorText } from "@/components/patterns";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import { NewPasswordFields, useNewPassword } from "./PasswordStrength";
import { RecoveryCodeSection } from "./RecoveryCode";
import { WizardCard } from "./WizardCard";

/** Flow A step 2: master password with strength meter (VAULT-01, SEC-09). */
export function CreatePassword({
  steps,
  onBack,
  onCreated,
}: {
  steps: string[];
  onBack: () => void;
  onCreated: (recoveryCode: string) => void;
}) {
  const t = useT();
  const pw = useNewPassword();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!pw.valid || busy) return;
    setBusy(true);
    setError(null);
    try {
      const code = await api.vault_create(pw.password);
      pw.setPassword("");
      pw.setConfirm("");
      onCreated(code);
    } catch (e) {
      setError(errorMessage(t, e));
      setBusy(false);
    }
  };

  return (
    <WizardCard
      steps={steps}
      current={2}
      title={t("vault.create.title")}
      onSubmit={() => void submit()}
      footer={
        <>
          {busy && <span style={{ fontSize: 12, color: "var(--fg2)" }}>{t("vault.create.busy")}</span>}
          <FooterSpacer />
          <Button onClick={onBack} disabled={busy}>
            {t("btn.back")}
          </Button>
          <Button variant="primary" type="submit" busy={busy} disabled={!pw.valid}>
            {t("vault.create.submit")}
          </Button>
        </>
      }
    >
      <NewPasswordFields
        state={pw}
        autoFocus
        disabled={busy}
        passwordLabel={t("vault.field.master")}
        confirmLabel={t("vault.field.confirm")}
      />
      <Callout icon="info" iconColor="var(--orange)" title={t("vault.create.lossTitle")}>
        {t("vault.create.lossBody")}
      </Callout>
      {error && <ErrorText icon="warning-circle">{error}</ErrorText>}
    </WizardCard>
  );
}

/** Flow A step 3: show the recovery code once and require confirmation before finishing (VAULT-02). */
export function CreateRecovery({ steps, code }: { steps: string[]; code: string }) {
  const t = useT();
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const onConfirmed = useCallback((ok: boolean) => setConfirmed(ok), []);

  const finish = async () => {
    if (!confirmed || busy) return;
    setBusy(true);
    await enterUnlocked();
  };

  return (
    <WizardCard
      steps={steps}
      current={3}
      title={t("vault.code.title")}
      body={t("vault.code.body")}
      onSubmit={() => void finish()}
      footer={
        <>
          <FooterSpacer />
          <Button variant="primary" type="submit" busy={busy} disabled={!confirmed}>
            {t("vault.code.finish")}
          </Button>
        </>
      }
    >
      <RecoveryCodeSection code={code} onConfirmedChange={onConfirmed} />
    </WizardCard>
  );
}
