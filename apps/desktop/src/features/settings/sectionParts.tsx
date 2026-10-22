import type { ReactNode } from "react";
import { Icon } from "@/components/controls";
import { ListRow, StatusLine } from "@/components/patterns";
import s from "./AiPane.module.css";

/** A loading or load-failed row inside a section's group. */
export function StatusRow({ tone, action, children }: { tone: "progress" | "error"; action?: ReactNode; children: ReactNode }) {
  return (
    // A progress line is a status region itself.
    <ListRow role={tone === "error" ? "status" : undefined} trailing={action}>
      <StatusLine tone={tone}>{children}</StatusLine>
    </ListRow>
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
