import { useId, useState } from "react";
import { errorMessage } from "@/app/errors";
import { Button, Segmented, TextField } from "@/components/controls";
import { FormRow, Group } from "@/components/layout";
import { FooterSpacer, Sheet, SheetHeader } from "@/components/overlay";
import { useT, type MessageKey } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { ProxyInput, ProxyKind, ProxyView } from "@/ipc/types";
import type { KeyDraft } from "./aiLogic";
import { SavedKeyField } from "./aiShared";
import { DEFAULT_PORT, KIND_LABEL, parseProxyUrl, validateProxy, type ProxyField, type ProxyFormValues } from "./proxyLogic";
import s from "./ProxyDialog.module.css";

interface Form extends Omit<ProxyFormValues, "password"> {
  password: KeyDraft;
}

type Errors = Partial<Record<ProxyField | "general", string>>;

const BACKEND_FIELDS: Record<string, ProxyField> = {
  name: "name",
  address: "address",
  port: "port",
  username: "username",
  password: "password",
};

function initialForm(p: ProxyView | null): Form {
  return {
    name: p?.name ?? "",
    kind: p?.kind ?? "socks5",
    address: p?.address ?? "",
    port: String(p?.port ?? DEFAULT_PORT.socks5),
    username: p?.username ?? "",
    password: { mode: "keep", value: "" },
  };
}

