import { useId, useRef, useState } from "react";
import { Button, Icon, IconButton, Segmented, TextField, controlStyles } from "@/components/controls";
import { FormRow, Group, Section } from "@/components/layout";
import { FooterSpacer, Menu, Sheet, SheetHeader, useMenu } from "@/components/overlay";
import { EFFORTS, UNKNOWN_EFFORTS, effortKey } from "@/features/ai/effort";
import { useT, type MessageKey } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AiAuthHeader, AiEffort, AiModel, AiProtocol, AiProviderInput, AiProviderView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import {
  addModels,
  formatEfforts,
  formatTokenCount,
  keyForInput,
  matchesFilter,
  normalizeBaseUrl,
  parseBaseUrl,
  parseModelIds,
  removeModel,
  suggestProtocol,
  toDraft,
  toggleEffort,
  toModels,
  typedModel,
  validateModels,
  withListedCapabilities,
  type KeyDraft,
  type ModelDraft,
} from "./aiLogic";
import { IDLE, SavedKeyField, TestConnection, aiErrorDetail, aiErrorMessage, type TestState } from "./aiShared";
import s from "./ProviderDialog.module.css";

interface Form {
  name: string;
  protocol: AiProtocol;
  /** The user picked a protocol, so the base URL no longer suggests one. */
  protocolChosen: boolean;
  baseUrl: string;
  authHeader: AiAuthHeader;
  key: KeyDraft;
  models: ModelDraft[];
}

type Errors = Partial<Record<"name" | "baseUrl" | "general", string>>;

/** The provider's model list, fetched on request (AI-03). */
type Fetched =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ok"; list: AiModel[]; filter: string }
  | { state: "error"; text: string };

const PICKER_LIMIT = 100;
const PLACEHOLDER_URL: Record<AiProtocol, string> = {
  chat_completions: "https://api.openai.com/v1",
  anthropic: "https://api.anthropic.com",
};

function initialForm(p: AiProviderView | null): Form {
  return {
    name: p?.name ?? "",
    protocol: p?.protocol ?? "chat_completions",
    protocolChosen: !!p,
    baseUrl: p?.base_url ?? "",
    authHeader: p?.auth_header ?? "x-api-key",
    key: { mode: "keep", value: "" },
    models: p?.models.map(toDraft) ?? [],
  };
}

