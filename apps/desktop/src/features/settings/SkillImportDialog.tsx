import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { errorMessage } from "@/app/errors";
import { Button, Icon, LinkButton, Spinner, TextField } from "@/components/controls";
import { Group, Section } from "@/components/layout";
import { FooterSpacer, Sheet, SheetHeader, toast } from "@/components/overlay";
import { formatBytes, useT, type MessageKey } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AppError, SkillImportPreview, SkillIssue, SkillView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import {
  SKILL_FILE_MAX_BYTES,
  byteLength,
  describeIssue,
  nameProblemKey,
  skillNameProblem,
  sourceName,
  suggestSkillName,
} from "./skillsLogic";
import s from "./SkillsSection.module.css";

type Choice = "rename" | "replace";

const SKILL_MD = "SKILL.md";
/** Up to this many files are open at first; with more, only SKILL.md is. */
const OPEN_ALL_UP_TO = 6;

/**
 * What importing a folder or `.zip` would save (AI-27), shown in full before anything is saved: every file with its
 * text, the frontmatter fields kept for export, the files skipped for not being text, and every issue, which keeps the
 * import button off. When the name is taken, the user replaces the saved skill or imports under a new name; the
 * built-in skill's name (AI-34) is never replaced, so the skill is imported under a new name.
 */