/** Add or edit a SOCKS5 or HTTP proxy (SSH-13). A saved password behaves like a host's (HOST-08). */
export function ProxyDialog({ proxy, onClose, onSaved }: { proxy: ProxyView | null; onClose: () => void; onSaved: (p: ProxyView) => void }) {
  const t = useT();
  const ids = { form: useId(), name: useId(), address: useId(), port: useId(), username: useId(), password: useId() };
  const [form, setForm] = useState<Form>(() => initialForm(proxy));
  const [errors, setErrors] = useState<Errors>({});
  const [saving, setSaving] = useState(false);
  const hasSaved = !!proxy?.has_password;
  const typedPassword = form.password.mode === "replace" ? form.password.value : "";

  const patch = (p: Partial<Form>, ...fields: ProxyField[]) => {
    setForm((f) => ({ ...f, ...p }));
    setErrors((e) => {
      const next = { ...e, general: undefined };
      for (const f of fields) next[f] = undefined;
      return next;
    });
  };

  const setKind = (kind: ProxyKind) => {
    // A port still at the other type's default follows the type.
    const port = form.port.trim() === String(DEFAULT_PORT[form.kind]) ? String(DEFAULT_PORT[kind]) : form.port;
    patch({ kind, port }, "port", "username");
  };

  /**
   * A proxy URL pasted or typed into the address fills in the type, port, and sign-in as well. It is
   * read on paste and when the field loses focus, not while typing, where `http://a` is already one.
   */
  const takeUrl = (text: string): boolean => {
    const url = parseProxyUrl(text);
    if (!url) return false;
    patch(
      {
        kind: url.kind,
        address: url.address,
        port: url.port !== null ? String(url.port) : form.port,
        ...(url.username ? { username: url.username } : {}),
        ...(url.password ? { password: { mode: "replace" as const, value: url.password } } : {}),
      },
      "address",
      "port",
      "username",
      "password",
    );
    return true;
  };

  const toInput = (): ProxyInput => ({
    id: proxy?.id ?? null,
    name: form.name.trim(),
    kind: form.kind,
    address: form.address.trim(),
    port: Number(form.port),
    username: form.username.trim(),
    // null keeps the saved password; a password typed over it replaces it.
    password: typedPassword ? typedPassword : hasSaved ? null : "",
  });

  const save = async () => {
    if (saving) return;
    const found = validateProxy({ ...form, password: typedPassword });
    const e: Errors = {};
    for (const [field, key] of Object.entries(found)) e[field as ProxyField] = t(`settings.proxy.err.${key}` as MessageKey);
    if (Object.keys(e).length) {
      setErrors(e);
      const first = (["name", "address", "port", "username", "password"] as const).find((k) => e[k]);
      if (first) document.getElementById(ids[first])?.focus();
      return;
    }
    setSaving(true);
    try {
      onSaved(await api.proxy_save(toInput()));
    } catch (err) {
      const ae = toAppError(err);
      const field = ae.code === "invalid_input" && ae.field ? BACKEND_FIELDS[ae.field] : undefined;
      setErrors(field ? { [field]: errorMessage(t, err) } : { general: errorMessage(t, err) });
      setSaving(false);
    }
  };

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
      <SheetHeader
        title={proxy ? t("settings.proxy.dlg.editTitle") : t("settings.proxy.dlg.addTitle")}
        subtitle={t("settings.proxy.dlg.subtitle")}
      />
      <form
        id={ids.form}
        className={s.form}
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <Group>
          <FormRow label={t("settings.proxy.f.name")} htmlFor={ids.name} error={errors.name}>
            <TextField
              id={ids.name}
              autoFocus
              value={form.name}
              invalid={!!errors.name}
              placeholder={t("settings.proxy.f.namePlaceholder")}
              onChange={(e) => patch({ name: e.target.value }, "name")}
            />
          </FormRow>
          <FormRow label={t("settings.proxy.f.kind")} top={form.kind === "http"}>
            <div className={s.stack}>
              <Segmented<ProxyKind>
                ariaLabel={t("settings.proxy.f.kind")}
                value={form.kind}
                options={[
                  { value: "socks5", label: KIND_LABEL.socks5 },
                  { value: "http", label: KIND_LABEL.http },
                ]}
                onChange={setKind}
              />
              {form.kind === "http" && <span className={s.hint}>{t("settings.proxy.f.kind.http")}</span>}
            </div>
          </FormRow>
          <FormRow
            label={t("settings.proxy.f.address")}
            htmlFor={ids.address}
            error={[errors.address, errors.port].filter(Boolean).join(" ") || null}
          >
            <div className={s.addrRow}>
              <TextField
                id={ids.address}
                mono
                value={form.address}
                invalid={!!errors.address}
                placeholder="127.0.0.1"
                onChange={(e) => patch({ address: e.target.value }, "address")}
                onPaste={(e) => {
                  if (takeUrl(e.clipboardData.getData("text"))) e.preventDefault();
                }}
                onBlur={() => takeUrl(form.address)}
              />
              <label className={s.portLabel} htmlFor={ids.port}>
                {t("settings.proxy.f.port")}
              </label>
              <TextField
                id={ids.port}
                mono
                inputMode="numeric"
                value={form.port}
                invalid={!!errors.port}
                onChange={(e) => patch({ port: e.target.value }, "port")}
              />
            </div>
          </FormRow>
        </Group>

        <Group>
          <FormRow label={t("settings.proxy.f.username")} htmlFor={ids.username} error={errors.username}>
            <TextField
              id={ids.username}
              mono
              autoComplete="off"
              value={form.username}
              invalid={!!errors.username}
              placeholder={t("settings.proxy.f.usernamePlaceholder")}
              onChange={(e) => patch({ username: e.target.value }, "username")}
            />
          </FormRow>
          {form.username.trim() !== "" && (
            <FormRow label={t("settings.proxy.f.password")} htmlFor={ids.password} error={errors.password}>
              <SavedKeyField
                id={ids.password}
                hasSaved={hasSaved}
                draft={form.password}
                allowClear={false}
                invalid={!!errors.password}
                placeholder={t("settings.proxy.f.passwordPlaceholder")}
                newPlaceholder={t("settings.proxy.f.passwordNew")}
                keepLabel={t("settings.proxy.f.keepPassword")}
                onChange={(password) => patch({ password }, "password")}
              />
            </FormRow>
          )}
        </Group>
        <div className={s.hint}>{t("settings.proxy.f.authHint")}</div>

        {errors.general && (
          <div className={s.problem} role="alert">
            {errors.general}
          </div>
        )}
      </form>
    </Sheet>
  );
}