/** Add or edit one provider (AI-01 to AI-04). Saved straight to the backend, not with the pane. */
export function ProviderDialog({
  provider,
  onClose,
  onSaved,
}: {
  provider: AiProviderView | null;
  onClose: () => void;
  onSaved: (saved: AiProviderView) => void;
}) {
  const t = useT();
  const ids = { form: useId(), name: useId(), url: useId(), key: useId() };
  const [form, setForm] = useState<Form>(() => initialForm(provider));
  const [errors, setErrors] = useState<Errors>({});
  const [adding, setAdding] = useState("");
  const [saving, setSaving] = useState(false);
  const [test, setTest] = useState<TestState>(IDLE);
  const [fetched, setFetched] = useState<Fetched>({ state: "idle" });
  /** Bumped when the connection settings change, so an answer to the old ones is dropped. */
  const generation = useRef(0);

  const hasSavedKey = !!provider?.has_api_key;
  const problems = validateModels(form.models);
  const modelProblem = problems.findIndex(Boolean);

  const patch = (p: Partial<Form>, field?: keyof Errors) => {
    setForm((f) => ({ ...f, ...p }));
    if (field) setErrors((e) => (e[field] || e.general ? { ...e, [field]: undefined, general: undefined } : e));
  };
  /** A change to what a request is made with: the test and the fetched list describe the old settings. */
  const patchConnection = (p: Partial<Form>, field?: keyof Errors) => {
    generation.current++;
    setTest(IDLE);
    setFetched((f) => (f.state === "idle" ? f : { state: "idle" }));
    patch(p, field);
  };

  const setBaseUrl = (baseUrl: string) =>
    patchConnection({ baseUrl, ...(form.protocolChosen ? {} : { protocol: suggestProtocol(baseUrl) }) }, "baseUrl");

  /** Models typed into the add field but not added yet still count when the user moves on. */
  const withPending = (rows: ModelDraft[]): ModelDraft[] => {
    const typed = parseModelIds(adding);
    return typed.length ? addModels(rows, typed.map(typedModel)) : rows;
  };
  const commitPending = (): ModelDraft[] => {
    const rows = withPending(form.models);
    if (rows !== form.models) {
      patch({ models: rows });
      setAdding("");
    }
    return rows;
  };

  const toInput = (rows: ModelDraft[]): AiProviderInput => ({
    id: provider?.id ?? null,
    name: form.name.trim(),
    protocol: form.protocol,
    base_url: normalizeBaseUrl(form.baseUrl),
    api_key: keyForInput(form.key, hasSavedKey),
    auth_header: form.authHeader,
    models: toModels(rows),
  });

  const urlError = (): string | undefined => {
    if (!form.baseUrl.trim()) return t("aiSettings.err.urlRequired");
    return parseBaseUrl(form.baseUrl) ? undefined : t("aiSettings.err.urlInvalid");
  };

  const showErrors = (e: Errors) => {
    setErrors(e);
    const first = (["name", "baseUrl"] as const).find((k) => e[k]);
    if (first) document.getElementById(first === "name" ? ids.name : ids.url)?.focus();
  };

  const save = async () => {
    if (saving) return;
    const rows = commitPending();
    const e: Errors = {};
    if (!form.name.trim()) e.name = t("aiSettings.err.nameRequired");
    const url = urlError();
    if (url) e.baseUrl = url;
    if (Object.keys(e).length) return showErrors(e);
    if (validateModels(rows).some(Boolean)) return;
    setSaving(true);
    try {
      const saved = await api.ai_provider_save(toInput(rows));
      onSaved(saved);
      onClose();
    } catch (err) {
      const ae = toAppError(err);
      if (ae.code === "invalid_input" && ae.field === "base_url") showErrors({ baseUrl: t("aiSettings.err.urlRejected") });
      else if (ae.code === "invalid_input" && ae.field === "name") showErrors({ name: t("aiSettings.err.nameRequired") });
      else setErrors({ general: aiErrorMessage(t, err) });
      setSaving(false);
    }
  };

  const fetchModels = async () => {
    const url = urlError();
    if (url) return showErrors({ baseUrl: url });
    const mine = ++generation.current;
    setFetched({ state: "loading" });
    try {
      const list = await api.ai_provider_models(toInput(form.models));
      if (mine !== generation.current) return;
      setFetched({ state: "ok", list, filter: "" });
      // AI-05: models already in the form learn their thinking levels from the list.
      setForm((f) => {
        const models = withListedCapabilities(f.models, list);
        return models === f.models ? f : { ...f, models };
      });
    } catch (err) {
      if (mine !== generation.current) return;
      const { status, message } = aiErrorDetail(t, err);
      const text = status || message ? [status, message].filter(Boolean).join(" · ") : aiErrorMessage(t, err);
      setFetched({ state: "error", text });
    }
  };

  const runTest = async () => {
    const url = urlError();
    if (url) return showErrors({ baseUrl: url });
    const rows = commitPending();
    const input = toInput(rows);
    const mine = ++generation.current;
    setTest({ state: "running" });
    try {
      const result = await api.ai_provider_test(input);
      if (mine === generation.current) setTest({ state: "done", result, model: input.models[0]?.id ?? null });
    } catch (err) {
      if (mine !== generation.current) return;
      const { status, message } = aiErrorDetail(t, err);
      setTest({ state: "error", status, detail: message ?? aiErrorMessage(t, err) });
    }
  };

  const setRow = (i: number, p: Partial<ModelDraft>) =>
    patchModels(form.models.map((d, j) => (j === i ? { ...d, ...p } : d)));
  const patchModels = (models: ModelDraft[]) => {
    // The test uses the first model, so changing the list can change what it would say.
    setTest(IDLE);
    patch({ models });
  };

  const addTyped = () => {
    const rows = withPending(form.models);
    if (rows !== form.models) patchModels(rows);
    setAdding("");
  };

  const togglePicked = (m: AiModel) =>
    patchModels(form.models.some((d) => d.id.trim() === m.id) ? removeModel(form.models, m.id) : addModels(form.models, [m]));

  const protocolOptions = [
    { value: "chat_completions" as const, label: t("aiSettings.protocol.chat_completions") },
    { value: "anthropic" as const, label: t("aiSettings.protocol.anthropic") },
  ];

  const modelMessage = (): string | null => {
    const p = problems[modelProblem];
    if (!p) return null;
    if (p.id === "empty") return t("aiSettings.models.err.idEmpty");
    if (p.id === "duplicate") return t("aiSettings.models.err.idDuplicate", { id: form.models[modelProblem].id.trim() });
    return t("aiSettings.models.err.tokens");
  };

  return (
    <Sheet
      width={720}
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
      <SheetHeader title={provider ? t("aiSettings.dlg.editTitle") : t("aiSettings.dlg.addTitle")} subtitle={t("aiSettings.dlg.subtitle")} />
      <form
        id={ids.form}
        className={s.form}
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <Group>
          <FormRow label={t("aiSettings.f.name")} htmlFor={ids.name} error={errors.name}>
            <TextField
              id={ids.name}
              autoFocus
              value={form.name}
              invalid={!!errors.name}
              placeholder={t("aiSettings.f.namePlaceholder")}
              onChange={(e) => patch({ name: e.target.value }, "name")}
            />
          </FormRow>
          <FormRow label={t("aiSettings.f.protocol")}>
            <Segmented<AiProtocol>
              ariaLabel={t("aiSettings.f.protocol")}
              value={form.protocol}
              options={protocolOptions}
              onChange={(protocol) => patchConnection({ protocol, protocolChosen: true })}
            />
          </FormRow>
          <FormRow label={t("aiSettings.f.baseUrl")} htmlFor={ids.url} top error={errors.baseUrl}>
            <div className={s.stack}>
              <TextField
                id={ids.url}
                mono
                value={form.baseUrl}
                invalid={!!errors.baseUrl}
                placeholder={PLACEHOLDER_URL[form.protocol]}
                onChange={(e) => setBaseUrl(e.target.value)}
              />
              <span className={s.hint}>{t(`aiSettings.f.baseUrl.${form.protocol}` as MessageKey)}</span>
              <span className={s.hint}>{t("aiSettings.f.baseUrl.https")}</span>
            </div>
          </FormRow>
          <FormRow label={t("aiSettings.f.apiKey")} htmlFor={ids.key}>
            <SavedKeyField
              id={ids.key}
              hasSaved={hasSavedKey}
              draft={form.key}
              placeholder={t("aiSettings.f.apiKeyPlaceholder")}
              onChange={(key) => patchConnection({ key })}
            />
          </FormRow>
          {form.protocol === "anthropic" && (
            <FormRow label={t("aiSettings.f.authHeader")} top>
              <div className={s.stack}>
                <Segmented<AiAuthHeader>
                  ariaLabel={t("aiSettings.f.authHeader")}
                  value={form.authHeader}
                  options={[
                    { value: "x-api-key", label: "x-api-key" },
                    { value: "authorization", label: "Authorization: Bearer" },
                  ]}
                  onChange={(authHeader) => patchConnection({ authHeader })}
                />
                <span className={s.hint}>{t("aiSettings.f.authHeader.hint")}</span>
              </div>
            </FormRow>
          )}
        </Group>

        <Section title={t("aiSettings.models")}>
          <div className={s.modelsBar}>
            <span className={s.hint}>{t("aiSettings.models.hint")}</span>
            <Button size="sm" icon="cloud-arrow-down" busy={fetched.state === "loading"} onClick={() => void fetchModels()}>
              {t("aiSettings.models.fetch")}
            </Button>
          </div>

          {fetched.state === "error" && (
            <div className={s.fetchError} role="alert">
              <Icon name="warning-circle" />
              <div>
                <div>{t("aiSettings.models.fetchFailed")}</div>
                <div className={`${s.fetchDetail} selectable`}>{fetched.text}</div>
              </div>
            </div>
          )}

          {fetched.state === "ok" && (
            <ModelPicker
              list={fetched.list}
              filter={fetched.filter}
              picked={new Set(form.models.map((d) => d.id.trim()))}
              onFilter={(filter) => setFetched({ ...fetched, filter })}
              onToggle={togglePicked}
              onHide={() => setFetched({ state: "idle" })}
            />
          )}

          <Group>
            {form.models.length === 0 ? (
              <div className={s.noModels}>{t("aiSettings.models.empty")}</div>
            ) : (
              <div className={s.table}>
                <div className={cx(s.row, s.head)}>
                  <span>{t("aiSettings.models.col.id")}</span>
                  <span>{t("aiSettings.models.col.name")}</span>
                  <span>{t("aiSettings.models.col.context")}</span>
                  <span>{t("aiSettings.models.col.output")}</span>
                  <span>{t("aiSettings.models.col.effort")}</span>
                  <span />
                </div>
                {form.models.map((d, i) => (
                  <div className={s.row} key={i}>
                    <TextField
                      mono
                      aria-label={t("aiSettings.models.col.id")}
                      value={d.id}
                      invalid={!!problems[i]?.id}
                      onChange={(e) => setRow(i, { id: e.target.value })}
                    />
                    <TextField
                      aria-label={t("aiSettings.models.col.name")}
                      value={d.name}
                      onChange={(e) => setRow(i, { name: e.target.value })}
                    />
                    <TextField
                      mono
                      aria-label={t("aiSettings.models.col.context")}
                      value={d.context}
                      invalid={!!problems[i]?.context}
                      placeholder={t("aiSettings.models.unknown")}
                      onChange={(e) => setRow(i, { context: e.target.value })}
                    />
                    <TextField
                      mono
                      aria-label={t("aiSettings.models.col.output")}
                      value={d.output}
                      invalid={!!problems[i]?.output}
                      placeholder={t("aiSettings.models.unknown")}
                      onChange={(e) => setRow(i, { output: e.target.value })}
                    />
                    <EffortCell id={d.id.trim() || d.name} efforts={d.efforts} onChange={(efforts) => setRow(i, { efforts })} />
                    <IconButton
                      icon="x"
                      size={13}
                      label={t("aiSettings.models.remove", { id: d.id.trim() || d.name })}
                      onClick={() => patchModels(form.models.filter((_, j) => j !== i))}
                    />
                  </div>
                ))}
              </div>
            )}
          </Group>
          {modelProblem >= 0 && (
            <div className={s.problem} role="alert">
              {modelMessage()}
            </div>
          )}

          <div className={s.addRow}>
            <TextField
              mono
              aria-label={t("aiSettings.models.addPlaceholder")}
              value={adding}
              placeholder={t("aiSettings.models.addPlaceholder")}
              onChange={(e) => setAdding(e.target.value)}
              onKeyDown={(e) => {
                if (e.key !== "Enter" || isImeEvent(e)) return;
                // Enter here adds the model; it must not save the whole form.
                e.preventDefault();
                addTyped();
              }}
            />
            <Button size="sm" icon="plus" disabled={!parseModelIds(adding).length} onClick={addTyped}>
              {t("aiSettings.models.add")}
            </Button>
          </div>
        </Section>

        <TestConnection scope="provider" state={test} onRun={() => void runTest()} />

        {errors.general && (
          <div className={s.problem} role="alert">
            {errors.general}
          </div>
        )}
      </form>
    </Sheet>
  );
}

