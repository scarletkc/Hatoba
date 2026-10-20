import { useEffect, useId, useRef, useState } from "react";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { Button, Icon, IconButton, Spinner, Switch, TextArea, TextField } from "@/components/controls";
import { FormRow, Group, layoutStyles } from "@/components/layout";
import { FooterSpacer, PopupSelect, Sheet, SheetHeader, confirm, toast, type SelectOption } from "@/components/overlay";
import {
  cleanMemoryContent,
  cleanMemoryName,
  editedIds,
  MEMORY_MAX_BYTES,
  MEMORY_NAME_MAX_CHARS,
  memoriesSize,
  memoryProblem,
  memoryScope,
  scopeFields,
  type MemoryProblem,
  type MemoryScope,
} from "@/features/ai/memory";
import { useT, type MessageKey } from "@/i18n";
import { api } from "@/ipc/api";
import type { AiMemoryView, HostView } from "@/ipc/types";
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

type Tr = ReturnType<typeof useT>;

/** How the list and the dialog name a scope: Global, This Computer or another computer's local terminal, or a host. */
function scopeLabel(t: Tr, scope: MemoryScope, hosts: readonly HostView[], device: string): string {
  const { host_id, local } = scopeFields(scope);
  if (host_id !== null) return hosts.find((h) => h.id === host_id)?.name ?? t("ai.history.hostGone");
  if (local !== null) return local === device ? t("terminal.local.thisComputer") : t("ai.place.localOn", { device: local });
  return t("aiSettings.memory.scope.global");
}

function scopeIcon(scope: MemoryScope): string {
  return scope === "global" ? "globe" : scope.startsWith("local:") ? "terminal-window" : "hard-drives";
}

/**
 * Where a scope goes in the list: the global memories, then this computer's, the saved hosts' by name, other
 * computers', and those of hosts deleted since.
 */
function scopeRank(scope: MemoryScope, hosts: readonly HostView[], device: string): [number, string] {
  const { host_id, local } = scopeFields(scope);
  if (host_id !== null) {
    const host = hosts.find((h) => h.id === host_id);
    return host ? [2, host.name] : [4, host_id];
  }
  if (local !== null) return local === device ? [1, ""] : [3, local];
  return [0, ""];
}

/** The memories by scope, in list order, each oldest first. */
function scopeGroups(list: readonly AiMemoryView[], hosts: readonly HostView[], device: string): [MemoryScope, AiMemoryView[]][] {
  const groups = new Map<MemoryScope, AiMemoryView[]>();
  for (const m of list) {
    const scope = memoryScope(m);
    groups.set(scope, [...(groups.get(scope) ?? []), m]);
  }
  return [...groups].sort(([x], [y]) => {
    const [rx, nx] = scopeRank(x, hosts, device);
    const [ry, ny] = scopeRank(y, hosts, device);
    return rx - ry || nx.localeCompare(ny);
  });
}

/**
 * Settings → AI → Memory (AI-53): the switch, then the memories the assistant keeps, by scope (global, then each
 * computer's and host's), oldest first, each scope with what it takes of its limit: add, edit or delete one, or
 * delete them all. `enabled` is null until the AI settings are loaded. The list is read again after each sync round,
 * as the other synced settings are. `/memory` in the AI panel opens Settings here, once `settled` says the AI
 * settings above the section have loaded or failed to, so the section no longer moves when it is scrolled to.
 */
