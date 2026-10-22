import type { FormEvent, ReactNode } from "react";
import { Icon } from "@/components/controls";
import { cx } from "@/lib/cx";
import s from "./WizardCard.module.css";

/** Step indicator from the design's sync wizard: ✓ done · filled current · outlined upcoming. */
export function Steps({ labels, current }: { labels: string[]; current: number }) {
  return (
    <ol className={s.steps}>
      {labels.map((label, i) => {
        const n = i + 1;
        const done = n < current;
        return (
          <li key={label} className={cx(s.step, n === current && s.stepCurrent)} aria-current={n === current ? "step" : undefined}>
            {i > 0 && <span className={s.rule} />}
            <span className={cx(s.bubble, done && s.bubbleDone, n === current && s.bubbleCurrent)}>
              {done ? <Icon name="check" /> : n}
            </span>
            {label}
          </li>
        );
      })}
    </ol>
  );
}

/** The 540px card used by every first-launch step (mirrors the design's wizard sheets). */
export function WizardCard({
  steps,
  current,
  title,
  body,
  footer,
  onSubmit,
  children,
}: {
  steps: string[];
  current: number;
  title: string;
  body?: string;
  footer: ReactNode;
  onSubmit: () => void;
  children: ReactNode;
}) {
  return (
    <form
      className={s.card}
      onSubmit={(e: FormEvent) => {
        e.preventDefault();
        onSubmit();
      }}
    >
      <div className={s.body}>
        <Steps labels={steps} current={current} />
        <div>
          <h1 className={s.title}>{title}</h1>
          {body && <div className={s.subtitle}>{body}</div>}
        </div>
        {children}
      </div>
      <div className={s.footer}>{footer}</div>
    </form>
  );
}
