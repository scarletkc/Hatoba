import { useCallback, useEffect, useRef, useState } from "react";
import { useVaultData } from "@/app/data";
import { Button, Icon, IconButton, Segmented, Spinner, Switch, controlStyles } from "@/components/controls";
import { Group, layoutStyles } from "@/components/layout";
import { Menu, PopupSelect, confirm, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { removeAllowance, savedAllowances, useAllowances } from "@/features/ai/allowances";
import { EFFORTS, effortKey } from "@/features/ai/effort";
import { InstructionsField } from "@/features/ai/InstructionsField";
import { byteCount, INSTRUCTIONS_MAX_BYTES } from "@/features/ai/instructions";
import { toolLabel } from "@/features/ai/tools";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { AiAllowance, AiEffort, AiModelRef, AiPermissionMode, AiProviderView, AiSettingsPatch, AiSettingsView, SearchProviderView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { changedSettings, modelChoices, parseBaseUrl, resolveSettings, sameRef, sanitizeSettings } from "./aiLogic";
import { aiErrorMessage } from "./aiShared";
import { useFreshReads } from "./freshReads";
import { McpSection } from "./McpSection";
import { ProviderDialog } from "./ProviderDialog";
import { SearchSection } from "./SearchSection";
import { SkillsSection } from "./SkillsSection";
import { EmptyBlock, StatusRow } from "./sectionParts";
import { Pane, SettingRow, usePrefs } from "./shared";
import s from "./AiPane.module.css";

interface Data {
  providers: AiProviderView[];
  search: SearchProviderView[];
  settings: AiSettingsView;
}

/** Replaces the item with the same ID, or appends it. */
function upsert<T extends { id: string }>(list: T[], item: T): T[] {
  return list.some((x) => x.id === item.id) ? list.map((x) => (x.id === item.id ? item : x)) : [...list, item];
}

/**
 * Settings → AI (spec §13): providers and their models (AI-01 to AI-04), the default model (AI-05),
 * web search (AI-14), custom instructions (AI-36), whether messages send the terminal screen by
 * default (AI-46), and the permission mode and saved approvals of this device (AI-16, AI-19).
 */
export function AiPane() {
  const t = useT();
  const [data, setData] = useState<Data | null>(null);
  const [loadError, setLoadError] = useState<unknown>(null);
  const [dialog, setDialog] = useState<{ provider: AiProviderView | null } | null>(null);
  const dataRef = useRef<Data | null>(null);
  const live = useRef(true);

  const commit = useCallback((next: Data) => {
    dataRef.current = next;
    if (live.current) setData(next);
  }, []);

  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);

  const reads = useFreshReads(
    async (): Promise<Data> => {
      const [providers, search, settings] = await Promise.all([
        api.ai_providers_list(),
        api.search_providers_list(),
        api.ai_settings_get(),
      ]);
      return { providers, search, settings: sanitizeSettings(settings, providers, search) };
    },
    (next) => {
      setLoadError(null);
      commit(next);
    },
    setLoadError,
  );

  /** Shows a change of the lists that a dialog has saved. */
  const edit = (next: Data) => {
    reads.edited();
    commit(next);
  };

  /** Stores the fields in `patch` of the synced settings. On failure the pane shows what the backend has. */
  const saveSettings = async (patch: AiSettingsPatch) => {
    const cur = dataRef.current;
    if (!cur) return;
    commit({ ...cur, settings: { ...cur.settings, ...patch } });
    try {
      await reads.save(() => api.ai_settings_save(patch));
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
    }
  };

  /** After providers were added, edited, or deleted: keeps the default model pointing at a model that exists. */
  const setProviders = (providers: AiProviderView[]) => {
    const cur = dataRef.current;
    if (!cur) return;
    edit({ ...cur, providers });
    const changes = changedSettings(cur.settings, resolveSettings(cur.settings, providers, cur.search));
    if (changes) void saveSettings(changes);
  };

  const removeProvider = async (p: AiProviderView) => {
    const ok = await confirm({
      title: t("aiSettings.provider.deleteTitle", { name: p.name }),
      body: t("aiSettings.provider.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.ai_provider_delete(p.id);
      setProviders((dataRef.current?.providers ?? []).filter((x) => x.id !== p.id));
      toast(t("aiSettings.provider.deleted", { name: p.name }));
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const loaded = data !== null;
  const providers = data?.providers ?? [];

  return (
    <Pane>
      <section className={layoutStyles.section}>
        <div className={s.sectionHead}>
          <span className={layoutStyles.sectionTitle}>{t("aiSettings.providers")}</span>
          {loaded && providers.length > 0 && (
            <Button size="sm" icon="plus" onClick={() => setDialog({ provider: null })}>
              {t("aiSettings.providers.add")}
            </Button>
          )}
        </div>
        <div className={cx(layoutStyles.group, s.clip)}>
          {!loaded && !loadError && (
            <StatusRow icon={<Spinner />}>{t("aiSettings.loading")}</StatusRow>
          )}
          {!loaded && !!loadError && (
            <StatusRow
              icon={<Icon name="warning-circle" color="var(--red)" />}
              action={
                <Button
                  size="sm"
                  onClick={() => {
                    setLoadError(null);
                    reads.refresh();
                  }}
                >
                  {t("btn.retry")}
                </Button>
              }
            >
              {t("aiSettings.loadFailed")} {aiErrorMessage(t, loadError)}
            </StatusRow>
          )}
          {loaded && providers.length === 0 && (
            <EmptyBlock icon="sparkle" title={t("aiSettings.providers.empty.title")} body={t("aiSettings.providers.empty.body")}>
              <Button variant="primary" icon="plus" onClick={() => setDialog({ provider: null })}>
                {t("aiSettings.providers.add")}
              </Button>
            </EmptyBlock>
          )}
          {providers.map((p) => (
            <ProviderRow key={p.id} provider={p} onEdit={() => setDialog({ provider: p })} onDelete={() => void removeProvider(p)} />
          ))}
        </div>
      </section>

      {data && (
        <>
          <section className={layoutStyles.section}>
            <div className={layoutStyles.sectionTitle}>{t("aiSettings.default")}</div>
            <Group>
              <SettingRow
                label={t("aiSettings.default.row")}
                hint={modelChoices(providers).length === 0 ? t("aiSettings.default.empty") : undefined}
              >
                <ModelSelect
                  providers={providers}
                  value={data.settings.default_model}
                  onChange={(default_model) => void saveSettings({ default_model })}
                />
              </SettingRow>
              <SettingRow label={t("aiSettings.default.effort")} hint={t("aiSettings.default.effortHint")}>
                <PopupSelect<AiEffort | null>
                  ariaLabel={t("aiSettings.default.effort")}
                  value={data.settings.default_effort}
                  options={[null, ...EFFORTS].map((e) => ({ value: e, label: t(effortKey(e)) }))}
                  onChange={(default_effort) => void saveSettings({ default_effort })}
                  minWidth={160}
                />
              </SettingRow>
            </Group>
          </section>

          <SearchSection
            providers={data.search}
            chosenId={data.settings.search_provider_id}
            onChoose={(id) => void saveSettings({ search_provider_id: id })}
            onSaved={(view) => {
              const cur = dataRef.current;
              if (!cur) return;
              edit({ ...cur, search: upsert(cur.search, view) });
              void saveSettings({ search_provider_id: view.id });
            }}
            onRemoved={(id) => {
              const cur = dataRef.current;
              if (!cur) return;
              const search = cur.search.filter((x) => x.id !== id);
              edit({ ...cur, search });
              const changes = changedSettings(cur.settings, sanitizeSettings(cur.settings, cur.providers, search));
              if (changes) void saveSettings(changes);
            }}
          />
        </>
      )}

      {data && (
        <InstructionsSection
          value={data.settings.custom_instructions}
          onSave={(custom_instructions) => void saveSettings({ custom_instructions })}
        />
      )}
      {data && (
        <section className={layoutStyles.section}>
          <div className={layoutStyles.sectionTitle}>{t("aiSettings.screen")}</div>
          <Group>
            <SettingRow label={t("aiSettings.screen.row")} hint={t("aiSettings.screen.hint")}>
              <Switch
                label={t("aiSettings.screen.row")}
                checked={data.settings.send_terminal_screen}
                onChange={(send_terminal_screen) => void saveSettings({ send_terminal_screen })}
              />
            </SettingRow>
          </Group>
        </section>
      )}
      <SkillsSection
        builtin={
          data
            ? {
                enabled: data.settings.builtin_skill_enabled,
                onChange: (enabled) => void saveSettings({ builtin_skill_enabled: enabled }),
              }
            : null
        }
      />
      <McpSection />
      <DeviceSection />

      {dialog && (
        <ProviderDialog
          provider={dialog.provider}
          onClose={() => setDialog(null)}
          onSaved={(saved) => setProviders(upsert(dataRef.current?.providers ?? [], saved))}
        />
      )}
    </Pane>
  );
}

function ProviderRow({ provider: p, onEdit, onDelete }: { provider: AiProviderView; onEdit: () => void; onDelete: () => void }) {
  const t = useT();
  const host = parseBaseUrl(p.base_url)?.host ?? p.base_url;
  return (
    <div className={s.provider}>
      <button type="button" className={s.providerMain} title={t("aiSettings.provider.edit", { name: p.name })} onClick={onEdit}>
        <span className={s.providerIcon} aria-hidden>
          <Icon name="sparkle" />
        </span>
        <span className={s.providerText}>
          <span className={s.providerName}>{p.name}</span>
          <span className={s.providerMeta}>
            <span>{t(`aiSettings.protocol.${p.protocol}`)}</span>
            <span className={s.dot}>·</span>
            <span className={s.providerHost}>{host}</span>
            <span className={s.dot}>·</span>
            <span className={p.models.length === 0 ? s.warn : undefined}>
              {p.models.length === 0 ? t("aiSettings.provider.noModels") : t("aiSettings.provider.models", { n: p.models.length })}
            </span>
          </span>
        </span>
        <Icon name="caret-right" className={s.providerCaret} />
      </button>
      <IconButton icon="trash" label={t("aiSettings.provider.delete", { name: p.name })} className={s.providerDelete} onClick={onDelete} />
    </div>
  );
}

/** The default model (AI-05): every provider's models, grouped by provider. */
function ModelSelect({
  providers,
  value,
  onChange,
}: {
  providers: AiProviderView[];
  value: AiModelRef | null;
  onChange: (ref: AiModelRef) => void;
}) {
  const t = useT();
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);
  const choices = modelChoices(providers);
  const selected = choices.find((c) => sameRef(c.ref, value));

  const entries: MenuEntry[] = providers.flatMap((p): MenuEntry[] =>
    p.models.length === 0
      ? []
      : [
          { kind: "header", label: p.name },
          ...p.models.map(
            (m): MenuEntry => ({
              label: m.name || m.id,
              checked: sameRef({ provider_id: p.id, model_id: m.id }, value),
              onSelect: () => onChange({ provider_id: p.id, model_id: m.id }),
            }),
          ),
        ],
  );

  return (
    <>
      <button
        ref={ref}
        type="button"
        aria-label={t("aiSettings.default.row")}
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        disabled={choices.length === 0}
        className={controlStyles.popup}
        style={{ minWidth: 260, maxWidth: 320 }}
        onClick={() => ref.current && menu.openBelow(ref.current)}
      >
        <span className={controlStyles.popupLabel} style={selected ? undefined : { color: "var(--fg2)" }}>
          {selected ? selected.model.name || selected.model.id : t("aiSettings.default.placeholder")}
        </span>
        {selected && <span className={controlStyles.popupHint}>{selected.providerName}</span>}
        <Icon name="caret-up-down" className={controlStyles.popupCaret} />
      </button>
      {menu.anchor && <Menu anchor={menu.anchor} entries={entries} onClose={menu.close} minWidth={ref.current?.offsetWidth} />}
    </>
  );
}

/** How long after the last keystroke custom instructions are saved. */
const INSTRUCTIONS_SAVE_MS = 800;

/**
 * AI-36: custom instructions, sent with every request. Saved a moment after typing stops, when the
 * box loses focus, and when the pane closes, unless they are over the limit. A change synced from
 * another device shows while the box is not being edited.
 */
function InstructionsSection({ value, onSave }: { value: string; onSave: (text: string) => void }) {
  const t = useT();
  const [draft, setDraft] = useState(value);
  const editing = useRef(false);
  const latest = useRef({ draft, value, onSave });
  latest.current = { draft, value, onSave };

  useEffect(() => {
    if (!editing.current) setDraft(value);
  }, [value]);

  const commit = useCallback(() => {
    const { draft, value, onSave } = latest.current;
    editing.current = false;
    if (draft !== value && byteCount(draft) <= INSTRUCTIONS_MAX_BYTES) onSave(draft);
  }, []);

  useEffect(() => {
    if (draft === value) return;
    const timer = window.setTimeout(commit, INSTRUCTIONS_SAVE_MS);
    return () => window.clearTimeout(timer);
  }, [draft, value, commit]);
  useEffect(() => commit, [commit]);

  return (
    <section className={layoutStyles.section}>
      <div className={layoutStyles.sectionTitle}>{t("aiSettings.instructions")}</div>
      <div className={s.sectionHint}>{t("aiSettings.instructions.hint")}</div>
      <InstructionsField
        id="ai-custom-instructions"
        label={t("aiSettings.instructions")}
        value={draft}
        max={INSTRUCTIONS_MAX_BYTES}
        rows={6}
        placeholder={t("aiSettings.instructions.placeholder")}
        onChange={(text) => {
          editing.current = true;
          setDraft(text);
        }}
        onBlur={commit}
      />
    </section>
  );
}

const ALLOWANCE_ICON: Record<string, string> = { command: "terminal", domain: "globe", edit_file: "note-pencil", write_file: "file-plus", send_input: "keyboard" };

/** AI-19: the approvals saved with Always Allow on approval cards, as rows that Remove deletes. */
function SavedApprovals() {
  const t = useT();
  const list = useAllowances((st) => st.list);
  const hosts = useVaultData((st) => st.hosts);
  useEffect(() => {
    savedAllowances().catch((e) => toast(aiErrorMessage(t, e), "error"));
  }, [t]);

  const hostName = (a: AiAllowance) => (a.host_id !== null ? (hosts.find((h) => h.id === a.host_id)?.name ?? t("ai.history.hostGone")) : (a.target ?? ""));
  const what = (a: AiAllowance) =>
    a.kind === "command" ? t("aiSettings.allowances.command", { prefix: a.value }) : a.kind === "domain" ? t("aiSettings.allowances.domain", { domain: a.value }) : toolLabel(t, a.value);
  const remove = (a: AiAllowance) => removeAllowance(a).catch((e) => toast(aiErrorMessage(t, e), "error"));

  return (
    <>
      <SettingRow label={t("aiSettings.allowances")} hint={list?.length === 0 ? t("aiSettings.allowances.empty") : undefined} />
      {list?.map((a) => (
        <div key={`${a.kind}\n${a.host_id ?? ""}\n${a.target ?? ""}\n${a.value}`} className={s.allowance}>
          <Icon name={ALLOWANCE_ICON[a.kind === "tool" ? a.value : a.kind] ?? "check-circle"} className={s.allowanceIcon} />
          <span className={s.allowanceText}>
            <span className={s.allowanceWhat}>{what(a)}</span>
            {a.kind !== "domain" && <span className={s.allowanceHost}>{hostName(a)}</span>}
          </span>
          <IconButton icon="x" label={t("btn.remove")} onClick={() => void remove(a)} />
        </div>
      ))}
    </>
  );
}

/** Settings that stay on this device and are not synced: the permission mode (AI-16) and the saved approvals (AI-19). */
function DeviceSection() {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();

  const chooseMode = async (mode: AiPermissionMode) => {
    if (mode === prefs.ai_permission_mode) return;
    if (mode === "bypass" && !prefs.ai_bypass_confirmed) {
      const ok = await confirm({
        title: t("aiSettings.bypass.title"),
        body: t("ai.bypass.body"),
        confirmLabel: t("aiSettings.bypass.confirm"),
        danger: true,
      });
      if (!ok) return;
      return setPrefs({ ai_permission_mode: "bypass", ai_bypass_confirmed: true });
    }
    setPrefs({ ai_permission_mode: mode });
  };

  return (
    <section className={layoutStyles.section}>
      <div className={layoutStyles.sectionTitle}>{t("aiSettings.device")}</div>
      <Group>
        <SettingRow label={t("aiSettings.mode")} hint={t("aiSettings.mode.hint")} top>
          <Segmented<AiPermissionMode>
            ariaLabel={t("aiSettings.mode")}
            value={prefs.ai_permission_mode}
            options={[
              { value: "manual", label: t("aiSettings.mode.manual") },
              { value: "bypass", label: t("aiSettings.mode.bypass") },
            ]}
            onChange={(mode) => void chooseMode(mode)}
          />
        </SettingRow>
        <SavedApprovals />
      </Group>
    </section>
  );
}
