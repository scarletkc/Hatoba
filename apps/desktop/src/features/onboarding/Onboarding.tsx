import { useState } from "react";
import { WindowChrome } from "@/app/TitleBar";
import { AppLogo, Button, Icon } from "@/components/controls";
import { FooterSpacer } from "@/components/overlay";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { CreatePassword, CreateRecovery } from "./CreateVault";
import { RestoreConnect, RestorePassword } from "./RestoreVault";
import type { SyncConfigInput } from "@/ipc/types";
import { WizardCard } from "./WizardCard";
import s from "./Onboarding.module.css";

type Flow = "create" | "restore";
type Step = "choose" | "password" | "recovery" | "connect" | "restorePassword";

/** First launch (spec §6.6 flows A/B, VAULT-01/02): create a new vault or restore one from the cloud. */
export function Onboarding() {
  const t = useT();
  const [flow, setFlow] = useState<Flow>("create");
  const [step, setStep] = useState<Step>("choose");
  const [code, setCode] = useState("");
  const [config, setConfig] = useState<SyncConfigInput | null>(null);

  const stepLabels =
    flow === "create"
      ? [t("vault.step.start"), t("vault.step.password"), t("vault.step.recovery")]
      : [t("vault.step.start"), t("vault.step.connect"), t("vault.step.password")];

  return (
    <div className={s.root}>
      <WindowChrome />
      <div className={s.center}>
        <AppLogo className={s.logo} />
        {step === "choose" && (
          <WizardCard
            steps={stepLabels}
            current={1}
            title={t("vault.welcome.title")}
            body={t("vault.welcome.body")}
            onSubmit={() => setStep(flow === "create" ? "password" : "connect")}
            footer={
              <>
                <FooterSpacer />
                <Button variant="primary" type="submit" autoFocus>
                  {t("btn.continue")}
                </Button>
              </>
            }
          >
            <div className={s.options} role="radiogroup" aria-label={t("vault.welcome.title")}>
              <OptionCard
                selected={flow === "create"}
                icon="lock-key"
                title={t("vault.opt.create")}
                body={t("vault.opt.createDesc")}
                onSelect={() => setFlow("create")}
              />
              <OptionCard
                selected={flow === "restore"}
                icon="cloud-arrow-down"
                title={t("vault.opt.restore")}
                body={t("vault.opt.restoreDesc")}
                onSelect={() => setFlow("restore")}
              />
            </div>
          </WizardCard>
        )}
        {step === "password" && (
          <CreatePassword
            steps={stepLabels}
            onBack={() => setStep("choose")}
            onCreated={(recoveryCode) => {
              setCode(recoveryCode);
              setStep("recovery");
            }}
          />
        )}
        {step === "recovery" && <CreateRecovery steps={stepLabels} code={code} />}
        {step === "connect" && (
          <RestoreConnect
            steps={stepLabels}
            onBack={() => setStep("choose")}
            onConnected={(cfg) => {
              setConfig(cfg);
              setStep("restorePassword");
            }}
          />
        )}
        {step === "restorePassword" && config && (
          <RestorePassword steps={stepLabels} config={config} onBack={() => setStep("connect")} />
        )}
      </div>
    </div>
  );
}

/** Radio card styled like the design's sync-wizard method picker. */
function OptionCard({
  selected,
  icon,
  title,
  body,
  onSelect,
}: {
  selected: boolean;
  icon: string;
  title: string;
  body: string;
  onSelect: () => void;
}) {
  return (
    <button type="button" role="radio" aria-checked={selected} className={cx(s.option, selected && s.optionOn)} onClick={onSelect}>
      <span className={cx(s.radio, selected && s.radioOn)}>{selected && <span className={s.radioDot} />}</span>
      <span className={s.optionText}>
        <span className={s.optionTitle}>{title}</span>
        <span className={s.optionBody}>{body}</span>
      </span>
      <Icon name={icon} className={s.optionIcon} />
    </button>
  );
}