export function SkillImportDialog({
  path,
  takenNames,
  onClose,
  onImported,
}: {
  path: string;
  /** The names of the saved skills. */
  takenNames: string[];
  onClose: () => void;
  onImported: (skill: SkillView) => void;
}) {
  const t = useT();
  const ids = { form: useId(), rename: useId() };
  const [preview, setPreview] = useState<SkillImportPreview | null>(null);
  const [loadError, setLoadError] = useState<unknown>(null);
  const [choice, setChoice] = useState<Choice>("rename");
  const [newName, setNewName] = useState("");
  const [open, setOpen] = useState<Set<string>>(() => new Set());
  const [importing, setImporting] = useState(false);
  const [rejected, setRejected] = useState<AppError | null>(null);
  const live = useRef(true);

  const load = async () => {
    setLoadError(null);
    setPreview(null);
    try {
      const got = await api.skill_import_preview(path);
      if (!live.current) return;
      setPreview(got);
      setNewName(got.name ? suggestSkillName(got.name, takenNames) : "");
      setOpen(new Set(got.files.length + 1 <= OPEN_ALL_UP_TO ? [SKILL_MD, ...got.files.map((f) => f.path)] : [SKILL_MD]));
    } catch (e) {
      if (live.current) setLoadError(e);
    }
  };

  useEffect(() => {
    live.current = true;
    void load();
    return () => {
      live.current = false;
    };
  }, [path]);

  const reserved = !!preview && preview.reserved_name && preview.issues.length === 0;
  const clash = !!preview && !reserved && preview.existing_id !== null && preview.issues.length === 0;
  const renameProblem = skillNameProblem(newName.trim(), takenNames);
  const renaming = reserved || (clash && choice === "rename");
  const canImport = !!preview && preview.issues.length === 0 && !importing && (!renaming || !renameProblem);

  const allPaths = preview ? [SKILL_MD, ...preview.files.map((f) => f.path)] : [];
  const allOpen = allPaths.length > 0 && allPaths.every((p) => open.has(p));
  const toggle = (p: string) =>
    setOpen((o) => {
      const next = new Set(o);
      if (!next.delete(p)) next.add(p);
      return next;
    });

  const doImport = async () => {
    if (!preview || !canImport) return;
    setImporting(true);
    setRejected(null);
    try {
      const replacing = clash && choice === "replace";
      // The preview's token: Rust saves the files only if they still read as shown here (AI-27).
      const skill = await api.skill_import(path, preview.token, replacing ? preview.existing_id : null, renaming ? newName.trim() : null);
      toast(t(replacing ? "aiSettings.skills.imp.replaced" : "aiSettings.skills.imp.done", { name: skill.name }), "success");
      onImported(skill);
      onClose();
    } catch (e) {
      if (!live.current) return;
      setRejected(toAppError(e));
      setImporting(false);
    }
  };

  const issueText = (issue: SkillIssue): string => {
    const { kind, params } = describeIssue(issue);
    const shown: Record<string, string | number> = { ...params };
    if (kind === "file_too_large" || kind === "too_large") {
      for (const k of ["size", "bytes", "max"]) if (typeof shown[k] === "number") shown[k] = formatBytes(shown[k]);
    }
    return t(`aiSettings.skills.issue.${kind}` as MessageKey, shown);
  };

  const renameError = renaming && renameProblem ? t(`aiSettings.skills.err.${nameProblemKey(renameProblem)}`) : null;
  const renameField = (
    <div className={cx(s.stack, s.rename)}>
      <TextField
        id={ids.rename}
        mono
        autoFocus
        aria-label={t("aiSettings.skills.imp.newName")}
        value={newName}
        invalid={!!renameError}
        onChange={(e) => setNewName(e.target.value)}
      />
      {renameProblem !== "invalid" && <span className={s.hint}>{t("aiSettings.skills.f.nameHint")}</span>}
      {renameError && (
        <div className={s.problem} role="alert">
          {renameError}
        </div>
      )}
    </div>
  );

  return (
    <Sheet
      width={720}
      onClose={importing ? undefined : onClose}
      closeOnBackdrop={false}
      footer={
        <>
          <FooterSpacer />
          <Button onClick={onClose} disabled={importing}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={ids.form} busy={importing} disabled={!canImport}>
            {clash && choice === "replace" ? t("aiSettings.skills.imp.submitReplace") : t("aiSettings.skills.imp.submit")}
          </Button>
        </>
      }
    >
      <SheetHeader title={t("aiSettings.skills.imp.title")} subtitle={t("aiSettings.skills.imp.subtitle")} />

      {!preview && !loadError && (
        <div className={s.loading} role="status">
          <Spinner />
          {t("aiSettings.skills.imp.loading", { source: sourceName(path) })}
        </div>
      )}
      {!!loadError && (
        <div className={s.loadFailed} role="alert">
          <div className={s.problem}>
            {t("aiSettings.skills.imp.failed")} {errorMessage(t, loadError)}
          </div>
          <Button size="sm" onClick={() => void load()}>
            {t("btn.retry")}
          </Button>
        </div>
      )}

      {preview && (
        <form
          id={ids.form}
          className={s.form}
          onSubmit={(e) => {
            e.preventDefault();
            void doImport();
          }}
        >
          {preview.issues.length > 0 && (
            <div className={s.issues} role="alert">
              <Icon name="warning-circle" />
              <div>
                <div className={s.issuesTitle}>{t("aiSettings.skills.imp.issuesTitle")}</div>
                <div className={s.note}>{t("aiSettings.skills.imp.issuesBody")}</div>
                <ul className={cx(s.issuesList, "selectable")}>
                  {preview.issues.map((issue, i) => (
                    <li key={i}>{issueText(issue)}</li>
                  ))}
                </ul>
              </div>
            </div>
          )}

          {(preview.name !== null || preview.description !== null) && (
            <Group>
              <div className={s.summary}>
                <span className={cx(s.summaryName, "selectable")}>{preview.name || "—"}</span>
                <span className={cx(s.summaryDescription, "selectable")}>{preview.description || "—"}</span>
              </div>
            </Group>
          )}

          {reserved && preview.name && (
            <Group>
              <div className={s.clash}>
                <div className={s.clashTitle}>{t("aiSettings.skills.imp.reservedTitle", { name: preview.name })}</div>
                {renameField}
              </div>
            </Group>
          )}

          {clash && preview.name && (
            <Group>
              <div className={s.clash} role="radiogroup" aria-label={t("aiSettings.skills.imp.clashTitle", { name: preview.name })}>
                <div className={s.clashTitle}>{t("aiSettings.skills.imp.clashTitle", { name: preview.name })}</div>
                <Radio checked={choice === "rename"} onSelect={() => setChoice("rename")}>
                  <span className={s.choiceLabel}>{t("aiSettings.skills.imp.rename")}</span>
                </Radio>
                {choice === "rename" && renameField}
                <Radio checked={choice === "replace"} onSelect={() => setChoice("replace")}>
                  <span className={s.choice}>
                    <span className={s.choiceLabel}>{t("aiSettings.skills.imp.replace")}</span>
                    <span className={s.hint}>{t("aiSettings.skills.imp.replaceHint")}</span>
                  </span>
                </Radio>
              </div>
            </Group>
          )}

          {preview.body !== null && (
            <Section title={t("aiSettings.skills.imp.files")}>
              <div className={s.blockBar}>
                <span className={s.hint}>{t("aiSettings.skills.fileCount", { n: preview.files.length + 1 })}</span>
                <LinkButton onClick={() => setOpen(new Set(allOpen ? [] : allPaths))}>
                  {allOpen ? t("aiSettings.skills.imp.collapseAll") : t("aiSettings.skills.imp.expandAll")}
                </LinkButton>
              </div>
              <Group>
                <ContentBlock path={SKILL_MD} content={preview.body} open={open.has(SKILL_MD)} onToggle={() => toggle(SKILL_MD)} />
                {preview.files.map((f) => (
                  <ContentBlock key={f.path} path={f.path} content={f.content} open={open.has(f.path)} onToggle={() => toggle(f.path)} />
                ))}
              </Group>
            </Section>
          )}

          {preview.frontmatter_keys.length > 0 && (
            <div className={s.note}>{t("aiSettings.skills.imp.frontmatter", { keys: preview.frontmatter_keys.join(", ") })}</div>
          )}

          {preview.skipped.length > 0 && (
            <Section title={t("aiSettings.skills.imp.skipped")}>
              <div className={s.hint}>{t("aiSettings.skills.imp.skippedBody")}</div>
              <Group>
                <ul className={cx(s.skippedList, "selectable")}>
                  {preview.skipped.map((p) => (
                    <li key={p}>{p}</li>
                  ))}
                </ul>
              </Group>
            </Section>
          )}

          {rejected && (
            <div className={s.problem} role="alert">
              {errorMessage(t, rejected)}
              {rejected.detail && <div className={cx(s.detail, "selectable")}>{rejected.detail}</div>}
            </div>
          )}
        </form>
      )}
    </Sheet>
  );
}

