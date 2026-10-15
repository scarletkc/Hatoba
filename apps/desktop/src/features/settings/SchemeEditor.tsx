import { useEffect, useId, useState } from "react";
import { errorMessage } from "@/app/errors";
import { Button, Segmented, TextField } from "@/components/controls";
import { FormRow, Group } from "@/components/layout";
import { FooterSpacer, Sheet, SheetHeader, confirm } from "@/components/overlay";
import { PALETTE_KEYS, type PaletteKey, type TermMode } from "@/features/terminal/schemes";
import { useT, type MessageKey } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { SchemeColors, TerminalSchemeView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import s from "./SchemeEditor.module.css";

/** What the editor opens with: a copy of a scheme, an imported one, or a saved one (`id`). */
export interface SchemeDraft {
  id: string | null;
  name: string;
  mode: TermMode;
  colors: SchemeColors;
}

const BASIC: PaletteKey[] = ["foreground", "background", "cursor", "selection"];
const ANSI = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;

/** `#rrggbb` from what a user types: with or without `#`, three or six hex digits. */
export function parseHexInput(text: string): string | null {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(text.trim());
  if (!m) return null;
  const digits = m[1].length === 3 ? [...m[1]].map((c) => c + c).join("") : m[1];
  return `#${digits.toLowerCase()}`;
}

/**
 * Edit a colour scheme of the user's (TERM-06): name, the appearance it lists under, and its 20 colours, with a
 * preview that follows every change. Saving a new one adds it; a saved one can be deleted here.
 */
export function SchemeEditor({
  draft,
  onClose,
  onSaved,
  onDeleted,
}: {
  draft: SchemeDraft;
  onClose: () => void;
  onSaved: (scheme: TerminalSchemeView) => void;
  onDeleted: (id: string) => void;
}) {
  const t = useT();
  const ids = { form: useId(), name: useId() };
  const [form, setForm] = useState(draft);
  const [errors, setErrors] = useState<{ name?: string; general?: string }>({});
  const [busy, setBusy] = useState<"save" | "delete" | null>(null);

  const setColor = (key: PaletteKey, hex: string) => setForm((f) => ({ ...f, colors: { ...f.colors, [key]: hex } }));

  const save = async () => {
    if (busy) return;
    if (!form.name.trim()) {
      setErrors({ name: t("settings.scheme.err.name") });
      document.getElementById(ids.name)?.focus();
      return;
    }
    setBusy("save");
    try {
      onSaved(await api.terminal_scheme_save({ id: form.id, name: form.name.trim(), mode: form.mode, colors: form.colors }));
    } catch (e) {
      const field = toAppError(e).field;
      setErrors(field === "name" ? { name: errorMessage(t, e) } : { general: errorMessage(t, e) });
      setBusy(null);
    }
  };

  const remove = async () => {
    if (!form.id || busy) return;
    const ok = await confirm({
      title: t("settings.scheme.deleteTitle", { name: draft.name }),
      body: t("settings.scheme.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    setBusy("delete");
    try {
      await api.terminal_scheme_delete(form.id);
      onDeleted(form.id);
    } catch (e) {
      setErrors({ general: errorMessage(t, e) });
      setBusy(null);
    }
  };

  const colorLabel = (key: PaletteKey) => t(`settings.scheme.color.${key.replace("bright_", "")}` as MessageKey);

  return (
    <Sheet
      width={600}
      onClose={busy ? undefined : onClose}
      closeOnBackdrop={false}
      footer={
        <>
          {form.id && (
            <Button variant="danger" onClick={() => void remove()} busy={busy === "delete"} disabled={!!busy}>
              {t("btn.delete")}
            </Button>
          )}
          <FooterSpacer />
          <Button onClick={onClose} disabled={!!busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={ids.form} busy={busy === "save"} disabled={!!busy}>
            {t("btn.save")}
          </Button>
        </>
      }
    >
      <SheetHeader title={draft.id ? t("settings.scheme.editTitle") : t("settings.scheme.newTitle")} />
      <SchemePreview colors={form.colors} />
      <form
        id={ids.form}
        className={s.form}
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <Group>
          <FormRow label={t("settings.scheme.name")} htmlFor={ids.name} error={errors.name}>
            <TextField
              id={ids.name}
              value={form.name}
              maxLength={64}
              invalid={!!errors.name}
              onChange={(e) => {
                setForm((f) => ({ ...f, name: e.target.value }));
                setErrors({});
              }}
            />
          </FormRow>
          <FormRow label={t("settings.appearance")}>
            <Segmented<TermMode>
              ariaLabel={t("settings.appearance")}
              value={form.mode}
              options={[
                { value: "dark", label: t("settings.appearance.dark") },
                { value: "light", label: t("settings.appearance.light") },
              ]}
              onChange={(mode) => setForm((f) => ({ ...f, mode }))}
            />
          </FormRow>
        </Group>

        <div className={s.basic}>
          {BASIC.map((key) => (
            <label key={key} className={s.basicItem}>
              <span className={s.label}>{t(`settings.scheme.color.${key}` as MessageKey)}</span>
              <ColorField label={t(`settings.scheme.color.${key}` as MessageKey)} value={form.colors[key]} onChange={(hex) => setColor(key, hex)} />
            </label>
          ))}
        </div>

        <div className={s.ansi} role="group" aria-label={t("settings.scheme.ansi")}>
          <span />
          <span className={s.head}>{t("settings.scheme.normal")}</span>
          <span className={s.head}>{t("settings.scheme.bright")}</span>
          {ANSI.map((name) => (
            <AnsiRow key={name} name={name} label={colorLabel(name)} colors={form.colors} setColor={setColor} brightLabel={t("settings.scheme.bright")} />
          ))}
        </div>

        {errors.general && (
          <div className={s.problem} role="alert">
            {errors.general}
          </div>
        )}
      </form>
    </Sheet>
  );
}

function AnsiRow({
  name,
  label,
  brightLabel,
  colors,
  setColor,
}: {
  name: (typeof ANSI)[number];
  label: string;
  brightLabel: string;
  colors: SchemeColors;
  setColor: (key: PaletteKey, hex: string) => void;
}) {
  const bright = `bright_${name}` as const;
  return (
    <>
      <span className={s.label}>{label}</span>
      <ColorField label={label} value={colors[name]} onChange={(hex) => setColor(name, hex)} />
      <ColorField label={`${label} (${brightLabel})`} value={colors[bright]} onChange={(hex) => setColor(bright, hex)} />
    </>
  );
}

/** A colour as a swatch that opens the system colour picker, and its hex code to type or paste. */
function ColorField({ label, value, onChange }: { label: string; value: string; onChange: (hex: string) => void }) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const invalid = parseHexInput(text) === null;
  return (
    <TextField
      mono
      fieldClassName={s.colorField}
      aria-label={label}
      value={text}
      invalid={invalid}
      maxLength={7}
      leading={
        <span className={s.swatch} style={{ background: value }}>
          <input type="color" tabIndex={-1} aria-hidden value={value} onChange={(e) => onChange(e.target.value)} />
        </span>
      }
      onChange={(e) => {
        setText(e.target.value);
        const hex = parseHexInput(e.target.value);
        if (hex) onChange(hex);
      }}
      onBlur={() => setText(value)}
    />
  );
}

/** A terminal drawn in the scheme's colours: a prompt, a listing, a diff, a selection, the cursor, and the 16 colours. */
export function SchemePreview({ colors: c, className }: { colors: SchemeColors; className?: string }) {
  const ansi = PALETTE_KEYS.slice(4);
  return (
    <div className={cx(s.preview, className)} style={{ background: c.background, color: c.foreground }} aria-hidden>
      <div>
        <span style={{ color: c.green }}>❯</span> <span style={{ color: c.blue }}>~/hatoba</span> <span style={{ color: c.bright_black }}>on</span>{" "}
        <span style={{ color: c.magenta }}>main</span> <span style={{ color: c.yellow }}>!2</span>
      </div>
      <div>
        <span style={{ color: c.cyan }}>src</span> <span style={{ color: c.bright_black }}>.env</span> README.md <span style={{ color: c.green }}>deploy.sh</span>{" "}
        <span style={{ color: c.red }}>core.dump</span>
      </div>
      <div>
        <span style={{ color: c.red }}>- listen 80;</span> <span style={{ color: c.green }}>+ listen 443 ssl;</span>
      </div>
      <div>
        <span style={{ color: c.bright_red }}>error</span> <span style={{ color: c.bright_yellow }}>warning</span> <span style={{ color: c.bright_blue }}>info</span>{" "}
        <span style={{ background: c.selection }}>selected text</span> <span className={s.cursor} style={{ background: c.cursor }} />
      </div>
      <div className={s.swatches}>
        {ansi.map((key) => (
          <i key={key} style={{ background: c[key] }} />
        ))}
      </div>
    </div>
  );
}
