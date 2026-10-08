import { useState } from "react";
import { Button, LinkButton } from "@/components/controls";
import { Group, Section } from "@/components/layout";
import { FooterSpacer, Sheet, SheetHeader } from "@/components/overlay";
import { useT } from "@/i18n";
import type { BuiltinSkillView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { ContentBlock } from "./SkillImportDialog";
import s from "./SkillsSection.module.css";

const SKILL_MD = "SKILL.md";

/** AI-34: the built-in `hatoba` skill, read-only: its description, SKILL.md and every other file. */
export function BuiltinSkillDialog({ skill, onClose }: { skill: BuiltinSkillView; onClose: () => void }) {
  const t = useT();
  const [open, setOpen] = useState<Set<string>>(() => new Set([SKILL_MD]));
  const allPaths = [SKILL_MD, ...skill.files.map((f) => f.path)];
  const allOpen = allPaths.every((p) => open.has(p));
  const toggle = (p: string) =>
    setOpen((o) => {
      const next = new Set(o);
      if (!next.delete(p)) next.add(p);
      return next;
    });

  return (
    <Sheet
      width={720}
      onClose={onClose}
      footer={
        <>
          <FooterSpacer />
          <Button variant="primary" onClick={onClose}>
            {t("btn.close")}
          </Button>
        </>
      }
    >
      <SheetHeader title={t("aiSettings.skills.builtin.title")} subtitle={t("aiSettings.skills.builtin.subtitle")} />
      <div className={s.form}>
        <Group>
          <div className={s.summary}>
            <span className={cx(s.summaryName, "selectable")}>{skill.name}</span>
            <span className={cx(s.summaryDescription, "selectable")}>{skill.description}</span>
          </div>
        </Group>
        <Section title={t("aiSettings.skills.files")}>
          <div className={s.blockBar}>
            <span className={s.hint}>{t("aiSettings.skills.fileCount", { n: allPaths.length })}</span>
            <LinkButton onClick={() => setOpen(new Set(allOpen ? [] : allPaths))}>
              {allOpen ? t("aiSettings.skills.imp.collapseAll") : t("aiSettings.skills.imp.expandAll")}
            </LinkButton>
          </div>
          <Group>
            <ContentBlock path={SKILL_MD} content={skill.body} open={open.has(SKILL_MD)} onToggle={() => toggle(SKILL_MD)} />
            {skill.files.map((f) => (
              <ContentBlock key={f.path} path={f.path} content={f.content} open={open.has(f.path)} onToggle={() => toggle(f.path)} />
            ))}
          </Group>
        </Section>
      </div>
    </Sheet>
  );
}