/**
 * AI-05: the thinking levels a model accepts, ticked in a menu. Unknown levels show as what the panel
 * offers for them (Low to High); unticking them all leaves the model with Default only.
 */
function EffortCell({ id, efforts, onChange }: { id: string; efforts: AiEffort[] | null; onChange: (efforts: AiEffort[]) => void }) {
  const t = useT();
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);
  const levels = efforts ?? UNKNOWN_EFFORTS;
  const text = formatEfforts(levels, (e) => t(effortKey(e)), t("aiSettings.models.effort.none"));
  return (
    <>
      <button
        ref={ref}
        type="button"
        className={cx(controlStyles.popup, s.effort)}
        aria-label={t("aiSettings.models.effort.label", { id })}
        aria-haspopup="menu"
        aria-expanded={!!menu.anchor}
        title={text}
        onClick={() => ref.current && menu.openBelow(ref.current, true)}
      >
        <span className={controlStyles.popupLabel}>{text}</span>
        <Icon name="caret-up-down" className={controlStyles.popupCaret} />
      </button>
      {menu.anchor && (
        <Menu
          anchor={menu.anchor}
          onClose={menu.close}
          minWidth={180}
          entries={[
            { kind: "header", label: t("aiSettings.models.effort.menu") },
            ...EFFORTS.map((e) => ({
              label: t(effortKey(e)),
              checked: levels.includes(e),
              keepOpen: true,
              onSelect: () => onChange(toggleEffort(levels, e, !levels.includes(e))),
            })),
          ]}
        />
      )}
    </>
  );
}

