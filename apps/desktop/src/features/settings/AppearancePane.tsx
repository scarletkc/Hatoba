import { useMemo, useRef, useState } from "react";
import { useApp } from "@/app/store";
import { Icon, Segmented, controlStyles } from "@/components/controls";
import { Group } from "@/components/layout";
import { Menu, toast, useMenu, type MenuEntry } from "@/components/overlay";
import { MAX_IMPORT_BYTES, parseSchemeFile, SchemeImportError, type ImportedScheme } from "@/features/terminal/schemeImport";
import { customScheme, paletteOf, SCHEMES, type TermMode, type TermScheme } from "@/features/terminal/schemes";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { Language, LocalPrefs, TerminalSchemeView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { useFreshReads } from "./freshReads";
import { SchemeEditor, type SchemeDraft } from "./SchemeEditor";
import { SchemePicker } from "./SchemePicker";
import { Pane, SettingRow, usePrefs, type PaneProps } from "./shared";
import s from "./AppearancePane.module.css";

type Appearance = LocalPrefs["appearance"];

/** `name`, or `name 2`, `name 3`… when a scheme already has it. */
export function freeName(name: string, taken: readonly string[]): string {
  const used = new Set(taken.map((n) => n.toLowerCase()));
  if (!used.has(name.toLowerCase())) return name;
  for (let n = 2; ; n++) if (!used.has(`${name} ${n}`.toLowerCase())) return `${name} ${n}`;
}

/** Settings → Appearance (design §07): theme thumbnails, density, terminal color schemes, language. */
export function AppearancePane({ settings, updateTerminal }: PaneProps) {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();
  const langMenu = useMenu();
  const langButton = useRef<HTMLButtonElement>(null);
  const term = settings?.terminal;
  /** Following the app appearance: a scheme for each mode. */
  const both = term?.theme === "system";

  // TERM-06: the user's schemes, read again after each sync round like the settings.
  const [views, setViews] = useState<TerminalSchemeView[]>([]);
  const schemeReads = useFreshReads(api.terminal_schemes_list, setViews, () => {});
  const custom = useMemo(() => views.map(customScheme), [views]);
  const [editing, setEditing] = useState<SchemeDraft | null>(null);
  const importMenu = useMenu();
  const [imported, setImported] = useState<{ mode: TermMode; schemes: ImportedScheme[] } | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const importFor = useRef<{ mode: TermMode; button: HTMLElement } | null>(null);
  // WKWebView greys out files whose extension maps to no type it knows, such as .itermcolors.
  const macos = useApp((a) => a.info.platform) === "macos";

  const takenNames = () => [...SCHEMES, ...custom].map((x) => x.name);
  const duplicate = (scheme: TermScheme) =>
    setEditing({
      id: null,
      name: freeName(t("settings.scheme.copyName", { name: scheme.name }), takenNames()),
      mode: scheme.mode,
      colors: paletteOf(scheme),
    });
  const edit = (scheme: TermScheme) => setEditing({ id: scheme.id, name: scheme.name, mode: scheme.mode, colors: paletteOf(scheme) });
  // An imported scheme lists where the user imported it, whatever its background.
  const fromImport = (mode: TermMode, scheme: ImportedScheme) =>
    setEditing({ id: null, name: freeName(scheme.name, takenNames()), mode, colors: scheme.colors });

  const readFile = async (file: File) => {
    const target = importFor.current;
    if (!target) return;
    try {
      if (file.size > MAX_IMPORT_BYTES) throw new SchemeImportError("tooLarge");
      const schemes = parseSchemeFile(await file.text(), file.name);
      if (schemes.length === 1) return fromImport(target.mode, schemes[0]);
      setImported({ mode: target.mode, schemes });
      importMenu.openBelow(target.button, true);
    } catch (e) {
      const reason = e instanceof SchemeImportError ? e.reason : "unreadable";
      toast(t(`settings.scheme.importFailed.${reason}`, { name: file.name }), "error", 5000);
    }
  };

  const saved = (view: TerminalSchemeView) => {
    schemeReads.edited();
    setViews((cur) => [...cur.filter((v) => v.id !== view.id), view].sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase())));
    setEditing(null);
    // A saved scheme is put to use, under the appearance it lists under.
    updateTerminal(view.mode === "dark" ? { dark_scheme: view.id } : { light_scheme: view.id });
  };
  const deleted = (id: string) => {
    schemeReads.edited();
    setViews((cur) => cur.filter((v) => v.id !== id));
    setEditing(null);
  };

  const picker = (mode: TermMode) => ({
    mode,
    custom,
    label: t("settings.termColors"),
    caption: both ? t(mode === "dark" ? "settings.termColors.forDark" : "settings.termColors.forLight") : undefined,
    onDuplicate: duplicate,
    onEdit: edit,
    onImport: (button: HTMLElement) => {
      importFor.current = { mode, button };
      fileInput.current?.click();
    },
  });

  const languages: { value: Language; label: string }[] = [
    { value: "system", label: t("settings.language.system") },
    { value: "zh-CN", label: t("settings.language.zh") },
    { value: "en", label: t("settings.language.en") },
    { value: "ja", label: t("settings.language.ja") },
  ];
  const langEntries: MenuEntry[] = languages.flatMap((l, i): MenuEntry[] => [
    { label: l.label, checked: prefs.language === l.value, onSelect: () => setPrefs({ language: l.value }) },
    ...(i === 0 ? [{ kind: "separator" } as const] : []),
  ]);

  return (
    <Pane>
      <Group>
        <SettingRow top label={t("settings.appearance")}>
          <div className={s.thumbs} role="radiogroup" aria-label={t("settings.appearance")}>
            <Thumb kind="light" label={t("settings.appearance.light")} selected={prefs.appearance === "light"} onSelect={() => setPrefs({ appearance: "light" })} />
            <Thumb kind="dark" label={t("settings.appearance.dark")} selected={prefs.appearance === "dark"} onSelect={() => setPrefs({ appearance: "dark" })} />
            <Thumb kind="system" label={t("settings.appearance.auto")} selected={prefs.appearance === "system"} onSelect={() => setPrefs({ appearance: "system" })} />
          </div>
        </SettingRow>
        <SettingRow label={t("settings.density")}>
          <Segmented
            ariaLabel={t("settings.density")}
            value={prefs.density}
            options={[
              { value: "regular", label: t("settings.density.regular") },
              { value: "compact", label: t("settings.density.compact") },
            ]}
            onChange={(density) => setPrefs({ density })}
          />
        </SettingRow>
      </Group>

      {term && (
        <Group>
          <SettingRow label={t("settings.termColors")}>
            <Segmented
              ariaLabel={t("settings.termColors")}
              value={term.theme}
              options={[
                { value: "dark", label: t("settings.termColors.dark") },
                { value: "light", label: t("settings.termColors.light") },
                { value: "system", label: t("settings.termColors.follow") },
              ]}
              onChange={(theme) => updateTerminal({ theme })}
            />
          </SettingRow>
          {term.theme !== "light" && (
            <SchemePicker {...picker("dark")} value={term.dark_scheme} onChange={(dark_scheme) => updateTerminal({ dark_scheme })} />
          )}
          {term.theme !== "dark" && (
            <SchemePicker {...picker("light")} value={term.light_scheme} onChange={(light_scheme) => updateTerminal({ light_scheme })} />
          )}
          <input
            ref={fileInput}
            type="file"
            hidden
            accept={macos ? undefined : ".json,.itermcolors"}
            onChange={(e) => {
              const file = e.target.files?.[0];
              e.target.value = "";
              if (file) void readFile(file);
            }}
          />
        </Group>
      )}

      <Group>
        <SettingRow label={t("settings.language")}>
          <button
            ref={langButton}
            type="button"
            aria-haspopup="menu"
            aria-expanded={!!langMenu.anchor}
            aria-label={t("settings.language")}
            className={cx(controlStyles.popup, s.langButton)}
            onClick={() => langButton.current && langMenu.openBelow(langButton.current, true)}
          >
            <Icon name="translate" className={controlStyles.popupIcon} style={{ fontSize: 14 }} />
            <span className={controlStyles.popupLabel}>{languages.find((l) => l.value === prefs.language)?.label}</span>
            <Icon name="caret-up-down" className={controlStyles.popupCaret} />
          </button>
        </SettingRow>
      </Group>

      {langMenu.anchor && <Menu anchor={langMenu.anchor} onClose={langMenu.close} minWidth={200} entries={langEntries} />}
      {importMenu.anchor && imported && (
        <Menu
          anchor={importMenu.anchor}
          onClose={importMenu.close}
          minWidth={220}
          entries={[
            { kind: "header", label: t("settings.scheme.importPick") },
            ...imported.schemes.map((scheme) => ({ label: scheme.name, onSelect: () => fromImport(imported.mode, scheme) })),
          ]}
        />
      )}
      {editing && <SchemeEditor draft={editing} onClose={() => setEditing(null)} onSaved={saved} onDeleted={deleted} />}
    </Pane>
  );
}

/** Miniature window used as the appearance picker (colors are fixed illustrations, not theme tokens). */
function Thumb({ kind, label, selected, onSelect }: { kind: Appearance; label: string; selected: boolean; onSelect: () => void }) {
  return (
    <button type="button" role="radio" aria-checked={selected} className={s.thumb} onClick={onSelect}>
      <span className={cx(s.preview, s[kind], selected && s.previewOn)}>
        {kind === "system" ? (
          <>
            <span className={cx(s.half, s.light)}>
              <span className={s.side} />
              <span className={s.lines}>
                <i style={{ width: "90%" }} />
                <i />
              </span>
            </span>
            <span className={cx(s.half, s.dark)}>
              <span className={s.lines}>
                <i style={{ width: "60%" }} />
                <i style={{ width: "80%" }} />
              </span>
            </span>
          </>
        ) : (
          <>
            <span className={s.side} />
            <span className={s.lines}>
              <i style={{ width: "70%" }} />
              <i style={{ width: "90%" }} />
              <i style={{ width: "60%" }} />
            </span>
          </>
        )}
      </span>
      <span className={cx(s.thumbLabel, selected && s.thumbLabelOn)}>{label}</span>
    </button>
  );
}
