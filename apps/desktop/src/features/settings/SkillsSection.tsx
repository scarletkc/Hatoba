import { useCallback, useEffect, useRef, useState } from "react";
import { Badge, Button, Icon, IconButton, Spinner, Switch } from "@/components/controls";
import { layoutStyles } from "@/components/layout";
import { Menu, confirm, toast, useMenu } from "@/components/overlay";
import { useT } from "@/i18n";
import { api, isTauri } from "@/ipc/api";
import type { BuiltinSkillView, SkillView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { pickSavePath } from "@/lib/native";
import { aiErrorMessage } from "./aiShared";
import { BuiltinSkillDialog } from "./BuiltinSkillDialog";
import { SkillDialog } from "./SkillDialog";
import { SkillImportDialog } from "./SkillImportDialog";
import { EmptyBlock, StatusRow } from "./sectionParts";
import { BUILTIN_SKILL_NAME, sortSkills, upsertSkill } from "./skillsLogic";
import a from "./AiPane.module.css";
import s from "./SkillsSection.module.css";

/** In a plain browser there is no folder picker, so the mock gets these paths (see `ipc/mock/aiExtensions.ts`). */
const BROWSER_FOLDER = "~/skills/docker-compose";
const BROWSER_ZIP = "~/Downloads/nginx-ops.zip";

async function pickSource(kind: "folder" | "zip", title: string): Promise<string | null> {
  if (!isTauri()) return kind === "folder" ? BROWSER_FOLDER : BROWSER_ZIP;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open(
    kind === "folder"
      ? { directory: true, multiple: false, title }
      : { directory: false, multiple: false, title, filters: [{ name: "Zip", extensions: ["zip"] }] },
  );
  return typeof picked === "string" ? picked : null;
}

/** The switch of the built-in skill, which is `Settings.ai.builtin_skill_enabled` (AI-34); the AI pane owns those settings. */
export interface BuiltinSwitch {
  enabled: boolean;
  onChange: (enabled: boolean) => void;
}

/**
 * Settings → AI → Skills (spec §13.8, AI-27): the built-in skill first (AI-34), with its switch and a read-only view,
 * then the user's skills with an enable switch, create and edit in a dialog, import a folder or `.zip` after a
 * preview, export as `.zip`, and delete. `builtin` is null until the AI settings are loaded.
 */
export function SkillsSection({ builtin }: { builtin: BuiltinSwitch | null }) {
  const t = useT();
  const [skills, setSkills] = useState<SkillView[] | null>(null);
  const [builtinSkill, setBuiltinSkill] = useState<BuiltinSkillView | null>(null);
  const [viewing, setViewing] = useState(false);
  const [loadError, setLoadError] = useState<unknown>(null);
  const [editing, setEditing] = useState<{ id: string | null } | null>(null);
  const [importPath, setImportPath] = useState<string | null>(null);
  const importMenu = useMenu();
  const importButton = useRef<HTMLButtonElement>(null);
  const live = useRef(true);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const [list, built] = await Promise.all([api.skills_list(), api.skill_builtin_get()]);
      if (!live.current) return;
      setBuiltinSkill(built);
      setSkills(sortSkills(list));
    } catch (e) {
      if (live.current) setLoadError(e);
    }
  }, []);

  useEffect(() => {
    live.current = true;
    void load();
    return () => {
      live.current = false;
    };
  }, [load]);

  const save = (skill: SkillView) => setSkills((cur) => upsertSkill(cur ?? [], skill));

  const toggle = async (skill: SkillView, enabled: boolean) => {
    save({ ...skill, enabled });
    try {
      await api.skill_set_enabled(skill.id, enabled);
    } catch (e) {
      save(skill);
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const remove = async (skill: SkillView) => {
    const ok = await confirm({
      title: t("aiSettings.skills.deleteTitle", { name: skill.name }),
      body: t("aiSettings.skills.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.skill_delete(skill.id);
      setSkills((cur) => (cur ?? []).filter((x) => x.id !== skill.id));
      toast(t("aiSettings.skills.deleted", { name: skill.name }));
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const exportSkill = async (skill: SkillView) => {
    try {
      const path = await pickSavePath(`${skill.name}.zip`, { name: "Zip", extension: "zip" });
      if (!path) return;
      await api.skill_export(skill.id, path);
      toast(t("aiSettings.skills.exported", { name: skill.name }), "success");
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const chooseSource = async (kind: "folder" | "zip") => {
    try {
      const path = await pickSource(kind, t(kind === "folder" ? "aiSettings.skills.import.folder" : "aiSettings.skills.import.zip"));
      if (path) setImportPath(path);
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const loaded = skills !== null;
  const list = skills ?? [];

  return (
    <section className={layoutStyles.section}>
      <div className={a.sectionHead}>
        <span className={layoutStyles.sectionTitle}>{t("aiSettings.skills")}</span>
        {loaded && (
          <div className={s.headButtons}>
            <Button
              ref={importButton}
              size="sm"
              icon="download-simple"
              aria-haspopup="menu"
              onClick={() => importButton.current && importMenu.openBelow(importButton.current, true)}
            >
              {t("aiSettings.skills.import")}
            </Button>
            {list.length > 0 && (
              <Button size="sm" icon="plus" onClick={() => setEditing({ id: null })}>
                {t("aiSettings.skills.add")}
              </Button>
            )}
          </div>
        )}
      </div>
      {importMenu.anchor && (
        <Menu
          anchor={importMenu.anchor}
          onClose={importMenu.close}
          entries={[
            { label: t("aiSettings.skills.import.folder"), icon: "folder-open", onSelect: () => void chooseSource("folder") },
            { label: t("aiSettings.skills.import.zip"), icon: "file-zip", onSelect: () => void chooseSource("zip") },
          ]}
        />
      )}

      <div className={cx(layoutStyles.group, a.clip)}>
        {!loaded && !loadError && <StatusRow icon={<Spinner />}>{t("aiSettings.loading")}</StatusRow>}
        {!loaded && !!loadError && (
          <StatusRow
            icon={<Icon name="warning-circle" color="var(--red)" />}
            action={
              <Button size="sm" onClick={() => void load()}>
                {t("btn.retry")}
              </Button>
            }
          >
            {t("aiSettings.skills.loadFailed")} {aiErrorMessage(t, loadError)}
          </StatusRow>
        )}
        {loaded && builtinSkill && (
          <BuiltinRow
            skill={builtinSkill}
            enabled={builtin?.enabled ?? builtinSkill.enabled}
            disabled={!builtin}
            onToggle={(on) => builtin?.onChange(on)}
            onView={() => setViewing(true)}
          />
        )}
        {loaded && list.length === 0 && (
          <EmptyBlock icon="book-open-text" title={t("aiSettings.skills.empty.title")} body={t("aiSettings.skills.empty.body")}>
            <Button variant="primary" icon="plus" onClick={() => setEditing({ id: null })}>
              {t("aiSettings.skills.add")}
            </Button>
          </EmptyBlock>
        )}
        {list.map((skill) => (
          <SkillRow
            key={skill.id}
            skill={skill}
            onEdit={() => setEditing({ id: skill.id })}
            onToggle={(enabled) => void toggle(skill, enabled)}
            onExport={() => void exportSkill(skill)}
            onDelete={() => void remove(skill)}
          />
        ))}
      </div>
      <div className={a.sectionHint}>{t("aiSettings.skills.hint")}</div>

      {editing && (
        <SkillDialog
          id={editing.id}
          takenNames={list.filter((x) => x.id !== editing.id).map((x) => x.name)}
          onClose={() => setEditing(null)}
          onSaved={save}
        />
      )}
      {viewing && builtinSkill && <BuiltinSkillDialog skill={builtinSkill} onClose={() => setViewing(false)} />}
      {importPath && (
        <SkillImportDialog
          path={importPath}
          takenNames={list.map((x) => x.name)}
          onClose={() => setImportPath(null)}
          onImported={(skill) => {
            save(skill);
            void load();
          }}
        />
      )}
    </section>
  );
}

/** AI-34: the built-in skill's row. It has a switch and a view, and no edit, export or delete. */
function BuiltinRow({
  skill,
  enabled,
  disabled,
  onToggle,
  onView,
}: {
  skill: BuiltinSkillView;
  enabled: boolean;
  disabled: boolean;
  onToggle: (enabled: boolean) => void;
  onView: () => void;
}) {
  const t = useT();
  const view = t("aiSettings.skills.builtin.view", { name: skill.name });
  return (
    <div className={cx(s.row, !enabled && s.off)}>
      <button type="button" className={s.main} title={view} onClick={onView}>
        <span className={s.icon} aria-hidden>
          <Icon name="book-open-text" />
        </span>
        <span className={s.text}>
          <span className={s.nameLine}>
            <span className={s.name}>{skill.name}</span>
            <Badge>{t("aiSettings.skills.builtin")}</Badge>
          </span>
          <span className={s.description}>{skill.description}</span>
          <span className={s.meta}>{t("aiSettings.skills.fileCount", { n: skill.files.length + 1 })}</span>
        </span>
      </button>
      <div className={s.actions}>
        <Switch checked={enabled} disabled={disabled} label={t("aiSettings.skills.builtin.enable", { name: skill.name })} onChange={onToggle} />
        <IconButton icon="eye" label={view} onClick={onView} />
      </div>
    </div>
  );
}

function SkillRow({
  skill,
  onEdit,
  onToggle,
  onExport,
  onDelete,
}: {
  skill: SkillView;
  onEdit: () => void;
  onToggle: (enabled: boolean) => void;
  onExport: () => void;
  onDelete: () => void;
}) {
  const t = useT();
  return (
    <div className={cx(s.row, !skill.enabled && s.off)}>
      <button type="button" className={s.main} title={t("aiSettings.skills.edit", { name: skill.name })} onClick={onEdit}>
        <span className={s.icon} aria-hidden>
          <Icon name="book-open-text" />
        </span>
        <span className={s.text}>
          <span className={s.name}>{skill.name}</span>
          <span className={s.description}>{skill.description}</span>
          <span className={s.meta}>
            {skill.files.length === 0 ? t("aiSettings.skills.noFiles") : t("aiSettings.skills.fileCount", { n: skill.files.length })}
          </span>
          {/* A skill from a build before the name was reserved: kept, but the name means the built-in skill (AI-34). */}
          {skill.name === BUILTIN_SKILL_NAME && <span className={s.reservedNote}>{t("aiSettings.skills.reservedNote")}</span>}
        </span>
      </button>
      <div className={s.actions}>
        <Switch checked={skill.enabled} label={t("aiSettings.skills.enable", { name: skill.name })} onChange={onToggle} />
        <IconButton icon="export" label={t("aiSettings.skills.export", { name: skill.name })} onClick={onExport} />
        <IconButton icon="trash" label={t("aiSettings.skills.delete", { name: skill.name })} onClick={onDelete} />
      </div>
    </div>
  );
}
