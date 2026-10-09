import { useEffect, useId, useRef, useState } from "react";
import { errorMessage } from "@/app/errors";
import { Button, Icon, IconButton, Spinner, TextArea, TextField } from "@/components/controls";
import { FormRow, Group, Section } from "@/components/layout";
import { FooterSpacer, Sheet, SheetHeader, confirm } from "@/components/overlay";
import { formatBytes, useT, type MessageKey } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AppError, SkillDetail, SkillView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import {
  SKILL_DESCRIPTION_MAX,
  SKILL_FILE_MAX_BYTES,
  SKILL_MAX_FILES,
  byteLength,
  descriptionLength,
  draftFromDetail,
  emptyDraft,
  hasProblems,
  nameProblemKey,
  newUid,
  sameDraft,
  toSkillInput,
  validateSkill,
  type SkillDraft,
  type SkillFileDraft,
} from "./skillsLogic";
import s from "./SkillsSection.module.css";

/** Create or edit one skill (AI-27). Saved straight to the backend, not with the pane. */
export function SkillDialog({
  id,
  takenNames,
  onClose,
  onSaved,
}: {
  /** null creates a skill. */
  id: string | null;
  /** The names of the other saved skills. */
  takenNames: string[];
  onClose: () => void;
  onSaved: (saved: SkillView) => void;
}) {
  const t = useT();
  const ids = { form: useId(), name: useId(), description: useId(), body: useId() };
  const [detail, setDetail] = useState<SkillDetail | null>(null);
  const [loadError, setLoadError] = useState<unknown>(null);
  const [draft, setDraft] = useState<SkillDraft>(emptyDraft);
  const [initial, setInitial] = useState<SkillDraft>(emptyDraft);
  /** Files whose text is shown; a new file starts open. */
  const [open, setOpen] = useState<Set<number>>(() => new Set());
  const [focusUid, setFocusUid] = useState<number | null>(null);
  const [submitted, setSubmitted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [asking, setAsking] = useState(false);
  const [rejected, setRejected] = useState<AppError | null>(null);
  const live = useRef(true);

  const loading = id !== null && !detail && !loadError;

  const load = async () => {
    if (id === null) return;
    setLoadError(null);
    try {
      const got = await api.skill_get(id);
      if (!live.current) return;
      const next = draftFromDetail(got);
      setDetail(got);
      setDraft(next);
      setInitial(next);
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  const problems = validateSkill(draft, takenNames);
  const dirty = !sameDraft(draft, initial);
  const busy = saving || asking;

  const patch = (p: Partial<SkillDraft>) => {
    setDraft((d) => ({ ...d, ...p }));
    setRejected(null);
  };
  const patchFile = (uid: number, p: Partial<SkillFileDraft>) =>
    patch({ files: draft.files.map((f) => (f.uid === uid ? { ...f, ...p } : f)) });

  const addFile = () => {
    const uid = newUid();
    patch({ files: [...draft.files, { uid, path: "", content: "" }] });
    setOpen((o) => new Set(o).add(uid));
    setFocusUid(uid);
  };
  const removeFile = (uid: number) => patch({ files: draft.files.filter((f) => f.uid !== uid) });
  const toggleFile = (uid: number) =>
    setOpen((o) => {
      const next = new Set(o);
      if (!next.delete(uid)) next.add(uid);
      return next;
    });

  const requestClose = async () => {
    if (busy) return;
    if (dirty) {
      setAsking(true);
      const discard = await confirm({
        title: t("aiSettings.skills.discard.title"),
        body: t("aiSettings.skills.discard.body"),
        confirmLabel: t("aiSettings.skills.discard.confirm"),
        cancelLabel: t("aiSettings.skills.discard.keep"),
        danger: true,
      });
      if (!live.current) return;
      setAsking(false);
      if (!discard) return;
    }
    onClose();
  };

  const save = async () => {
    if (busy || loading || loadError) return;
    setSubmitted(true);
    if (hasProblems(problems)) {
      const firstFile = problems.files.findIndex(Boolean);
      if (problems.name) document.getElementById(ids.name)?.focus();
      else if (problems.description) document.getElementById(ids.description)?.focus();
      else if (problems.body) document.getElementById(ids.body)?.focus();
      else if (firstFile >= 0) setOpen((o) => new Set(o).add(draft.files[firstFile].uid));
      return;
    }
    setSaving(true);
    try {
      const saved = await api.skill_save(toSkillInput(id, detail?.skill.enabled ?? true, draft));
      onSaved(saved);
      onClose();
    } catch (e) {
      setRejected(toAppError(e));
      setSaving(false);
    }
  };

  // An empty field is only a problem once the user tried to save; everything else shows as it is typed.
  const nameProblem = problems.name && (submitted || problems.name !== "empty") ? problems.name : null;
  const nameError = nameProblem ? t(`aiSettings.skills.err.${nameProblemKey(nameProblem)}`) : null;
  const descProblem = problems.description && (submitted || problems.description !== "empty") ? problems.description : null;
  const descLength = descriptionLength(draft.description);
  const bodyBytes = byteLength(draft.body);

  return (
    <Sheet
      width={720}
      onClose={busy ? undefined : () => void requestClose()}
      closeOnBackdrop={false}
      footer={
        <>
          <FooterSpacer />
          <Button onClick={() => void requestClose()} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={ids.form} busy={saving} disabled={loading || !!loadError}>
            {t("btn.save")}
          </Button>
        </>
      }
    >
      <SheetHeader
        title={id ? t("aiSettings.skills.dlg.editTitle") : t("aiSettings.skills.dlg.newTitle")}
        subtitle={t("aiSettings.skills.dlg.subtitle")}
      />

      {loading && (
        <div className={s.loading} role="status">
          <Spinner />
          {t("aiSettings.skills.dlg.loading")}
        </div>
      )}
      {!!loadError && (
        <div className={s.loadFailed} role="alert">
          <div className={s.problem}>
            {t("aiSettings.skills.loadFailed")} {errorMessage(t, loadError)}
          </div>
          <Button size="sm" onClick={() => void load()}>
            {t("btn.retry")}
          </Button>
        </div>
      )}

      {!loading && !loadError && (
        <form
          id={ids.form}
          className={s.form}
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <Group>
            <FormRow label={t("aiSettings.skills.f.name")} htmlFor={ids.name} top error={nameError}>
              <div className={s.stack}>
                <TextField
                  id={ids.name}
                  mono
                  autoFocus={id === null}
                  value={draft.name}
                  invalid={!!nameProblem}
                  placeholder={t("aiSettings.skills.f.namePlaceholder")}
                  onChange={(e) => patch({ name: e.target.value })}
                />
                {nameProblem !== "invalid" && <span className={s.hint}>{t("aiSettings.skills.f.nameHint")}</span>}
              </div>
            </FormRow>
            <FormRow
              label={t("aiSettings.skills.f.description")}
              htmlFor={ids.description}
              top
              error={
                descProblem === "empty"
                  ? t("aiSettings.skills.err.descRequired")
                  : descProblem === "too_long"
                    ? t("aiSettings.skills.err.descTooLong", { chars: descLength, max: SKILL_DESCRIPTION_MAX })
                    : null
              }
            >
              <div className={s.stack}>
                <TextArea
                  id={ids.description}
                  className={s.plain}
                  rows={3}
                  value={draft.description}
                  aria-invalid={!!descProblem || undefined}
                  placeholder={t("aiSettings.skills.f.descriptionPlaceholder")}
                  onChange={(e) => patch({ description: e.target.value })}
                />
                <span className={cx(s.counter, descLength > SKILL_DESCRIPTION_MAX && s.counterOver)}>
                  {descLength.toLocaleString()} / {SKILL_DESCRIPTION_MAX.toLocaleString()}
                </span>
              </div>
            </FormRow>
          </Group>

          <Section title={t("aiSettings.skills.f.body")}>
            <div className={s.hint}>{t("aiSettings.skills.f.bodyHint")}</div>
            <TextArea
              id={ids.body}
              className={cx(s.code, s.body)}
              aria-label={t("aiSettings.skills.f.body")}
              aria-invalid={!!problems.body || undefined}
              value={draft.body}
              placeholder={t("aiSettings.skills.f.bodyPlaceholder")}
              onChange={(e) => patch({ body: e.target.value })}
            />
            <span className={cx(s.counter, !!problems.body && s.counterOver)}>
              {formatBytes(bodyBytes)} / {formatBytes(SKILL_FILE_MAX_BYTES)}
            </span>
            {problems.body && <div className={s.problem}>{t("aiSettings.skills.err.bodyTooLarge")}</div>}
          </Section>

          <Section title={t("aiSettings.skills.files")}>
            <div className={s.filesBar}>
              <span className={s.hint}>{t("aiSettings.skills.files.hint")}</span>
              <Button size="sm" icon="plus" onClick={addFile}>
                {t("aiSettings.skills.files.add")}
              </Button>
            </div>
            <Group>
              {draft.files.length === 0 ? (
                <div className={s.fileEmpty}>{t("aiSettings.skills.files.empty")}</div>
              ) : (
                <div className={s.fileList}>
                  {draft.files.map((file, i) => (
                    <FileEditor
                      key={file.uid}
                      file={file}
                      open={open.has(file.uid)}
                      autoFocus={focusUid === file.uid}
                      problem={submitted || (problems.files[i] !== "path_empty" && problems.files[i] !== null) ? problems.files[i] : null}
                      onToggle={() => toggleFile(file.uid)}
                      onChange={(p) => patchFile(file.uid, p)}
                      onRemove={() => removeFile(file.uid)}
                    />
                  ))}
                </div>
              )}
            </Group>
            {problems.tooManyFiles && <div className={s.problem}>{t("aiSettings.skills.err.tooManyFiles", { max: SKILL_MAX_FILES })}</div>}
            {problems.tooLarge && <div className={s.problem}>{t("aiSettings.skills.err.tooLarge")}</div>}
          </Section>

          {detail && detail.frontmatter_keys.length > 0 && (
            <div className={s.note}>{t("aiSettings.skills.frontmatter", { keys: detail.frontmatter_keys.join(", ") })}</div>
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

/** One extra file: its path in a row, and its text below when the row is open. */
function FileEditor({
  file,
  open,
  autoFocus,
  problem,
  onToggle,
  onChange,
  onRemove,
}: {
  file: SkillFileDraft;
  open: boolean;
  autoFocus: boolean;
  problem: ReturnType<typeof validateSkill>["files"][number];
  onToggle: () => void;
  onChange: (patch: Partial<SkillFileDraft>) => void;
  onRemove: () => void;
}) {
  const t = useT();
  const bytes = byteLength(file.content);
  const label = file.path.trim() || t("aiSettings.skills.file.unnamed");
  return (
    <div className={s.file}>
      <div className={s.fileHead}>
        <button
          type="button"
          className={cx(s.fileToggle, open && s.fileOpen)}
          aria-expanded={open}
          aria-label={t("aiSettings.skills.file.toggle", { path: label })}
          onClick={onToggle}
        >
          <Icon name="caret-right" />
        </button>
        <TextField
          mono
          autoFocus={autoFocus}
          aria-label={t("aiSettings.skills.file.path")}
          value={file.path}
          invalid={!!problem && problem !== "too_large"}
          placeholder={t("aiSettings.skills.file.pathPlaceholder")}
          onChange={(e) => onChange({ path: e.target.value })}
        />
        <span className={cx(s.size, bytes > SKILL_FILE_MAX_BYTES && s.counterOver)}>{formatBytes(bytes)}</span>
        <IconButton icon="x" size={13} label={t("aiSettings.skills.file.remove", { path: label })} onClick={onRemove} />
      </div>
      {open && (
        <div className={s.fileBody}>
          <TextArea
            className={s.code}
            rows={8}
            aria-label={t("aiSettings.skills.file.content", { path: label })}
            aria-invalid={problem === "too_large" || undefined}
            value={file.content}
            onChange={(e) => onChange({ content: e.target.value })}
          />
        </div>
      )}
      {problem && (
        <div className={s.problem} role="alert">
          {t(`aiSettings.skills.file.err.${problem}` as MessageKey)}
        </div>
      )}
    </div>
  );
}