export function MemorySection({ enabled, settled }: { enabled: MemorySwitch | null; settled: boolean }) {
  const t = useT();
  const [memories, setMemories] = useState<AiMemoryView[] | null>(null);
  const [loadError, setLoadError] = useState<unknown>(null);
  const [editing, setEditing] = useState<{ memory: AiMemoryView | null } | null>(null);
  const section = useRef<HTMLElement>(null);
  const requested = useApp((st) => st.settingsSection);
  const device = useApp((st) => st.info.device_name);
  const hosts = useVaultData((st) => st.hosts);
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

      {(!loaded || list.length === 0) && (
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
          {loaded && (
            <EmptyBlock icon="brain" title={t("aiSettings.memory.empty.title")} body={t("aiSettings.memory.empty.body")}>
              <Button variant="primary" icon="plus" onClick={() => setEditing({ memory: null })}>
                {t("aiSettings.memory.add")}
              </Button>
            </EmptyBlock>
          )}
        </div>
      )}
      {scopeGroups(list, hosts, device).map(([scope, group]) => (
        <div key={scope} className={a.scope}>
          <div className={a.scopeHead}>
            <span className={a.scopeName}>
              <Icon name={scopeIcon(scope)} />
              {scopeLabel(t, scope, hosts, device)}
            </span>
            <span>{t("aiSettings.memory.size", { n: nf.format(memoriesSize(group)), max: nf.format(MEMORY_MAX_BYTES) })}</span>
          </div>
          <div className={cx(layoutStyles.group, a.clip)}>
            {group.map((memory) => (
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
        </div>
      ))}

      {editing && (
        <MemoryDialog
          memory={editing.memory}
          memories={list}
          hosts={hosts}
          device={device}
          onClose={() => setEditing(null)}
          onSaved={(saved, replaced) => {
            reads.edited();
            setMemories((cur) => {
              // Rust deleted the others it saved in place of, which count as one with it (AI-53).
              const rest = (cur ?? []).filter((m) => m.id === saved.id || !replaced.includes(m.id));
              return rest.some((m) => m.id === saved.id) ? rest.map((m) => (m.id === saved.id ? saved : m)) : [...rest, saved];
            });
            setEditing(null);
          }}
        />
      )}
    </section>
  );
}

/**
 * Adds a memory, or edits `memory`, in the scope the dialog picks, checked as Rust checks it against the others in
 * `memories`. The scopes offered are Global, this computer's and each saved host's, and the memory's own when it is
 * none of them.
 */
function MemoryDialog({
  memory,
  memories,
  hosts,
  device,
  onClose,
  onSaved,
}: {
  memory: AiMemoryView | null;
  memories: readonly AiMemoryView[];
  hosts: readonly HostView[];
  device: string;
  onClose: () => void;
  onSaved: (saved: AiMemoryView, replaced: readonly string[]) => void;
}) {
  const t = useT();
  const ids = { form: useId(), name: useId(), content: useId() };
  const [name, setName] = useState(memory?.name ?? "");
  const [content, setContent] = useState(memory?.content ?? "");
  const [scope, setScope] = useState<MemoryScope>(memory ? memoryScope(memory) : "global");
  /** A rule the memory breaks, or what Rust answered. */
  const [problem, setProblem] = useState<{ rule: MemoryProblem } | { text: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const nf = new Intl.NumberFormat(t.locale);
  const replaced = editedIds(memories, memory?.id ?? null, scope, name);
  const others = memories.filter((m) => !replaced.includes(m.id) && memoryScope(m) === scope);
  const used = memoriesSize([...others, { name: cleanMemoryName(name), content: cleanMemoryContent(content) }]);
  const rule = problem && "rule" in problem ? problem.rule : null;
  const nameProblem = rule === "empty_name" || rule === "long_name" || rule === "taken";

  const scopes: MemoryScope[] = [
    "global",
    ...(device ? [`local:${device}` as const] : []),
    ...[...hosts].sort((x, y) => x.name.localeCompare(y.name)).map((h) => `host:${h.id}` as const),
  ];
  if (memory && !scopes.includes(memoryScope(memory))) scopes.push(memoryScope(memory));
  const options: SelectOption<MemoryScope>[] = scopes.map((value) => ({ value, label: scopeLabel(t, value, hosts, device), icon: scopeIcon(value) }));

  const save = async () => {
    if (saving) return;
    const found = memoryProblem(memories, replaced, scope, name, content);
    if (found) {
      setProblem({ rule: found });
      document.getElementById(found === "empty_content" || found === "full" ? ids.content : ids.name)?.focus();
      return;
    }
    setSaving(true);
    try {
      onSaved(await api.ai_memory_save({ id: memory?.id ?? null, name, content, ...scopeFields(scope) }), replaced);
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
          <FormRow label={t("aiSettings.memory.f.scope")}>
            <PopupSelect<MemoryScope>
              ariaLabel={t("aiSettings.memory.f.scope")}
              value={scope}
              options={options}
              onChange={(next) => {
                setScope(next);
                setProblem(null);
              }}
              minWidth={220}
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
