import type { ReactNode } from "react";
import { Icon } from "@/components/controls";
import s from "./AiPane.module.css";

/** A loading or load-failed row inside a section's group. */
export function StatusRow({ icon, action, children }: { icon: ReactNode; action?: ReactNode; children: ReactNode }) {
  return (
    <div className={s.status} role="status">
      {icon}
      <span className={s.statusText}>{children}</span>
      {action}
    </div>
  );
}

/** What a section shows when it has nothing yet: an icon, a title, a sentence, and the actions. */
export function EmptyBlock({ icon, title, body, children }: { icon: string; title: string; body: string; children?: ReactNode }) {
  return (
    <div className={s.empty}>
      <div className={s.emptyIcon}>
        <Icon name={icon} />
      </div>
      <div className={s.emptyTitle}>{title}</div>
      <div className={s.emptyBody}>{body}</div>
      {children && <div className={s.emptyActions}>{children}</div>}
    </div>
  );
}