/** The fetched model list: tick a model to add it with its limits, untick to remove it. */
function ModelPicker({
  list,
  filter,
  picked,
  onFilter,
  onToggle,
  onHide,
}: {
  list: AiModel[];
  filter: string;
  picked: Set<string>;
  onFilter: (filter: string) => void;
  onToggle: (m: AiModel) => void;
  onHide: () => void;
}) {
  const t = useT();
  const matches = list.filter((m) => matchesFilter(m, filter));
  const shown = matches.slice(0, PICKER_LIMIT);
  return (
    <div className={s.picker}>
      <div className={s.pickerBar}>
        <TextField
          leading={<Icon name="magnifying-glass" />}
          aria-label={t("aiSettings.models.picker.filter")}
          placeholder={t("aiSettings.models.picker.filter")}
          value={filter}
          disabled={list.length === 0}
          onChange={(e) => onFilter(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && e.preventDefault()}
        />
        <span className={s.pickerCount}>{t("aiSettings.models.picker.count", { n: list.length })}</span>
        <Button size="xs" onClick={onHide}>
          {t("aiSettings.models.picker.hide")}
        </Button>
      </div>
      <div className={s.pickerList} role="group" aria-label={t("aiSettings.models")}>
        {list.length === 0 && <div className={s.pickerNote}>{t("aiSettings.models.picker.empty")}</div>}
        {list.length > 0 && matches.length === 0 && <div className={s.pickerNote}>{t("aiSettings.models.picker.noMatch")}</div>}
        {shown.map((m) => {
          const on = picked.has(m.id);
          return (
            <button
              key={m.id}
              type="button"
              role="checkbox"
              aria-checked={on}
              className={cx(s.pickerRow, on && s.pickerRowOn)}
              onClick={() => onToggle(m)}
            >
              <span className={cx(controlStyles.checkboxBox, on && s.boxOn)}>{on && <Icon name="check" />}</span>
              <span className={s.pickerName}>{m.name || m.id}</span>
              {m.name && m.name !== m.id && <span className={s.pickerId}>{m.id}</span>}
              <span className={s.pickerLimit}>{formatTokenCount(m.context_window)}</span>
            </button>
          );
        })}
        {matches.length > shown.length && (
          <div className={s.pickerNote}>{t("aiSettings.models.picker.more", { n: PICKER_LIMIT })}</div>
        )}
      </div>
    </div>
  );
}
