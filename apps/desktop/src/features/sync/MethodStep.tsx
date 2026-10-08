import { useState } from "react";
import { Button, Icon, LinkButton } from "@/components/controls";
import { useApp } from "@/app/store";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { EncryptionDialog } from "./EncryptionDialog";
import type { Method } from "./wizardTypes";
import { WizardFrame, WizardTitle } from "./WizardFrame";
import s from "./Wizard.module.css";

/** Step 1: deploy the Worker from the app (recommended, spec §6.7), connect a deployed Worker, or D1 direct (spec §6.1). */
export function MethodStep({ method, onChange, onNext }: { method: Method; onChange: (m: Method) => void; onNext: () => void }) {
  const t = useT();
  const [explain, setExplain] = useState(false);
  const cancel = () => useApp.getState().navigate({ kind: "hosts", filter: { kind: "all" } });

  const options: { value: Method; title: string; desc: string; icon: string; recommended?: boolean }[] = [
    { value: "deploy", title: t("sync.opt.deploy"), desc: t("sync.opt.deploy.desc"), icon: "cloud-arrow-up", recommended: true },
    { value: "worker", title: t("sync.opt.worker"), desc: t("sync.opt.worker.desc"), icon: "plugs-connected" },
    { value: "d1", title: t("sync.opt.token"), desc: t("sync.opt.token.desc"), icon: "key" },
  ];

  return (
    <>
      <WizardFrame
        step={1}
        footerLead={
          <LinkButton onClick={() => setExplain(true)} aria-haspopup="dialog">
            {t("sync.howEncrypt")}
          </LinkButton>
        }
        footer={
          <>
            <Button onClick={cancel}>{t("btn.cancel")}</Button>
            <Button variant="primary" onClick={onNext}>
              {t("btn.continue")}
            </Button>
          </>
        }
      >
        <WizardTitle title={t("sync.w1.title")} body={t("sync.w1.body")} />
        <div className={s.options} role="radiogroup" aria-label={t("sync.step.method")}>
          {options.map((o) => (
            <button
              key={o.value}
              type="button"
              role="radio"
              aria-checked={method === o.value}
              className={cx(s.option, method === o.value && s.optionOn)}
              onClick={() => onChange(o.value)}
            >
              <span className={s.radio}>{method === o.value && <span className={s.radioDot} />}</span>
              <span className={s.optionText}>
                <span className={s.optionTitle}>
                  {o.title}
                  {o.recommended && <span className={s.recommended}>{t("sync.opt.recommended")}</span>}
                </span>
                <span className={s.optionDesc}>
                  {o.desc}
                </span>
              </span>
              <Icon name={o.icon} className={s.optionIcon} />
            </button>
          ))}
        </div>
      </WizardFrame>
      {explain && <EncryptionDialog onClose={() => setExplain(false)} />}
    </>
  );
}
