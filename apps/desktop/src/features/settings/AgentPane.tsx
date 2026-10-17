import { useEffect, useRef, useState } from "react";
import { errorMessage } from "@/app/errors";
import { Button, Icon, TextField } from "@/components/controls";
import { Group, layoutStyles } from "@/components/layout";
import { PopupSelect, toast, type SelectOption } from "@/components/overlay";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { SshAgentKind, SshAgentView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { useFreshReads } from "./freshReads";
import { Pane, SettingRow, usePrefs } from "./shared";
import s from "./AgentPane.module.css";

/** The select's value for "type the path yourself"; never a socket path. */
const CUSTOM = "\u0000custom";

/** The name of the agent behind a socket, as far as the socket path says (SSH-09). */
function agentLabel(t: ReturnType<typeof useT>, kind: SshAgentKind): string {
  switch (kind) {
    case "bitwarden":
      return t("settings.agent.kind.bitwarden");
    case "one_password":
      return t("settings.agent.kind.onePassword");
    case "gnu_pgp":
      return t("settings.agent.kind.gpg");
    case "keyring":
      return t("settings.agent.kind.keyring");
    case "system":
      return t("settings.agent.kind.other");
  }
}

/**
 * Settings → SSH Agent (SSH-09): the agent this device authenticates with, and what it holds.
 *
 * On macOS and Linux a window is not started from a login shell, so it never reads the
 * `~/.zshrc` that exports `SSH_AUTH_SOCK`. Hatoba therefore finds the agents installed here by
 * their own sockets, which is what makes Bitwarden work without exporting anything.
 */
export function AgentPane() {
  const t = useT();
  const [prefs, setPrefs] = usePrefs();
  const saved = prefs.ssh_agent_socket ?? "";
  const [view, setView] = useState<SshAgentView | null>(null);
  /** The user chose **Custom path…** and has not named one yet: the field stays, the choice is automatic. */
  const [drafting, setDrafting] = useState(false);
  const [draft, setDraft] = useState(saved);

  // Reads when the tab opens and after each sync round, as every other settings pane does.
  const reads = useFreshReads(
    api.ssh_agent_status,
    (next) => setView(next),
    (e) => {
      toast(errorMessage(t, e), "error");
      setView(null);
    },
  );

  // Checked again after every change of the choice, so the tab shows that choice's agent.
  const lastSaved = useRef(saved);
  useEffect(() => {
    if (lastSaved.current === saved) return;
    lastSaved.current = saved;
    reads.refresh();
  }, [reads, saved]);

  useEffect(() => setDraft(saved), [saved]);

  // **Automatic** may find the agent of this device, and a saved path of the user's own shows the
  // field; there is nothing else to choose between, so the choice is automatic or a path.
  const selection = saved !== "" || drafting ? CUSTOM : "";

  const options: SelectOption<string>[] = [
    { value: "", label: t("settings.agent.automatic") },
    { value: CUSTOM, label: t("settings.agent.custom") },
  ];

  const choose = (value: string) => {
    const custom = value === CUSTOM;
    setDrafting(custom);
    if (!custom) setPrefs({ ssh_agent_socket: null });
    // A **Custom path…** left alone changes nothing: an empty path means automatic.
  };

  const savePath = () => {
    const path = draft.trim();
    setDrafting(false);
    if (path === saved) return;
    setPrefs({ ssh_agent_socket: path === "" ? null : path });
  };

  const used = view?.socket ? view.kind : null;

  return (
    <Pane>
      <Group>
        <SettingRow label={t("settings.agent.source")}>
          <PopupSelect<string>
            ariaLabel={t("settings.agent.source")}
            icon="key"
            value={selection}
            options={options}
            minWidth={180}
            onChange={choose}
          />
        </SettingRow>
        {selection === CUSTOM && (
          <SettingRow label={t("settings.agent.path")} top>
            <TextField
              mono
              fieldClassName={s.pathInput}
              value={draft}
              placeholder={t("settings.agent.path.placeholder")}
              aria-label={t("settings.agent.path")}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={savePath}
              onKeyDown={(e) => {
                if (e.key === "Enter") savePath();
              }}
            />
          </SettingRow>
        )}
      </Group>

      <section className={layoutStyles.section}>
        <div className={s.head}>
          <span className={layoutStyles.sectionTitle}>{t("settings.agent.status")}</span>
          <Button size="sm" icon="arrows-clockwise" onClick={() => reads.refresh()}>
            {t("settings.agent.recheck")}
          </Button>
        </div>
        <div className={cx(layoutStyles.group, s.clip)}>
          {view === null && <p className={s.muted}>{t("settings.agent.checking")}</p>}
          {view?.error != null && (
            <div className={s.problem}>
              <Icon name="warning-circle" className={s.warnIcon} />
              <div className={s.problemText}>
                <span>{view.error}</span>
                <span className={s.hint}>{t("settings.agent.hint")}</span>
              </div>
            </div>
          )}
          {view && view.error === null && (
            <>
              <div className={s.agent}>
                <span className={s.icon} aria-hidden>
                  <Icon name="key" />
                </span>
                <div className={s.agentText}>
                  <span className={s.agentName}>
                    {used ? agentLabel(t, used) : t("settings.agent.kind.other")}
                    {view.keys.length > 0 && (
                      <span className={s.count}>{t("settings.agent.keys", { n: view.keys.length })}</span>
                    )}
                  </span>
                  <span className={s.socket} title={view.socket ?? ""}>
                    {view.socket}
                  </span>
                </div>
              </div>
              {view.keys.length === 0 ? (
                <p className={s.muted}>{t("settings.agent.keys.none")}</p>
              ) : (
                view.keys.map((k, i) => (
                  <div className={s.key} key={`${k.fingerprint}-${i}`}>
                    <span className={s.algorithm}>{k.algorithm}</span>
                    <span className={s.fingerprint}>{k.fingerprint}</span>
                    {k.comment && <span className={s.comment}>{k.comment}</span>}
                  </div>
                ))
              )}
            </>
          )}
        </div>
      </section>
    </Pane>
  );
}
