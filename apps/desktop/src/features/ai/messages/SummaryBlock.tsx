import { useEffect, useState } from "react";
import { Card, DisclosureCaret, Icon, controlStyles } from "@hatoba/ui";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { Markdown } from "./Markdown";
import s from "./Messages.module.css";

export function SummaryBlock({ text, flash }: { text: string; flash?: boolean }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (flash) setOpen(true);
  }, [flash]);
  return (
    <Card className={cx(s.summary, flash && s.flash)}>
      <button type="button" className={s.summaryHead} aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <Icon name="arrows-in-line-vertical" size={14} />
        <span>{t("ai.summary")}</span>
        <DisclosureCaret open={open} size={12} />
      </button>
      {open && (
        <div className={cx(s.summaryBody, controlStyles.reveal)}>
          <Markdown text={text} />
        </div>
      )}
    </Card>
  );
}
