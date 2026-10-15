import { useCallback, useEffect, useRef, useState } from "react";
import { useVaultData } from "@/app/data";
import { Button, Icon, IconButton, Segmented, Spinner, StatusDot, Switch, controlStyles } from "@/components/controls";
import { Group, layoutStyles } from "@/components/layout";
import { Menu, PopupSelect, confirm, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { LOCAL_TARGET, removeAllowance, savedAllowances, useAllowances } from "@/features/ai/allowances";
import { EFFORTS, effortKey } from "@/features/ai/effort";
import { answersText, verdictText } from "@/features/ai/autoText";
import { InstructionsField } from "@/features/ai/InstructionsField";
import { judgeChosen } from "@/features/ai/models";
import { byteCount, INSTRUCTIONS_MAX_BYTES } from "@/features/ai/instructions";
import { toolLabel } from "@/features/ai/tools";
import { useT, type MessageKey } from "@/i18n";
import { api } from "@/ipc/api";
import type {
  AiAllowance,
  AiEffort,
  AiJudgeDecision,
  AiJudgeSampleOutcome,
  AiModelRef,
  AiPermissionMode,
  AiProviderView,
  AiSettingsPatch,
  AiSettingsView,
  SearchProviderView,
} from "@/ipc/types";
import { cx } from "@/lib/cx";
import {
  changedSettings,
  judgeChoices,
  modelChoices,
  parseBaseUrl,
  resolveSettings,
  sameRef,
  sanitizeSettings,
  type ModelChoice,
} from "./aiLogic";
import { aiErrorMessage } from "./aiShared";
import { ChatgptSignInDialog, chatgptRowKey } from "./ChatgptSignIn";
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
 * Settings → AI (spec §13): providers and their models (AI-01 to AI-04), Continue with ChatGPT (AI-52), the default model (AI-05),
 * web search (AI-14), custom instructions (AI-36), whether messages send the terminal screen by
 * default (AI-46), the judge of Auto mode (AI-47), and the permission mode and saved approvals of this
 * device (AI-16, AI-19).
 */
export function AiPane() {
  const t = useT();
  const [data, setData] = useState<Data | null>(null);
  const [loadError, setLoadError] = useState<unknown>(null);
  const [dialog, setDialog] = useState<{ provider: AiProviderView | null } | null>(null);
  const [signingIn, setSigningIn] = useState(false);
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
            <span className={s.headActions}>
              <Button size="sm" onClick={() => setSigningIn(true)}>
                {t("aiSettings.chatgpt.continue")}
              </Button>
              <Button size="sm" icon="plus" onClick={() => setDialog({ provider: null })}>
                {t("aiSettings.providers.add")}
              </Button>
            </span>
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
              <Button onClick={() => setSigningIn(true)}>{t("aiSettings.chatgpt.continue")}</Button>
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
                  choices={modelChoices(providers)}
                  value={data.settings.default_model}
                  label={t("aiSettings.default.row")}
                  placeholder={t("aiSettings.default.placeholder")}
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
            native={data.settings.native_search}
            onChoose={(choice) => void saveSettings(choice)}
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
      {data && (
        <JudgeSection
          providers={providers}
          value={data.settings.judge_model}
          onChange={(judge_model) => void saveSettings({ judge_model })}
        />
      )}
      <DeviceSection judge={data && judgeChosen(data.providers, data.settings)} />

      {dialog && (
        <ProviderDialog
          provider={dialog.provider}
          onClose={() => setDialog(null)}
          onSaved={(saved) => setProviders(upsert(dataRef.current?.providers ?? [], saved))}
        />
      )}
      {signingIn && (
        <ChatgptSignInDialog
          onDone={(saved) => setProviders(upsert(dataRef.current?.providers ?? [], saved))}
          onClose={() => setSigningIn(false)}
        />
      )}
    </Pane>
  );
}

