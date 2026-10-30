import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import {
  Button,
  EmptyState,
  FormRow,
  Group,
  HintText,
  Icon,
  IconButton,
  LinkButton,
  PopupSelect,
  Section,
  TagInput,
  TextArea,
  TextField,
  WarningText,
  confirm,
  isImeEvent,
  toast,
  type SelectOption,
} from "@hatoba/ui";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { InstructionsField } from "@/features/ai/InstructionsField";
import { byteCount, HOST_NOTES_MAX_BYTES } from "@/features/ai/instructions";
import { proxySummary } from "@/features/settings/proxyLogic";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { ProxyMode, TelnetHostInput, TelnetHostView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { ProtocolRow, type HostDraft } from "./ProtocolRow";
import { useHostsUi } from "./ui";
import s from "./HostEditPage.module.css";

interface Form {
  name: string;
  address: string;
  port: string;
  /** Typed at the login prompt (TELNET-03); empty to type it by hand. */
  username: string;
  /** Only a new or replacement password. The saved one is never loaded (HOST-08). */
  password: string;
  replacing: boolean;
  /** The saved password goes when the host is saved. */
  forget: boolean;
  groupId: string | null;
  tags: string[];
  proxyMode: ProxyMode;
  proxyId: string | null;
  note: string;
  aiNotes: string;
  favorite: boolean;
}

type FieldKey = "name" | "address" | "port" | "username" | "password" | "proxy" | "aiNotes";
type Errors = Partial<Record<FieldKey, string>>;

const BACKEND_FIELDS: Record<string, FieldKey> = {
  name: "name",
  address: "address",
  port: "port",
  username: "username",
  password: "password",
  proxy_id: "proxy",
  ai_notes: "aiNotes",
};

function initialForm(host: TelnetHostView | undefined, groupId: string | null, draft: HostDraft | undefined): Form {
  return {
    name: host?.name ?? draft?.name ?? "",
    address: host?.address ?? draft?.address ?? "",
    port: host ? String(host.port) : (draft?.port ?? "23"),
    username: host?.username ?? draft?.username ?? "",
    password: draft?.password ?? "",
    replacing: false,
    forget: false,
    groupId: host ? host.group_id : groupId,
    tags: host?.tags ?? draft?.tags ?? [],
    proxyMode: host?.proxy_mode ?? draft?.proxyMode ?? "device_default",
    proxyId: host?.proxy_id ?? draft?.proxyId ?? null,
    note: host?.note ?? draft?.note ?? "",
    aiNotes: host?.ai_notes ?? draft?.aiNotes ?? "",
    favorite: host?.favorite ?? draft?.favorite ?? false,
  };
}

/** Creates or edits a Telnet host (TELNET-01), laid out as the SSH host editor is. */
export function TelnetEditPage({
  hostId,
  groupId,
  back,
  draft,
}: {
  hostId: string | null;
  groupId: string | null;
  /** The group whose hosts Back and Save return to, or null for every host. */
  back: string | null;
  /** What a new host carries over from the SSH editor's form, when the protocol was switched. */
  draft?: HostDraft;
}) {
  const t = useT();
  const navigate = useApp((st) => st.navigate);
  const telnetHosts = useVaultData((st) => st.telnetHosts);
  const groups = useVaultData((st) => st.groups);
  const proxies = useVaultData((st) => st.proxies);
  const defaultProxyId = useApp((st) => st.prefs.default_proxy_id);
  const tagList = useVaultData((st) => st.tags);

  const host = useMemo(() => telnetHosts.find((h) => h.id === hostId), [telnetHosts, hostId]);
  const [form, setForm] = useState<Form>(() => initialForm(host, groupId, draft));
  const [errors, setErrors] = useState<Errors>({});
  const [saving, setSaving] = useState(false);
  const passwordRef = useRef<HTMLInputElement>(null);
  const showSaved = !!host?.has_password && !form.replacing && !form.forget;

  const patch = (p: Partial<Form>, field?: FieldKey) => {
    setForm((f) => ({ ...f, ...p }));
    if (field) setErrors((e) => (e[field] ? { ...e, [field]: undefined } : e));
  };

  const goBack = () => navigate({ kind: "hosts", group: back });

  const validate = (): Errors => {
    const e: Errors = {};
    if (!form.address.trim()) e.address = t("hosts.err.addressRequired");
    const port = Number(form.port);
    if (!/^\d+$/.test(form.port.trim()) || port < 1 || port > 65535) e.port = t("hosts.err.portInvalid");
    if (form.proxyMode === "proxy" && !proxies.some((p) => p.id === form.proxyId))
      e.proxy = t("hosts.err.proxyDeleted");
    if (byteCount(form.aiNotes) > HOST_NOTES_MAX_BYTES)
      e.aiNotes = t("ai.instructions.tooLong", { max: HOST_NOTES_MAX_BYTES.toLocaleString(t.locale) });
    return e;
  };

  const toInput = (): TelnetHostInput => ({
    id: hostId,
    name: form.name.trim(),
    address: form.address.trim(),
    port: Number(form.port),
    username: form.username.trim(),
    // HOST-08: null keeps the saved password; an empty one removes it.
    password: form.password ? form.password : form.forget ? "" : null,
    group_id: form.groupId,
    tags: form.tags,
    favorite: form.favorite,
    proxy_mode: form.proxyMode,
    proxy_id: form.proxyMode === "proxy" ? form.proxyId : null,
    note: form.note,
    ai_notes: form.aiNotes,
  });

  const showErrors = (e: Errors) => {
    setErrors(e);
    const first = (["name", "address", "port", "username", "password", "aiNotes"] as const).find((k) => e[k]);
    if (first) document.getElementById(`telnet-${first}`)?.focus();
  };

  const save = async () => {
    if (saving) return;
    const e = validate();
    if (Object.values(e).some(Boolean)) return showErrors(e);
    setSaving(true);
    try {
      const saved = await api.telnet_host_save(toInput());
      await useVaultData.getState().reload();
      useHostsUi.getState().select([saved.id]);
      goBack();
    } catch (err) {
      const ae = toAppError(err);
      const field = ae.code === "invalid_input" && ae.field ? BACKEND_FIELDS[ae.field] : undefined;
      if (field) showErrors({ [field]: errorMessage(t, err) });
      else if (ae.code === "invalid_input" && ae.field === "host") toast(t("hosts.err.tooLarge"), "error");
      else toast(errorMessage(t, err), "error");
      setSaving(false);
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
      await api.telnet_host_delete(host.id);
      await useVaultData.getState().reload();
      toast(t("hosts.deleted", { name: host.name }));
      goBack();
    } catch (err) {
      toast(errorMessage(t, err), "error");
    }
  };

  // Ctrl+S saves; Escape leaves an untouched form, as in the SSH host editor.
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

  const onKeyDown = (e: KeyboardEvent) => {
    const target = e.target as HTMLElement;
    if (
      e.key === "Enter" &&
      !isImeEvent(e) &&
      !e.defaultPrevented &&
      target.tagName === "INPUT" &&
      target.getAttribute("type") !== "checkbox"
    ) {
      e.preventDefault();
      void save();
    }
  };

  const groupOptions: SelectOption<string | null>[] = [
    { value: null, label: t("hosts.f.none") },
    ...groups.map((g) => ({ value: g.id, label: g.name })),
  ];
  const proxyValue = form.proxyMode === "proxy" ? `proxy:${form.proxyId ?? ""}` : form.proxyMode;
  const defaultProxy = proxies.find((p) => p.id === defaultProxyId);
  const defaultHint = defaultProxy?.name ?? t(defaultProxyId ? "hosts.f.proxyDeleted" : "hosts.f.proxyDirect");
  const proxyOptions: SelectOption<string>[] = [
    { value: "device_default", label: t("hosts.f.proxyDefault"), hint: defaultHint },
    { value: "direct", label: t("hosts.f.proxyDirect") },
    ...proxies.map((p) => ({ value: `proxy:${p.id}`, label: p.name, hint: proxySummary(p) })),
    ...(form.proxyMode === "proxy" && !proxies.some((p) => p.id === form.proxyId)
      ? [{ value: proxyValue, label: t("hosts.f.proxyDeleted") }]
      : []),
  ];
  const setProxy = (value: string) =>
    value.startsWith("proxy:")
      ? patch({ proxyMode: "proxy", proxyId: value.slice("proxy:".length) }, "proxy")
      : patch({ proxyMode: value as ProxyMode, proxyId: null }, "proxy");

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

  const groupName = groups.find((g) => g.id === form.groupId)?.name ?? t("hosts.title");
  // A new host switches to the SSH editor with what it has typed so far.
  const toSsh = () =>
    navigate({
      kind: "host-edit",
      hostId: null,
      groupId: form.groupId,
      back,
      draft: {
        name: form.name,
        address: form.address,
        port: form.port.trim() === "23" ? undefined : form.port,
        username: form.username,
        password: form.password,
        tags: form.tags,
        proxyMode: form.proxyMode,
        proxyId: form.proxyId,
        note: form.note,
        aiNotes: form.aiNotes,
        favorite: form.favorite,
      },
    });

  return (
    <div className={s.page} onKeyDown={onKeyDown}>
      <div className={s.bar}>
        <IconButton icon="caret-left" label={t("btn.back")} className={s.back} onClick={goBack} />
        <span className={s.crumbGroup}>{groupName}</span>
        <span className={s.crumbSep}>/</span>
        <span className={s.crumbName}>{form.name.trim() || t("hosts.newHost")}</span>
        <div className={s.spacer} />
        <IconButton
          icon="star"
          fill={form.favorite}
          className={cx(s.favButton, form.favorite && s.favOn)}
          aria-pressed={form.favorite}
          label={form.favorite ? t("hosts.menu.unfavorite") : t("hosts.menu.favorite")}
          onClick={() => patch({ favorite: !form.favorite })}
        />
        <Button onClick={goBack}>{t("btn.cancel")}</Button>
        <Button variant="primary" busy={saving} onClick={() => void save()}>
          {t("btn.save")}
        </Button>
      </div>

      <div className={s.scroll}>
        <div className={s.column}>
          <Section title={t("hosts.edit.sec.basic")}>
            <Group>
              {!hostId && <ProtocolRow value="telnet" onChange={(p) => p === "ssh" && toSsh()} />}
              <FormRow label={t("hosts.f.name")} htmlFor="telnet-name" error={errors.name}>
                <TextField
                  id="telnet-name"
                  autoFocus
                  value={form.name}
                  invalid={!!errors.name}
                  placeholder={form.address.trim() || t("hosts.f.namePlaceholder")}
                  onChange={(e) => patch({ name: e.target.value }, "name")}
                />
              </FormRow>
              <FormRow
                label={t("hosts.f.address")}
                htmlFor="telnet-address"
                error={[errors.address, errors.port].filter(Boolean).join(" ") || null}
              >
                <div className={s.addrRow}>
                  <TextField
                    id="telnet-address"
                    mono
                    value={form.address}
                    invalid={!!errors.address}
                    placeholder={t("hosts.f.addressPlaceholder")}
                    onChange={(e) => patch({ address: e.target.value }, "address")}
                  />
                  <label className={s.portLabel} htmlFor="telnet-port">
                    {t("hosts.f.port")}
                  </label>
                  <TextField
                    id="telnet-port"
                    mono
                    inputMode="numeric"
                    value={form.port}
                    invalid={!!errors.port}
                    onChange={(e) => patch({ port: e.target.value }, "port")}
                  />
                </div>
              </FormRow>
            </Group>
          </Section>

          <Section title={t("hosts.edit.sec.login")}>
            <Group>
              <FormRow label={t("hosts.f.user")} htmlFor="telnet-username" error={errors.username}>
                <TextField
                  id="telnet-username"
                  mono
                  value={form.username}
                  invalid={!!errors.username}
                  onChange={(e) => patch({ username: e.target.value }, "username")}
                />
              </FormRow>
              <FormRow label={t("hosts.f.password")} htmlFor="telnet-password" error={errors.password}>
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
                    <LinkButton onClick={() => patch({ forget: true })}>{t("hosts.f.forgetPassword")}</LinkButton>
                  </div>
                ) : (
                  <div className={s.passwordRow}>
                    <TextField
                      id="telnet-password"
                      ref={passwordRef}
                      secret
                      autoComplete="new-password"
                      value={form.password}
                      invalid={!!errors.password}
                      placeholder={
                        form.replacing ? t("hosts.f.passwordNewPlaceholder") : t("hosts.f.telnetPasswordPlaceholder")
                      }
                      onChange={(e) => patch({ password: e.target.value }, "password")}
                    />
                    {(form.replacing || form.forget) && (
                      <LinkButton onClick={() => patch({ replacing: false, forget: false, password: "" })}>
                        {t("hosts.f.keepSaved")}
                      </LinkButton>
                    )}
                  </div>
                )}
              </FormRow>
              <FormRow label="">
                <div className={s.jump}>
                  <HintText>{t("hosts.f.telnetLoginHint")}</HintText>
                  <WarningText>{t("hosts.f.telnetPlaintext")}</WarningText>
                </div>
              </FormRow>
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
              <FormRow label={t("hosts.f.proxy")} top error={errors.proxy}>
                <div className={s.keyRow}>
                  <div className={s.fill}>
                    <PopupSelect<string>
                      ariaLabel={t("hosts.f.proxy")}
                      icon="globe"
                      value={proxyValue}
                      options={proxyOptions}
                      showHint={form.proxyMode === "device_default"}
                      minWidth={340}
                      onChange={setProxy}
                    />
                  </div>
                  <LinkButton onClick={() => useApp.getState().openSettings(true, "proxies")}>
                    {t("hosts.f.manageProxies")}
                  </LinkButton>
                </div>
              </FormRow>
            </Group>
          </Section>

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
              <HintText>{t("hosts.f.aiNotesHint")}</HintText>
              <InstructionsField
                id="telnet-aiNotes"
                label={t("hosts.f.aiNotes")}
                value={form.aiNotes}
                max={HOST_NOTES_MAX_BYTES}
                rows={4}
                placeholder={t("hosts.f.aiNotesPlaceholder")}
                error={errors.aiNotes}
                onChange={(aiNotes) => patch({ aiNotes }, "aiNotes")}
              />
            </div>
          </Section>

          {host && (
            <div className={s.footer}>
              <div className={s.spacer} />
              <LinkButton tone="danger" onClick={() => void remove()}>
                {t("hosts.delete")}
              </LinkButton>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