/** One file of a skill shown read-only: a row with its path and size, and its whole text when open. */
export function ContentBlock({ path, content, open, onToggle }: { path: string; content: string; open: boolean; onToggle: () => void }) {
  const t = useT();
  const bytes = byteLength(content);
  return (
    <div className={cx(s.block, open && s.blockOpen)}>
      <button type="button" className={s.blockHead} aria-expanded={open} aria-label={t("aiSettings.skills.file.toggle", { path })} onClick={onToggle}>
        <span className={s.fileToggle} aria-hidden>
          <Icon name="caret-right" />
        </span>
        <span className={s.blockPath}>{path}</span>
        <span className={cx(s.size, bytes > SKILL_FILE_MAX_BYTES && s.counterOver)}>{formatBytes(bytes)}</span>
      </button>
      {open && (
        <pre className={cx(s.content, "selectable", content === "" && s.contentEmpty)} tabIndex={0}>
          {content === "" ? t("aiSettings.skills.imp.noContent") : content}
        </pre>
      )}
    </div>
  );
}

function Radio({ checked, onSelect, children }: { checked: boolean; onSelect: () => void; children: ReactNode }) {
  return (
    <button type="button" role="radio" aria-checked={checked} className={s.radio} onClick={onSelect}>
      <span className={cx(s.radioDot, checked && s.radioDotOn)} aria-hidden />
      {children}
    </button>
  );
}