function ProviderRow({ provider: p, onEdit, onDelete }: { provider: AiProviderView; onEdit: () => void; onDelete: () => void }) {
  const t = useT();
  const host = parseBaseUrl(p.base_url)?.host ?? p.base_url;
  const signIn = p.chatgpt ? chatgptRowKey(p.chatgpt.status) : null;
  return (
    <div className={s.provider}>
      <button type="button" className={s.providerMain} title={t("aiSettings.provider.edit", { name: p.name })} onClick={onEdit}>
        <span className={s.providerIcon} aria-hidden>
          <Icon name="sparkle" />
        </span>
        <span className={s.providerText}>
          <span className={s.providerName}>{p.name}</span>
          <span className={s.providerMeta}>
            {p.chatgpt ? (
              <>
                <span>{t("aiSettings.chatgpt.kind")}</span>
                {p.chatgpt.email && (
                  <>
                    <span className={s.dot}>·</span>
                    <span className={s.providerAccount}>{p.chatgpt.email}</span>
                  </>
                )}
              </>
            ) : (
              <>
                <span>{t(`aiSettings.protocol.${p.protocol}`)}</span>
                <span className={s.dot}>·</span>
                <span className={s.providerHost}>{host}</span>
              </>
            )}
            <span className={s.dot}>·</span>
            {signIn && (
              <>
                <span className={s.warn}>{t(signIn)}</span>
                <span className={s.dot}>·</span>
              </>
            )}
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

/**
 * A model picker with `choices` grouped by provider, for the default model (AI-05) and the judge of Auto mode
 * (AI-47). With `noneLabel`, a first entry chooses no model.
 */
function ModelSelect({
  choices,
  value,
  label,
  placeholder,
  noneLabel,
  onChange,
}: {
  choices: ModelChoice[];
  value: AiModelRef | null;
  /** Names the button. */
  label: string;
  /** Shows while no model is chosen. */
  placeholder: string;
  noneLabel?: string;
  onChange: (ref: AiModelRef | null) => void;
}) {
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);
  const selected = choices.find((c) => sameRef(c.ref, value));

  const entries: MenuEntry[] = [];
  if (noneLabel) {
    entries.push({ label: noneLabel, checked: !selected, onSelect: () => onChange(null) });
    if (choices.length > 0) entries.push({ kind: "separator" });
  }
  choices.forEach((c, i) => {
    if (c.ref.provider_id !== choices[i - 1]?.ref.provider_id) entries.push({ kind: "header", label: c.providerName });
    entries.push({ label: c.model.name || c.model.id, checked: sameRef(c.ref, value), onSelect: () => onChange(c.ref) });
  });

  return (
    <>
      <button
        ref={ref}
        type="button"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        disabled={choices.length === 0}
        className={controlStyles.popup}
        style={{ minWidth: 260, maxWidth: 320 }}
        onClick={() => ref.current && menu.openBelow(ref.current)}
      >
        <span className={controlStyles.popupLabel} style={selected ? undefined : { color: "var(--fg2)" }}>
          {selected ? selected.model.name || selected.model.id : placeholder}
        </span>
        {selected && <span className={controlStyles.popupHint}>{selected.providerName}</span>}
        <Icon name="caret-up-down" className={controlStyles.popupCaret} />
      </button>
      {menu.anchor && <Menu anchor={menu.anchor} entries={entries} onClose={menu.close} minWidth={ref.current?.offsetWidth} />}
    </>
  );
}

/** What **Test** beside the judge shows (AI-47), for the judge it ran with. */
type JudgeTest = { judge: AiModelRef } & (
  | { state: "running" }
  | { state: "done"; outcomes: AiJudgeSampleOutcome[] }
  | { state: "error"; message: string }
);

/** The short labels of the sample calls, by the names Rust gives them; a sample a newer version added shows its name. */
const SAMPLE_LABEL: Record<string, MessageKey> = {
  disk_space: "aiSettings.judge.sample.disk_space",
  restart_requested: "aiSettings.judge.sample.restart_requested",
  delete_unrequested: "aiSettings.judge.sample.delete_unrequested",
  key_sent_out: "aiSettings.judge.sample.key_sent_out",
  restart_forbidden: "aiSettings.judge.sample.restart_forbidden",
  pipe_to_shell: "aiSettings.judge.sample.pipe_to_shell",
  key_via_shell: "aiSettings.judge.sample.key_via_shell",
  restart_requested_zh: "aiSettings.judge.sample.restart_requested_zh",
  delete_unrequested_ja: "aiSettings.judge.sample.delete_unrequested_ja",
  docs_page: "aiSettings.judge.sample.docs_page",
  token_in_url: "aiSettings.judge.sample.token_in_url",
  input_read: "aiSettings.judge.sample.input_read",
  answer_unrequested: "aiSettings.judge.sample.answer_unrequested",
};

const DECISION_LABEL: Record<AiJudgeDecision, MessageKey> = {
  run: "aiSettings.judge.decision.run",
  ask: "aiSettings.judge.decision.ask",
};

/** A sample the judge answered with the decision expected; a failure, which asks, does not count. */
const asExpected = (o: AiJudgeSampleOutcome) => o.verdict.failure === null && o.verdict.decision === o.expected;

/**
 * AI-47: the judge of Auto mode, which syncs: a model of any provider, System One ones included, or none, which
 * leaves Auto unavailable. **Test** runs sample calls with the judge the picker shows and counts the decisions that
 * match the ones expected, since probabilities differ between backends; **Details** lists each one.
 */
function JudgeSection({
  providers,
  value,
  onChange,
}: {
  providers: AiProviderView[];
  value: AiModelRef | null;
  onChange: (ref: AiModelRef | null) => void;
}) {
  const t = useT();
  const [test, setTest] = useState<JudgeTest | null>(null);
  /** Whether the samples of the test's outcome show; each test starts with them hidden. */
  const [details, setDetails] = useState(false);
  /** Bumped by each test, so that only the last one's answer shows. */
  const generation = useRef(0);
  const choices = judgeChoices(providers);
  // A test shows only while its judge is the one chosen.
  const shown = test && sameRef(test.judge, value) ? test : null;

  const run = async () => {
    if (!value) return;
    const judge = value;
    const mine = ++generation.current;
    setTest({ judge, state: "running" });
    setDetails(false);
    try {
      const outcomes = await api.ai_judge_test(judge);
      if (mine === generation.current) setTest({ judge, state: "done", outcomes });
    } catch (e) {
      if (mine === generation.current) setTest({ judge, state: "error", message: aiErrorMessage(t, e) });
    }
  };

  return (
    <section className={layoutStyles.section}>
      <div className={layoutStyles.sectionTitle}>{t("aiSettings.judge")}</div>
      <Group>
        <SettingRow label={t("aiSettings.judge.row")} hint={t(choices.length === 0 ? "aiSettings.default.empty" : "aiSettings.judge.rowHint")}>
          <ModelSelect
            choices={choices}
            value={value}
            label={t("aiSettings.judge.row")}
            placeholder={t("aiSettings.judge.placeholder")}
            noneLabel={t("aiSettings.judge.none")}
            onChange={onChange}
          />
          <Button icon="flask" busy={shown?.state === "running"} disabled={!value || choices.length === 0} onClick={() => void run()}>
            {t("aiSettings.judge.test")}
          </Button>
        </SettingRow>
        {shown && <JudgeTestResult test={shown} details={details} onDetails={() => setDetails((open) => !open)} />}
      </Group>
      <div className={s.sectionHint}>{t("aiSettings.judge.hint")}</div>
    </section>
  );
}

/**
 * The outcome of **Test** beside the judge (AI-47): how many samples came out as expected, and under **Details** each
 * sample with the decision expected, the judge's, and its answers or why it gave none. A sample that ran when it
 * should have asked is red, one that asked when it could have run orange (it only costs a click).
 */
function JudgeTestResult({ test, details, onDetails }: { test: JudgeTest; details: boolean; onDetails: () => void }) {
  const t = useT();
  if (test.state === "running") {
    return (
      <div className={s.judgeStatus} role="status">
        <Spinner size={12} />
        {t("aiSettings.judge.test.running")}
      </div>
    );
  }
  if (test.state === "error") {
    return (
      <div className={s.judgeStatus} role="status">
        <StatusDot color="var(--red)" />
        <span className={s.judgeFail}>{test.message}</span>
      </div>
    );
  }
  const { outcomes } = test;
  const matched = outcomes.filter(asExpected).length;
  const ranWrongly = outcomes.some((o) => !asExpected(o) && o.verdict.decision === "run");
  const summary = matched === outcomes.length ? "var(--green)" : ranWrongly ? "var(--red)" : "var(--orange)";
  return (
    <div className={s.judgeResult}>
      <div className={s.judgeStatus}>
        <StatusDot color={summary} />
        <span role="status">{t("aiSettings.judge.test.summary", { n: matched, total: outcomes.length })}</span>
        <Button
          size="xs"
          className={s.judgeDetails}
          trailingIcon={details ? "caret-up" : "caret-down"}
          aria-expanded={details}
          onClick={onDetails}
        >
          {t("aiSettings.judge.test.details")}
        </Button>
      </div>
      {details && <JudgeSamples outcomes={outcomes} />}
    </div>
  );
}

/** **Details** of a test's outcome: each sample with the decision expected, the judge's, and its answers. */
function JudgeSamples({ outcomes }: { outcomes: AiJudgeSampleOutcome[] }) {
  const t = useT();
  return (
    <ul className={s.samples}>
      {outcomes.map((o) => {
        const failed = o.verdict.failure !== null;
        const ok = asExpected(o);
        const mark = ok
          ? { icon: "check-circle", color: "var(--green)", label: t("aiSettings.judge.test.match") }
          : failed
            ? { icon: "warning-circle", color: "var(--orange)", label: t("aiSettings.judge.test.unanswered") }
            : { icon: "x-circle", color: o.verdict.decision === "run" ? "var(--red)" : "var(--orange)", label: t("aiSettings.judge.test.mismatch") };
        return (
          <li key={o.name} className={s.sample}>
            <span className={s.sampleMark} role="img" aria-label={mark.label} title={mark.label}>
              <Icon name={mark.icon} fill color={mark.color} />
            </span>
            <span className={s.sampleText}>
              <span className={s.sampleName}>{SAMPLE_LABEL[o.name] ? t(SAMPLE_LABEL[o.name]) : o.name}</span>
              <span className={s.sampleMeta}>
                {t("aiSettings.judge.test.expected", { decision: t(DECISION_LABEL[o.expected]) })}
                <span className={s.dot}>·</span>
                <span className={ok ? undefined : s.sampleOff}>{t("aiSettings.judge.test.judged", { decision: t(DECISION_LABEL[o.verdict.decision]) })}</span>
              </span>
              <span className={cx(s.sampleDetail, "selectable")}>{failed ? verdictText(t, o.verdict) : answersText(t, o.verdict.answers)}</span>
            </span>
          </li>
        );
      })}
    </ul>
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

  const hostName = (a: AiAllowance) =>
    a.host_id !== null
      ? (hosts.find((h) => h.id === a.host_id)?.name ?? t("ai.history.hostGone"))
      : a.target === LOCAL_TARGET
        ? t("terminal.local.thisComputer")
        : (a.target ?? "");
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

/**
 * Settings that stay on this device and are not synced: the permission mode (AI-16) and the saved approvals
 * (AI-19). Auto (AI-47) is offered once a judge is chosen (`judge`; null until the AI settings are read).
 */
function DeviceSection({ judge }: { judge: boolean | null }) {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();

  const chooseMode = async (mode: AiPermissionMode) => {
    if (mode === prefs.ai_permission_mode || (mode === "auto" && judge !== true)) return;
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
        <SettingRow label={t("aiSettings.mode")} hint={t(judge === false ? "aiSettings.mode.hintNoJudge" : "aiSettings.mode.hint")} top>
          <Segmented<AiPermissionMode>
            ariaLabel={t("aiSettings.mode")}
            value={prefs.ai_permission_mode}
            options={[
              { value: "manual", label: t("aiSettings.mode.manual") },
              // The device's own Auto stays selected once the judge is gone; every call then asks.
              { value: "auto", label: t("aiSettings.mode.auto"), disabled: judge !== true && prefs.ai_permission_mode !== "auto" },
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
