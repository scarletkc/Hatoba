import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp, type HostFilter } from "@/app/store";
import { Button, Icon, IconButton, LinkButton, Segmented, StatusDot, TextArea, TextField } from "@/components/controls";
import { EmptyState, FormRow, Group, Section, TagInput } from "@/components/layout";
import { PopupSelect, confirm, toast, type SelectOption } from "@/components/overlay";
import { InstructionsField } from "@/features/ai/InstructionsField";
import { charCount, HOST_NOTES_MAX_CHARS } from "@/features/ai/instructions";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AuthKind, HostInput, HostView, KeyView, QuickTarget } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { ENV_FOCUS_ID, EnvVarsSection } from "./EnvVarsSection";
import { envInput, envRows, validateEnvRows, type EnvRow } from "./envVars";
import { ForwardsSection } from "./ForwardsSection";
import { useHostsUi } from "./ui";
import s from "./HostEditPage.module.css";

interface Form {
  name: string;
  address: string;
  port: string;
  username: string;
  authKind: AuthKind;
  /** Only used when typing a new/replacement password. The saved password is never loaded (HOST-08). */
  password: string;
  replacing: boolean;
  keyId: string | null;
  groupId: string | null;
  tags: string[];
  jumpHostId: string | null;
  note: string;
  /** What the AI assistant is told about the host (AI-37). */
  aiNotes: string;
  favorite: boolean;
  /** Environment variables sent when a terminal opens (SSH-14). */
  env: EnvRow[];
}

type FieldKey = "name" | "address" | "port" | "username" | "password" | "key" | "jump" | "aiNotes" | "env";
type Errors = Partial<Record<FieldKey, string>>;

type TestState =
  | { state: "idle" }
  | { state: "running" }
  | { state: "ok"; text: string }
  | { state: "error"; text: string };

const BACKEND_FIELDS: Record<string, FieldKey> = {
  name: "name",
  address: "address",
  port: "port",
  username: "username",
  password: "password",
  key_id: "key",
  jump_host_id: "jump",
  ai_notes: "aiNotes",
  env: "env",
};

function initialForm(host: HostView | undefined, groupId: string | null, prefill: QuickTarget | undefined): Form {
  return {
    name: host?.name ?? prefill?.address ?? "",
    address: host?.address ?? prefill?.address ?? "",
    port: String(host?.port ?? prefill?.port ?? 22),
    username: host?.username ?? prefill?.username ?? "",
    authKind: host?.auth_kind ?? "password",
    password: "",
    replacing: false,
    keyId: host?.key_id ?? null,
    groupId: host ? host.group_id : groupId,
    tags: host?.tags ?? [],
    jumpHostId: host?.jump_host_id ?? null,
    note: host?.note ?? "",
    aiNotes: host?.ai_notes ?? "",
    favorite: host?.favorite ?? false,
    env: envRows(host?.env ?? []),
  };
}

function keyHint(k: KeyView): string {
  return k.algorithm === "rsa" ? `RSA ${k.bits}` : k.algorithm.toUpperCase();
}

