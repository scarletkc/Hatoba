import type { ReactNode } from "react";
import { Icon } from "@hatoba/ui";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import s from "./Wizard.module.css";

export type WizardStep = 1 | 2 | 3;

/** The 540px card with the 3-step progress header, shown inline in the page. */
export function WizardFrame({
  step,
  footer,
  footerLead,
  children,
}: {
  step: WizardStep;
  footer: ReactNode;
  /** Left-aligned footer content (e.g. the "How is my data encrypted?" link or upload progress). */
  footerLead?: ReactNode;
  children: ReactNode;
}) {
  const t = useT();
  const labels = [t("sync.step.method"), t("sync.step.connect"), t("sync.step.password")];
  return (
    <div className={s.stage}>
      <div className={s.card} role="group" aria-label={t("sidebar.sync")}>
        <div className={s.body}>
          <ol className={s.steps} aria-label={t("sync.wizard.steps")}>
            {labels.map((label, i) => {
              const n = (i + 1) as WizardStep;
              const state = n < step ? "done" : n === step ? "current" : "todo";
              return (
                <li key={n} className={s.stepItem} aria-current={state === "current" ? "step" : undefined}>
                  {i > 0 && <span className={s.stepLine} />}
                  <span className={cx(s.step, state === "current" && s.stepCurrent)}>
                    <span className={cx(s.stepDot, s[`stepDot_${state}`])}>
                      {state === "done" ? <Icon name="check" /> : n}
                    </span>
                    {label}
                  </span>
                </li>
              );
            })}
          </ol>
          {children}
        </div>
        <div className={s.footer}>
          {footerLead && <span className={s.footerLead}>{footerLead}</span>}
          <span className={s.footerSpacer} />
          {footer}
        </div>
      </div>
    </div>
  );
}

export function WizardTitle({ title, body }: { title: ReactNode; body?: ReactNode }) {
  return (
    <div>
      <h2 className={s.title}>{title}</h2>
      {body && <p className={s.subtitle}>{body}</p>}
    </div>
  );
}
