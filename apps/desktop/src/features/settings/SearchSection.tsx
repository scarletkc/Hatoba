import { useRef, useState } from "react";
import { Button, LinkButton, TextField } from "@/components/controls";
import { FormRow, Group, layoutStyles } from "@/components/layout";
import { PopupSelect, confirm, toast } from "@/components/overlay";
import { useT, type MessageKey } from "@/i18n";
import { api } from "@/ipc/api";
import type { SearchKind, SearchProviderInput, SearchProviderView } from "@/ipc/types";
import { SEARCH_KINDS, keyForInput, normalizeBaseUrl, parseBaseUrl, searchNeedsUrl, searchProviderFor, type KeyDraft } from "./aiLogic";
import { IDLE, SavedKeyField, TestConnection, aiErrorDetail, aiErrorMessage, type TestState } from "./aiShared";
import { SettingRow } from "./shared";
import s from "./AiPane.module.css";

const KEEP: KeyDraft = { mode: "keep", value: "" };

/**
 * Web search (AI-14): the backend of `web_search`. Settings hold one provider per kind and choose
 * at most one of them, so switching the kind switches to that kind's saved settings.
 */
export function SearchSection({
  providers,
  chosenId,
  onChoose,
  onSaved,
  onRemoved,
}: {
  providers: SearchProviderView[];
  chosenId: string | null;
  /** The search provider to use, or null for none. */
  onChoose: (id: string | null) => void;
  onSaved: (saved: SearchProviderView) => void;
  onRemoved: (id: string) => void;
}) {
  const t = useT();
  const [kind, setKind] = useState<SearchKind | null>(() => providers.find((p) => p.id === chosenId)?.kind ?? null);
  const saved = kind ? searchProviderFor(providers, kind, chosenId) : null;
  const [baseUrl, setBaseUrl] = useState(saved?.base_url ?? "");
  const [key, setKey] = useState<KeyDraft>(KEEP);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [test, setTest] = useState<TestState>(IDLE);
  /** Bumped when the form changes, so the answer to an older test is dropped. */
  const generation = useRef(0);

  const needsUrl = kind ? searchNeedsUrl(kind) : false;
  const hasSavedKey = !!saved?.has_api_key;

  const reset = (next: SearchKind | null) => {
    const found = next ? searchProviderFor(providers, next, chosenId) : null;
    setKind(next);
    setBaseUrl(found?.base_url ?? "");
    setKey(KEEP);
    setError(null);
    setTest(IDLE);
    generation.current++;
  };

  const choose = (next: SearchKind | null) => {
    if (next === kind) return;
    reset(next);
    // A kind that is already saved takes effect at once; a new one waits for Save.
    const found = next ? searchProviderFor(providers, next, chosenId) : null;
    if (!next || found) onChoose(found?.id ?? null);
  };

  const edit = (patch: { baseUrl?: string; key?: KeyDraft }) => {
    if (patch.baseUrl !== undefined) setBaseUrl(patch.baseUrl);
    if (patch.key !== undefined) setKey(patch.key);
    setError(null);
    setTest(IDLE);
    generation.current++;
  };

  const toInput = (): SearchProviderInput | null => {
    if (!kind) return null;
    return {
      id: saved?.id ?? null,
      kind,
      base_url: needsUrl ? normalizeBaseUrl(baseUrl) : null,
      api_key: needsUrl ? null : keyForInput(key, hasSavedKey),
    };
  };

  /** Why the form can't be used yet, or null. */
  const problem = (): string | null => {
    if (!kind) return null;
    if (needsUrl) {
      if (!baseUrl.trim()) return t("aiSettings.search.err.urlRequired");
      return parseBaseUrl(baseUrl) ? null : t("aiSettings.err.urlInvalid");
    }
    return keyForInput(key, hasSavedKey) === null && !hasSavedKey ? t("aiSettings.err.keyRequired") : null;
  };

  const dirty =
    !!kind &&
    (!saved ||
      (needsUrl ? normalizeBaseUrl(baseUrl) !== (saved.base_url ?? "") : keyForInput(key, hasSavedKey) !== null));

  const runTest = async () => {
    const input = toInput();
    const why = problem();
    if (why) return setError(why);
    if (!input) return;
    const mine = ++generation.current;
    setTest({ state: "running" });
    try {
      const result = await api.search_provider_test(input);
      if (mine === generation.current) setTest({ state: "done", result, model: null });
    } catch (err) {
      if (mine !== generation.current) return;
      const { status, message } = aiErrorDetail(t, err);
      setTest({ state: "error", status, detail: message ?? aiErrorMessage(t, err) });
    }
  };

  const save = async () => {
    const input = toInput();
    const why = problem();
    if (why) return setError(why);
    if (!input || saving) return;
    setSaving(true);
    try {
      const view = await api.search_provider_save(input);
      onSaved(view);
      setKey(KEEP);
      setBaseUrl(view.base_url ?? "");
      setTest(IDLE);
    } catch (err) {
      setError(aiErrorMessage(t, err));
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (!saved || !kind) return;
    const ok = await confirm({
      title: t("aiSettings.search.removeTitle", { name: t(`aiSettings.search.${kind}` as MessageKey) }),
      body: t("aiSettings.search.removeBody"),
      confirmLabel: t("btn.remove"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.search_provider_delete(saved.id);
      onRemoved(saved.id);
      reset(null);
    } catch (err) {
      toast(aiErrorMessage(t, err), "error");
    }
  };

  const options: { value: SearchKind | null; label: string }[] = [
    { value: null, label: t("aiSettings.search.none") },
    ...SEARCH_KINDS.map((k) => ({ value: k, label: t(`aiSettings.search.${k}` as MessageKey) })),
  ];

  return (
    <div className={layoutStyles.section}>
      <div className={layoutStyles.sectionTitle}>{t("aiSettings.search")}</div>
      <Group>
        <SettingRow label={t("aiSettings.search.row")} hint={t("aiSettings.search.hint")}>
          <PopupSelect<SearchKind | null>
            ariaLabel={t("aiSettings.search.row")}
            value={kind}
            minWidth={180}
            options={options}
            onChange={choose}
          />
        </SettingRow>
        {kind && (
          <>
            {needsUrl ? (
              <FormRow label={t("aiSettings.search.baseUrl")} htmlFor="ai-search-url" top error={error}>
                <div className={s.stack}>
                  <TextField
                    id="ai-search-url"
                    mono
                    value={baseUrl}
                    invalid={!!error}
                    placeholder="https://searx.example.com"
                    onChange={(e) => edit({ baseUrl: e.target.value })}
                  />
                  <span className={s.fieldHint}>{t("aiSettings.search.baseUrlHint")}</span>
                </div>
              </FormRow>
            ) : (
              <FormRow label={t("aiSettings.search.apiKey")} htmlFor="ai-search-key" error={error}>
                <SavedKeyField
                  id="ai-search-key"
                  hasSaved={hasSavedKey}
                  draft={key}
                  allowClear={false}
                  invalid={!!error}
                  placeholder={t("aiSettings.search.apiKeyPlaceholder")}
                  onChange={(next) => edit({ key: next })}
                />
              </FormRow>
            )}
            <div className={s.searchActions}>
              <TestConnection scope="search" state={test} onRun={() => void runTest()} />
              <div className={s.searchButtons}>
                {saved && (
                  <LinkButton tone="danger" onClick={() => void remove()}>
                    {t("aiSettings.search.remove")}
                  </LinkButton>
                )}
                <Button variant="primary" busy={saving} disabled={!dirty} onClick={() => void save()}>
                  {t("btn.save")}
                </Button>
              </div>
            </div>
          </>
        )}
      </Group>
    </div>
  );
}