export function HostEditPage({
  hostId,
  groupId,
  back,
  prefill,
}: {
  hostId: string | null;
  groupId: string | null;
  back: HostFilter;
  /** A quick-connect target the new host starts from (HOST-12). */
  prefill?: QuickTarget;
}) {
  const t = useT();
  const navigate = useApp((st) => st.navigate);
  const hosts = useVaultData((st) => st.hosts);
  const groups = useVaultData((st) => st.groups);
  const keys = useVaultData((st) => st.keys);
  const tagList = useVaultData((st) => st.tags);

  const host = useMemo(() => hosts.find((h) => h.id === hostId), [hosts, hostId]);
  const [form, setForm] = useState<Form>(() => initialForm(host, groupId, prefill));
  const [errors, setErrors] = useState<Errors>({});
  const [userTouched, setUserTouched] = useState(false);
  // Save or Test Connection was tried: empty required fields show as problems from now on.
  const [submitted, setSubmitted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [test, setTest] = useState<TestState>({ state: "idle" });
  const passwordRef = useRef<HTMLInputElement>(null);

  const envProblems = validateEnvRows(form.env);
  const envBlocked = envProblems.some(Boolean);
  const hasSavedPassword = !!host?.has_password;
  const showSaved = form.authKind === "password" && hasSavedPassword && !form.replacing;

  const patch = (p: Partial<Form>, field?: FieldKey) => {
    setForm((f) => ({ ...f, ...p }));
    if (field) setErrors((e) => (e[field] ? { ...e, [field]: undefined } : e));
    // A test result describes the old connection settings.
    if (Object.keys(p).some((k) => ["address", "port", "username", "authKind", "password", "keyId", "jumpHostId", "replacing"].includes(k)))
      setTest({ state: "idle" });
  };

  const goBack = () => navigate({ kind: "hosts", filter: back });

  const validate = (): Errors => {
    const e: Errors = {};
    if (!form.name.trim()) e.name = t("hosts.err.nameRequired");
    if (!form.address.trim()) e.address = t("hosts.err.addressRequired");
    const port = Number(form.port);
    if (!/^\d+$/.test(form.port.trim()) || port < 1 || port > 65535) e.port = t("hosts.err.portInvalid");
    if (form.authKind === "key" && !form.keyId) e.key = t("hosts.err.keyRequired");
    if (form.jumpHostId) {
      if (form.jumpHostId === hostId) e.jump = t("hosts.err.jumpSelf");
      else if (hostId && jumpChainReaches(hosts, form.jumpHostId, hostId)) e.jump = t("hosts.err.jumpLoop");
    }
    // The field says so itself; this only keeps the form from saving.
    if (charCount(form.aiNotes) > HOST_NOTES_MAX_CHARS) e.aiNotes = t("ai.instructions.tooLong", { max: HOST_NOTES_MAX_CHARS.toLocaleString(t.locale) });
    return e;
  };

  const toInput = (): HostInput => ({
    id: hostId,
    name: form.name.trim(),
    address: form.address.trim(),
    port: Number(form.port),
    username: form.username.trim(),
    auth_kind: form.authKind,
    password: form.authKind === "password" && form.password ? form.password : null,
    key_id: form.authKind === "key" ? form.keyId : null,
    group_id: form.groupId,
    tags: form.tags,
    favorite: form.favorite,
    jump_host_id: form.jumpHostId,
    note: form.note,
    ai_notes: form.aiNotes,
    env: envInput(form.env),
  });

  const showErrors = (e: Errors) => {
    setErrors(e);
    setUserTouched(true);
    setSubmitted(true);
    const first = (["name", "address", "port"] as const).find((k) => e[k]);
    if (first) document.getElementById(`host-${first}`)?.focus();
    // After the render that shows the rows' problems, so the field to focus has its id.
    else if (envBlocked || e.env) requestAnimationFrame(() => document.getElementById(ENV_FOCUS_ID)?.focus());
    else if (e.aiNotes) document.getElementById("host-aiNotes")?.focus();
  };

  const save = async () => {
    if (saving) return;
    const e = validate();
    if (Object.values(e).some(Boolean) || envBlocked) return showErrors(e);
    setSaving(true);
    try {
      const saved = await api.host_save(toInput());
      await useVaultData.getState().reload();
      useHostsUi.getState().select(saved.id);
      goBack();
    } catch (err) {
      const ae = toAppError(err);
      const field = ae.code === "invalid_input" && ae.field ? BACKEND_FIELDS[ae.field] : undefined;
      if (field) showErrors({ [field]: errorMessage(t, err) });
      else toast(errorMessage(t, err, { host: form.address, port: Number(form.port) }), "error");
      setSaving(false);
    }
  };

  const runTest = async () => {
    const e = validate();
    if (e.address || e.port || e.key || e.name || e.aiNotes || envBlocked) return showErrors(e);
    setTest({ state: "running" });
    try {
      const r = await api.ssh_test(toInput());
      if (r.ok) {
        const base = r.host_key_verified ? t("hosts.test.ok") : t("hosts.test.okUnverified");
        setTest({ state: "ok", text: r.latency_ms !== null ? `${base} · ${r.latency_ms} ms` : base });
      } else {
        setTest({
          state: "error",
          text: errorMessage(t, r.error ?? { code: "ssh", detail: "" }, { host: form.address, port: Number(form.port) }),
        });
      }
    } catch (err) {
      setTest({ state: "error", text: errorMessage(t, err, { host: form.address, port: Number(form.port) }) });
    }
  };

  const remove = async () => {
    if (!host) return;
    const ok = await confirm({
      title: t("hosts.deleteTitle", { name: host.name }),
      body: t("hosts.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.host_delete(host.id);
      await useVaultData.getState().reload();
      toast(t("hosts.deleted", { name: host.name }));
      goBack();
    } catch (err) {
      toast(errorMessage(t, err), "error");
    }
  };

  // Ctrl+S saves; Escape leaves an untouched form. Menus and dialogs get the key first.
  const initialJson = useRef(JSON.stringify(form));
  const dirty = JSON.stringify(form) !== initialJson.current;
  const latest = useRef({ save, goBack, dirty });
  latest.current = { save, goBack, dirty };
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.defaultPrevented || document.querySelector('[role="dialog"],[role="alertdialog"],[role="menu"]')) return;
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void latest.current.save();
      } else if (e.key === "Escape" && !latest.current.dirty) {
        latest.current.goBack();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Enter in a single-line field saves (tag input handles its own Enter).
  const onKeyDown = (e: KeyboardEvent) => {
    const target = e.target as HTMLElement;
    if (e.key === "Enter" && !e.defaultPrevented && target.tagName === "INPUT" && target.getAttribute("type") !== "checkbox") {
      e.preventDefault();
      void save();
    }
  };

  const keyOptions: SelectOption<string | null>[] = keys.map((k) => ({ value: k.id, label: k.name, hint: keyHint(k) }));
  const groupOptions: SelectOption<string | null>[] = [
    { value: null, label: t("hosts.f.none") },
    ...groups.map((g) => ({ value: g.id, label: g.name })),
  ];
  const jumpOptions: SelectOption<string | null>[] = useMemo(
    () => [
      { value: null, label: t("hosts.f.none") },
      ...hosts
        .filter((h) => h.id !== hostId)
        .map((h) => ({ value: h.id, label: h.name, hint: `${h.username ? `${h.username}@` : ""}${h.address}:${h.port}` })),
    ],
    [hosts, hostId, t],
  );

  if (hostId && !host) {
    return (
      <div className={s.page}>
        <EmptyState
          icon="hard-drives"
          title={t("hosts.notFound.title")}
          body={t("hosts.notFound.body")}
          actions={<Button onClick={goBack}>{t("btn.back")}</Button>}
        />
      </div>
    );
  }

  const groupName = groups.find((g) => g.id === form.groupId)?.name ?? t("sidebar.all");
  const methodOptions = [
    { value: "password" as const, label: t("hosts.auth.password"), icon: "password" },
    { value: "key" as const, label: t("hosts.auth.key"), icon: "key" },
    { value: "ask" as const, label: t("hosts.auth.ask"), icon: "keyboard" },
    { value: "agent" as const, label: t("hosts.auth.agent"), icon: "identification-badge" },
  ];

  return (
    <div className={s.page} onKeyDown={onKeyDown}>
      <div className={s.bar}>
        <IconButton icon="caret-left" label={t("btn.back")} className={s.back} onClick={goBack} />
        <span className={s.crumbGroup}>{groupName}</span>
        <span className={s.crumbSep}>/</span>
        <span className={s.crumbName}>{form.name.trim() || t("hosts.newHost")}</span>
        <div className={s.spacer} />
        <button
          type="button"
          className={cx(s.favButton, form.favorite && s.favOn)}
          aria-pressed={form.favorite}
          title={form.favorite ? t("hosts.menu.unfavorite") : t("hosts.menu.favorite")}
          aria-label={form.favorite ? t("hosts.menu.unfavorite") : t("hosts.menu.favorite")}
          onClick={() => patch({ favorite: !form.favorite })}
        >
          <Icon name="star" fill={form.favorite} />
        </button>
        <Button onClick={goBack}>{t("btn.cancel")}</Button>
        <Button variant="primary" busy={saving} onClick={() => void save()}>
          {t("btn.save")}
        </Button>
      </div>

      <div className={s.scroll}>
        <div className={s.column}>
          <Section title={t("hosts.edit.sec.basic")}>
            <Group>
              <FormRow label={t("hosts.f.name")} htmlFor="host-name" error={errors.name}>
                <TextField
                  id="host-name"
                  autoFocus
                  value={form.name}
                  invalid={!!errors.name}
                  placeholder={t("hosts.f.namePlaceholder")}
                  onChange={(e) => patch({ name: e.target.value }, "name")}
                />
              </FormRow>
              <FormRow label={t("hosts.f.address")} htmlFor="host-address" error={[errors.address, errors.port].filter(Boolean).join(" ") || null}>
                <div className={s.addrRow}>
                  <TextField
                    id="host-address"
                    mono
                    value={form.address}
                    invalid={!!errors.address}
                    placeholder={t("hosts.f.addressPlaceholder")}
                    onChange={(e) => patch({ address: e.target.value }, "address")}
                  />
                  <label className={s.portLabel} htmlFor="host-port">
                    {t("hosts.f.port")}
                  </label>
                  <TextField
                    id="host-port"
                    mono
                    inputMode="numeric"
                    value={form.port}
                    invalid={!!errors.port}
                    onChange={(e) => patch({ port: e.target.value }, "port")}
                  />
                </div>
              </FormRow>
              <FormRow label={t("hosts.f.user")} htmlFor="host-username" error={errors.username}>
                <div>
                  <TextField
                    id="host-username"
                    mono
                    value={form.username}
                    invalid={!!errors.username}
                    onBlur={() => setUserTouched(true)}
                    onChange={(e) => patch({ username: e.target.value }, "username")}
                  />
                  {userTouched && !form.username.trim() && !errors.username && (
                    <div className={s.warn}>
                      <Icon name="warning" size={13} />
                      {t("hosts.f.userWarn")}
                    </div>
                  )}
                </div>
              </FormRow>
            </Group>
          </Section>

          <Section title={t("hosts.edit.sec.auth")}>
            <Group>
              <FormRow label={t("hosts.f.method")}>
                <Segmented
                  ariaLabel={t("hosts.f.method")}
                  value={form.authKind}
                  options={methodOptions}
                  onChange={(authKind) => patch({ authKind }, "key")}
                />
              </FormRow>

              {form.authKind === "password" && (
                <FormRow label={t("hosts.f.password")} htmlFor="host-password" error={errors.password}>
                  {showSaved ? (
                    <div className={s.savedRow}>
                      <div className={s.savedField}>
                        <Icon name="check-circle" fill />
                        {t("hosts.f.passwordSaved")}
                      </div>
                      <Button
                        size="sm"
                        onClick={() => {
                          patch({ replacing: true });
                          requestAnimationFrame(() => passwordRef.current?.focus());
                        }}
                      >
                        {t("hosts.f.replace")}
                      </Button>
                    </div>
                  ) : (
                    <div className={s.passwordRow}>
                      <TextField
                        id="host-password"
                        ref={passwordRef}
                        secret
                        autoComplete="new-password"
                        value={form.password}
                        invalid={!!errors.password}
                        placeholder={form.replacing ? t("hosts.f.passwordNewPlaceholder") : t("hosts.f.passwordPlaceholder")}
                        onChange={(e) => patch({ password: e.target.value }, "password")}
                      />
                      {form.replacing && (
                        <LinkButton onClick={() => patch({ replacing: false, password: "" })}>{t("hosts.f.keepSaved")}</LinkButton>
                      )}
                    </div>
                  )}
                </FormRow>
              )}

              {form.authKind === "key" && (
                <FormRow label={t("hosts.f.key")} error={errors.key}>
                  <div className={s.keyRow}>
                    <div className={s.fill}>
                      <PopupSelect
                        ariaLabel={t("hosts.f.key")}
                        icon="key"
                        value={form.keyId}
                        options={keyOptions}
                        disabled={keys.length === 0}
                        placeholder={keys.length === 0 ? t("hosts.f.noKeys") : t("hosts.f.keyPlaceholder")}
                        minWidth={240}
                        onChange={(keyId) => patch({ keyId }, "key")}
                      />
                    </div>
                    <LinkButton onClick={() => navigate({ kind: "keys" })}>{t("hosts.f.manageKeys")}</LinkButton>
                  </div>
                </FormRow>
              )}

              {form.authKind === "ask" && (
                <FormRow label="">
                  <span className={s.hint}>{t("hosts.f.askHint")}</span>
                </FormRow>
              )}
              {form.authKind === "agent" && (
                <FormRow label="">
                  <span className={s.hint}>{t("hosts.f.agentHint")}</span>
                </FormRow>
              )}
            </Group>
          </Section>

          <Section title={t("hosts.edit.sec.org")}>
            <Group>
              <FormRow label={t("hosts.f.group")}>
                <div>
                  <PopupSelect
                    ariaLabel={t("hosts.f.group")}
                    icon="folder-simple"
                    value={form.groupId}
                    options={groupOptions}
                    minWidth={200}
                    onChange={(groupId) => patch({ groupId })}
                  />
                </div>
              </FormRow>
              <FormRow label={t("hosts.f.tags")}>
                <TagInput
                  value={form.tags}
                  placeholder={t("hosts.f.addTag")}
                  suggestions={tagList.map((x) => x.name)}
                  onChange={(tags) => patch({ tags })}
                />
              </FormRow>
              <FormRow label={t("hosts.f.jump")} top error={errors.jump}>
                <div className={s.jump}>
                  <PopupSelect
                    ariaLabel={t("hosts.f.jump")}
                    icon="path"
                    value={form.jumpHostId}
                    options={jumpOptions}
                    minWidth={340}
                    onChange={(jumpHostId) => patch({ jumpHostId }, "jump")}
                  />
                  <span className={s.jumpHint}>{t("hosts.f.jumpHint")}</span>
                </div>
              </FormRow>
            </Group>
          </Section>

          <ForwardsSection hostId={hostId} />

          <EnvVarsSection
            rows={form.env}
            problems={envProblems}
            showEmpty={submitted}
            error={errors.env}
            onChange={(env) => patch({ env }, "env")}
          />

          <Section title={t("hosts.edit.sec.note")}>
            <TextArea
              aria-label={t("hosts.f.note")}
              value={form.note}
              rows={3}
              onChange={(e) => patch({ note: e.target.value })}
            />
          </Section>

          <Section title={t("hosts.edit.sec.ai")}>
            <div className={s.aiNotes}>
              <span className={s.hint}>{t("hosts.f.aiNotesHint")}</span>
              <InstructionsField
                id="host-aiNotes"
                label={t("hosts.f.aiNotes")}
                value={form.aiNotes}
                max={HOST_NOTES_MAX_CHARS}
                rows={4}
                placeholder={t("hosts.f.aiNotesPlaceholder")}
                error={errors.aiNotes}
                onChange={(aiNotes) => patch({ aiNotes }, "aiNotes")}
              />
              <span className={s.aiPrivacy}>{t("hosts.f.aiNotesPrivacy")}</span>
            </div>
          </Section>

          <div className={s.footer}>
            <Button icon="plugs-connected" busy={test.state === "running"} onClick={() => void runTest()}>
              {t("hosts.test")}
            </Button>
            {test.state !== "idle" && (
              <span className={cx(s.testResult, test.state === "error" && s.testError)} role="status">
                {test.state === "running" ? (
                  t("hosts.test.running")
                ) : (
                  <>
                    <StatusDot color={test.state === "ok" ? "var(--green)" : "var(--red)"} />
                    {test.text}
                  </>
                )}
              </span>
            )}
            <div className={s.spacer} />
            {host && (
              <LinkButton tone="danger" onClick={() => void remove()}>
                {t("hosts.delete")}
              </LinkButton>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/** True if following jump-host links from `start` ever reaches `target` (which would make a loop). */
function jumpChainReaches(hosts: HostView[], start: string, target: string): boolean {
  const byId = new Map(hosts.map((h) => [h.id, h]));
  const seen = new Set<string>();
  let cur: string | null = start;
  while (cur && !seen.has(cur)) {
    if (cur === target) return true;
    seen.add(cur);
    cur = byId.get(cur)?.jump_host_id ?? null;
  }
  return false;
}
