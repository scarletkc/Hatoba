import { useEffect, useId, useRef, useState } from "react";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { Button, Icon, IconButton, Spinner, Switch, TextArea, TextField } from "@/components/controls";
import { FormRow, Group, layoutStyles } from "@/components/layout";
import { FooterSpacer, Sheet, SheetHeader, confirm, toast } from "@/components/overlay";
import {
  cleanMemoryContent,
  cleanMemoryName,
  editedIds,
  MEMORY_MAX_BYTES,
  MEMORY_NAME_MAX_CHARS,
  memoriesSize,
  memoryProblem,
  type MemoryProblem,
} from "@/features/ai/memory";
import { useT, type MessageKey } from "@/i18n";
import { api } from "@/ipc/api";
import type { AiMemoryView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { aiErrorMessage } from "./aiShared";
import { useFreshReads } from "./freshReads";
import { EmptyBlock, StatusRow } from "./sectionParts";
import { SettingRow } from "./shared";
import a from "./AiPane.module.css";
import s from "./SkillsSection.module.css";

/** The switch of memory, which is `Settings.ai.memory`; the AI pane owns those settings. */
export interface MemorySwitch {
  on: boolean;
  onChange: (on: boolean) => void;
}

const PROBLEM: Record<MemoryProblem, MessageKey> = {
  empty_name: "aiSettings.memory.err.empty_name",
  long_name: "aiSettings.memory.err.long_name",
  empty_content: "aiSettings.memory.err.empty_content",
  taken: "aiSettings.memory.err.taken",
  full: "aiSettings.memory.err.full",
};

/**
 * Settings → AI → Memory (AI-53): the switch, then the memories the assistant keeps, oldest first, with what they
 * take of the limit: add, edit or delete one, or delete them all. `enabled` is null until the AI settings are loaded.
 * The list is read again after each sync round, as the other synced settings are. `/memory` in the AI panel opens
 * Settings here, once `settled` says the AI settings above the section have loaded or failed to, so the section no
 * longer moves when it is scrolled to.
 */
export function MemorySection({ enabled, settled }: { enabled: MemorySwitch | null; settled: boolean }) {
  const t = useT();
  const [memories, setMemories] = useState<AiMemoryView[] | null>(null);
  const [loadError, setLoadError] = useState<unknown>(null);
  const [editing, setEditing] = useState<{ memory: AiMemoryView | null } | null>(null);
  const section = useRef<HTMLElement>(null);
  const requested = useApp((st) => st.settingsSection);
  const loaded = memories !== null;
  const list = memories ?? [];

  const reads = useFreshReads(
    () => api.ai_memories_list(),
    (next) => {
      setLoadError(null);
      setMemories(next);
    },
    setLoadError,
  );

  // `/memory` asked for this section: show it once it and what is above it have their heights.
  useEffect(() => {
    if (requested !== "memory" || !settled || (memories === null && !loadError)) return;
    section.current?.scrollIntoView({ block: "start" });
    useApp.setState({ settingsSection: null });
  }, [requested, settled, memories, loadError]);

  const remove = async (memory: AiMemoryView) => {
    const ok = await confirm({
      title: t("aiSettings.memory.deleteTitle", { name: memory.name }),
      body: t("aiSettings.memory.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    setMemories((cur) => (cur ?? []).filter((m) => m.id !== memory.id));
    try {
      await reads.save(() => api.ai_memory_delete(memory.id));
      toast(t("aiSettings.memory.deleted", { name: memory.name }));
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const removeAll = async () => {
    const ok = await confirm({
      title: t("aiSettings.memory.deleteAllTitle", { n: list.length }),
      body: t("aiSettings.memory.deleteBody"),
      confirmLabel: t("aiSettings.memory.deleteAll"),
      danger: true,
    });
    if (!ok) return;
    setMemories([]);
    try {
      await reads.save(() => api.ai_memories_delete_all());
      toast(t("aiSettings.memory.deletedAll"));
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const nf = new Intl.NumberFormat(t.locale);
  const used = memoriesSize(list);
  const off = enabled ? !enabled.on : false;

  return (
    <section ref={section} className={layoutStyles.section}>
      <div className={a.sectionHead}>
        <span className={layoutStyles.sectionTitle}>{t("aiSettings.memory")}</span>
        {loaded && list.length > 0 && (
          <div className={s.headButtons}>
            <Button size="sm" icon="trash" onClick={() => void removeAll()}>
              {t("aiSettings.memory.deleteAll")}
            </Button>
            <Button size="sm" icon="plus" onClick={() => setEditing({ memory: null })}>
              {t("aiSettings.memory.add")}
            </Button>
          </div>
        )}
      </div>
      {enabled && (
        <Group>
          <SettingRow label={t("aiSettings.memory.row")} hint={t("aiSettings.memory.hint")}>
            <Switch label={t("aiSettings.memory.row")} checked={enabled.on} onChange={enabled.onChange} />
          </SettingRow>
        </Group>
      )}

      <div className={cx(layoutStyles.group, a.clip)}>
        {!loaded && !loadError && <StatusRow icon={<Spinner />}>{t("aiSettings.loading")}</StatusRow>}
        {!loaded && !!loadError && (
          <StatusRow
            icon={<Icon name="warning-circle" color="var(--red)" />}
            action={
              <Button size="sm" onClick={() => reads.refresh()}>
                {t("btn.retry")}
              </Button>
            }
          >
            {t("aiSettings.memory.loadFailed")} {aiErrorMessage(t, loadError)}
          </StatusRow>
        )}
        {loaded && list.length === 0 && (
          <EmptyBlock icon="brain" title={t("aiSettings.memory.empty.title")} body={t("aiSettings.memory.empty.body")}>
            <Button variant="primary" icon="plus" onClick={() => setEditing({ memory: null })}>
              {t("aiSettings.memory.add")}
            </Button>
          </EmptyBlock>
        )}
        {list.map((memory) => (
          <div key={memory.id} className={cx(s.row, off && s.off)}>
            <button type="button" className={s.main} title={t("aiSettings.memory.edit", { name: memory.name })} onClick={() => setEditing({ memory })}>
              <span className={s.icon} aria-hidden>
                <Icon name="brain" />
              </span>
              <span className={s.text}>
                <span className={s.nameLine}>
                  <span className={s.name}>{memory.name}</span>
                </span>
                <span className={s.description}>{memory.content}</span>
              </span>
            </button>
            <div className={s.actions}>
              <IconButton icon="trash" label={t("aiSettings.memory.delete", { name: memory.name })} onClick={() => void remove(memory)} />
            </div>
          </div>
        ))}
      </div>
      {loaded && list.length > 0 && (
        <div className={a.sectionHint}>{t("aiSettings.memory.size", { n: nf.format(used), max: nf.format(MEMORY_MAX_BYTES) })}</div>
      )}

      {editing && (
        <MemoryDialog
          memory={editing.memory}
          memories={list}
          onClose={() => setEditing(null)}
          onSaved={(saved) => {
            reads.edited();
            setMemories((cur) => {
              // Rust deleted the others of its name, which count as one with it (AI-53).
              const rest = (cur ?? []).filter((m) => m.id === saved.id || m.name !== saved.name);
              return rest.some((m) => m.id === saved.id) ? rest.map((m) => (m.id === saved.id ? saved : m)) : [...rest, saved];
            });
            setEditing(null);
          }}
        />
      )}
    </section>
  );
}

/** Adds a memory, or edits `memory`, checked as Rust checks it against the others in `memories`. */
function MemoryDialog({
  memory,
  memories,
  onClose,
  onSaved,
}: {
  memory: AiMemoryView | null;
  memories: readonly AiMemoryView[];
  onClose: () => void;
  onSaved: (saved: AiMemoryView) => void;
}) {
  const t = useT();
  const ids = { form: useId(), name: useId(), content: useId() };
  const [name, setName] = useState(memory?.name ?? "");
  const [content, setContent] = useState(memory?.content ?? "");
  /** A rule the memory breaks, or what Rust answered. */
  const [problem, setProblem] = useState<{ rule: MemoryProblem } | { text: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const nf = new Intl.NumberFormat(t.locale);
  const replaced = editedIds(memories, memory?.id ?? null, name);
  const others = memories.filter((m) => !replaced.includes(m.id));
  const used = memoriesSize([...others, { name: cleanMemoryName(name), content: cleanMemoryContent(content) }]);
  const rule = problem && "rule" in problem ? problem.rule : null;
  const nameProblem = rule === "empty_name" || rule === "long_name" || rule === "taken";

  const save = async () => {
    if (saving) return;
    const found = memoryProblem(memories, replaced, name, content);
    if (found) {
      setProblem({ rule: found });
      document.getElementById(found === "empty_content" || found === "full" ? ids.content : ids.name)?.focus();
      return;
    }
    setSaving(true);
    try {
      onSaved(await api.ai_memory_save({ id: memory?.id ?? null, name, content }));
    } catch (e) {
      setProblem({ text: errorMessage(t, e) });
      setSaving(false);
    }
  };

  const message = !problem
    ? null
    : "rule" in problem
      ? t(PROBLEM[problem.rule], { max: nf.format(MEMORY_MAX_BYTES), n: MEMORY_NAME_MAX_CHARS })
      : problem.text;

  return (
    <Sheet
      width={520}
      onClose={saving ? undefined : onClose}
      closeOnBackdrop={false}
      footer={
        <>
          <FooterSpacer />
          <Button onClick={onClose} disabled={saving}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={ids.form} busy={saving}>
            {t("btn.save")}
          </Button>
        </>
      }
    >
      <SheetHeader title={memory ? t("aiSettings.memory.dlg.editTitle") : t("aiSettings.memory.dlg.newTitle")} />
      <form
        id={ids.form}
        className={s.form}
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <Group>
          <FormRow label={t("aiSettings.memory.f.name")} htmlFor={ids.name} error={nameProblem ? message : null}>
            <TextField
              id={ids.name}
              autoFocus
              value={name}
              invalid={nameProblem}
              placeholder={t("aiSettings.memory.f.namePlaceholder")}
              onChange={(e) => {
                setName(e.target.value);
                setProblem(null);
              }}
            />
          </FormRow>
        </Group>
        <div className={s.stack}>
          <TextArea
            id={ids.content}
            className={s.plain}
            rows={6}
            value={content}
            aria-label={t("aiSettings.memory.f.content")}
            aria-invalid={(!!problem && !nameProblem) || undefined}
            placeholder={t("aiSettings.memory.f.contentPlaceholder")}
            onChange={(e) => {
              setContent(e.target.value);
              setProblem(null);
            }}
          />
          {problem && !nameProblem && <span className={s.problem}>{message}</span>}
          <span className={cx(s.counter, used > MEMORY_MAX_BYTES && s.counterOver)}>
            {t("aiSettings.memory.size", { n: nf.format(used), max: nf.format(MEMORY_MAX_BYTES) })}
          </span>
        </div>
      </form>
    </Sheet>
  );
}
