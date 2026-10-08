import { useCallback, useEffect, useRef, useState } from "react";
import { Button, Icon, IconButton, Segmented, Spinner, TextField, controlStyles } from "@/components/controls";
import { Group, layoutStyles } from "@/components/layout";
import { Menu, PopupSelect, confirm, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { EFFORTS, effortKey } from "@/features/ai/effort";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { AiEffort, AiModelRef, AiPermissionMode, AiProviderView, AiSettingsView, SearchProviderView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import {
  TOOL_LIMIT_DEFAULT,
  modelChoices,
  parseBaseUrl,
  parseToolLimit,
  resolveSettings,
  sameRef,
  sameSettings,
  sanitizeSettings,
} from "./aiLogic";
import { aiErrorMessage } from "./aiShared";
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
 * web search (AI-14), and the permission mode and tool call limit of this device (AI-16, AI-18).
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

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const [providers, search, settings] = await Promise.all([
        api.ai_providers_list(),
        api.search_providers_list(),
        api.ai_settings_get(),
      ]);
      if (live.current) commit({ providers, search, settings: sanitizeSettings(settings, providers, search) });
    } catch (e) {
      if (live.current) setLoadError(e);
    }
  }, [commit]);

  useEffect(() => {
    live.current = true;
    void load();
    return () => {
      live.current = false;
    };
  }, [load]);

  /** Stores the synced settings. On failure the pane shows what the backend has. */
  const saveSettings = async (settings: AiSettingsView) => {
    const cur = dataRef.current;
    if (!cur) return;
    commit({ ...cur, settings });
    try {
      await api.ai_settings_save(settings);
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
      void load();
    }
  };

  /** After providers were added, edited, or deleted: keeps the default model pointing at a model that exists. */
  const setProviders = (providers: AiProviderView[]) => {
    const cur = dataRef.current;
    if (!cur) return;
    commit({ ...cur, providers });
    const next = resolveSettings(cur.settings, providers, cur.search);
    if (!sameSettings(next, cur.settings)) void saveSettings(next);
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
                <Button size="sm" onClick={() => void load()}>
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
        <div className={s.sectionHint}>{t("aiSettings.providers.hint")}</div>
      </section>

      {data && (
        <>
          <section className={layoutStyles.section}>
            <div className={layoutStyles.sectionTitle}>{t("aiSettings.default")}</div>
            <Group>
              <SettingRow
                label={t("aiSettings.default.row")}
                hint={modelChoices(providers).length === 0 ? t("aiSettings.default.empty") : t("aiSettings.default.hint")}
              >
                <ModelSelect
                  providers={providers}
                  value={data.settings.default_model}
                  onChange={(default_model) => void saveSettings({ ...data.settings, default_model })}
                />
              </SettingRow>
              <SettingRow label={t("aiSettings.default.effort")} hint={t("aiSettings.default.effortHint")}>
                <PopupSelect<AiEffort | null>
                  ariaLabel={t("aiSettings.default.effort")}
                  value={data.settings.default_effort}
                  options={[null, ...EFFORTS].map((e) => ({ value: e, label: t(effortKey(e)) }))}
                  onChange={(default_effort) => void saveSettings({ ...data.settings, default_effort })}
                  minWidth={160}
                />
              </SettingRow>
            </Group>
          </section>

          <SearchSection
            providers={data.search}
            chosenId={data.settings.search_provider_id}
            onChoose={(id) => void saveSettings({ ...data.settings, search_provider_id: id })}
            onSaved={(view) => {
              const cur = dataRef.current;
              if (!cur) return;
              const search = upsert(cur.search, view);
              commit({ ...cur, search });
              void saveSettings({ ...cur.settings, search_provider_id: view.id });
            }}
            onRemoved={(id) => {
              const cur = dataRef.current;
              if (!cur) return;
              const search = cur.search.filter((x) => x.id !== id);
              commit({ ...cur, search });
              const next = sanitizeSettings(cur.settings, cur.providers, search);
              if (!sameSettings(next, cur.settings)) void saveSettings(next);
            }}
          />
        </>
      )}

      <SkillsSection
        builtin={
          data
            ? {
                enabled: data.settings.builtin_skill_enabled,
                onChange: (enabled) => void saveSettings({ ...(dataRef.current?.settings ?? data.settings), builtin_skill_enabled: enabled }),
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

/** Settings that stay on this device and are not synced: the permission mode (AI-16) and the tool call limit (AI-18). */
function DeviceSection() {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();
  const limit = prefs.ai_tool_call_limit ?? TOOL_LIMIT_DEFAULT;
  const [limitText, setLimitText] = useState(String(limit));
  useEffect(() => setLimitText(String(limit)), [limit]);

  const commitLimit = () => {
    const n = parseToolLimit(limitText);
    if (n === null) return setLimitText(String(limit));
    setLimitText(String(n));
    if (n !== limit) setPrefs({ ai_tool_call_limit: n });
  };

  const chooseMode = async (mode: AiPermissionMode) => {
    if (mode === prefs.ai_permission_mode) return;
    if (mode === "bypass" && !prefs.ai_bypass_confirmed) {
      const ok = await confirm({
        title: t("aiSettings.bypass.title"),
        body: t("aiSettings.bypass.body"),
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
        <SettingRow label={t("aiSettings.limit")} hint={t("aiSettings.limit.hint")} top>
          <div className={s.limitField}>
            <TextField
              mono
              inputMode="numeric"
              aria-label={t("aiSettings.limit")}
              value={limitText}
              onChange={(e) => setLimitText(e.target.value)}
              onBlur={commitLimit}
              onKeyDown={(e) => e.key === "Enter" && commitLimit()}
            />
          </div>
        </SettingRow>
      </Group>
      <div className={s.sectionHint}>{t("aiSettings.device.hint")}</div>
    </section>
  );
}
