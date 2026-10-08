import { useCallback, useEffect, useRef, useState } from "react";
import { Button, Icon, IconButton, Spinner, Switch } from "@/components/controls";
import { layoutStyles } from "@/components/layout";
import { confirm, toast } from "@/components/overlay";
import { useT, type MessageKey } from "@/i18n";
import { api } from "@/ipc/api";
import type { McpServerState, McpServerStatus, McpServerView, McpToolView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { pickSavePath } from "@/lib/native";
import { aiErrorMessage } from "./aiShared";
import { McpImportDialog } from "./McpImportDialog";
import { McpServerDialog } from "./McpServerDialog";
import { annotationKeys, serverState, stderrText, toolAllowed, transportSummary, upsertServer, withAlwaysAllow } from "./mcpLogic";
import { EmptyBlock, StatusRow } from "./sectionParts";
import a from "./AiPane.module.css";
import s from "./McpSection.module.css";

/**
 * Settings → AI → MCP servers (spec §13.9, AI-29 to AI-33): the servers with a live state dot and the switch that turns
 * one on for this device, a status area per server (start, stop, the error and stderr of a failed one, the tools with
 * their Always allow switches), add and edit in a dialog, import from other clients' JSON, and export.
 */
export function McpSection() {
  const t = useT();
  const [servers, setServers] = useState<McpServerView[] | null>(null);
  const [status, setStatus] = useState<Record<string, McpServerStatus>>({});
  const [loadError, setLoadError] = useState<unknown>(null);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  /** Servers with a start or stop in flight, for the busy button. */
  const [working, setWorking] = useState<Record<string, "start" | "stop">>({});
  const [editing, setEditing] = useState<{ server: McpServerView | null } | null>(null);
  const [importing, setImporting] = useState(false);
  const live = useRef(true);

  const setOneStatus = useCallback((next: McpServerStatus) => {
    if (live.current) setStatus((cur) => ({ ...cur, [next.server_id]: next }));
  }, []);

  const refreshStatus = useCallback(
    async (id: string) => {
      try {
        setOneStatus(await api.mcp_server_status(id));
      } catch {
        // The server may be gone already; the list is the authority on that.
      }
    },
    [setOneStatus],
  );

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const list = await api.mcp_servers_list();
      const states = await Promise.all(list.map((x) => api.mcp_server_status(x.id).catch(() => null)));
      if (!live.current) return;
      setServers(list);
      setStatus((cur) => {
        const next = { ...cur };
        for (const st of states) if (st) next[st.server_id] = st;
        return next;
      });
    } catch (e) {
      if (live.current) setLoadError(e);
    }
  }, []);

  useEffect(() => {
    live.current = true;
    void load();
    // The state of a server changes when a conversation starts it, as well as when this pane does.
    const unlisten = api.listen("ai://mcp-status", setOneStatus);
    return () => {
      live.current = false;
      void unlisten.then((off) => off());
    };
  }, [load, setOneStatus]);

  const patchServer = (server: McpServerView) => setServers((cur) => upsertServer(cur ?? [], server));

  const toggleExpanded = (id: string) =>
    setExpanded((cur) => {
      const next = new Set(cur);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const setEnabled = async (server: McpServerView, enabled: boolean) => {
    patchServer({ ...server, enabled });
    try {
      await api.mcp_server_set_enabled(server.id, enabled);
      await refreshStatus(server.id);
    } catch (e) {
      patchServer(server);
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const run = async (server: McpServerView, action: "start" | "stop") => {
    setWorking((cur) => ({ ...cur, [server.id]: action }));
    try {
      if (action === "start") setOneStatus(await api.mcp_server_start(server.id));
      else {
        await api.mcp_server_stop(server.id);
        await refreshStatus(server.id);
      }
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
      await refreshStatus(server.id);
    } finally {
      if (live.current) {
        setWorking((cur) => {
          const { [server.id]: _done, ...rest } = cur;
          return rest;
        });
      }
    }
  };

  const setAlwaysAllow = async (server: McpServerView, tool: string | null, allow: boolean) => {
    patchServer(withAlwaysAllow(server, tool, allow));
    try {
      await api.mcp_set_always_allow(server.id, tool, allow);
    } catch (e) {
      patchServer(server);
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const remove = async (server: McpServerView) => {
    const ok = await confirm({
      title: t("aiSettings.mcp.deleteTitle", { name: server.name }),
      body: t("aiSettings.mcp.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await api.mcp_server_delete(server.id);
      setServers((cur) => (cur ?? []).filter((x) => x.id !== server.id));
      toast(t("aiSettings.mcp.deleted", { name: server.name }));
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const exportServers = async () => {
    try {
      const path = await pickSavePath("mcp-servers.json", { name: "JSON", extension: "json" });
      if (!path) return;
      await api.save_text_file(path, await api.mcp_export());
      toast(t("aiSettings.mcp.exported"), "success", 4200);
    } catch (e) {
      toast(aiErrorMessage(t, e), "error");
    }
  };

  const loaded = servers !== null;
  const list = servers ?? [];

  return (
    <section className={layoutStyles.section}>
      <div className={a.sectionHead}>
        <span className={layoutStyles.sectionTitle}>{t("aiSettings.mcp")}</span>
        {loaded && (
          <div className={s.headButtons}>
            <Button size="sm" icon="download-simple" onClick={() => setImporting(true)}>
              {t("aiSettings.mcp.import")}
            </Button>
            {list.length > 0 && (
              <Button size="sm" icon="export" onClick={() => void exportServers()}>
                {t("aiSettings.mcp.export")}
              </Button>
            )}
            {list.length > 0 && (
              <Button size="sm" icon="plus" onClick={() => setEditing({ server: null })}>
                {t("aiSettings.mcp.add")}
              </Button>
            )}
          </div>
        )}
      </div>

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
            {t("aiSettings.mcp.loadFailed")} {aiErrorMessage(t, loadError)}
          </StatusRow>
        )}
        {loaded && list.length === 0 && (
          <EmptyBlock icon="plugs-connected" title={t("aiSettings.mcp.empty.title")} body={t("aiSettings.mcp.empty.body")}>
            <Button variant="primary" icon="plus" onClick={() => setEditing({ server: null })}>
              {t("aiSettings.mcp.add")}
            </Button>
          </EmptyBlock>
        )}
        {list.map((server) => (
          <ServerRow
            key={server.id}
            server={server}
            status={status[server.id]}
            open={expanded.has(server.id)}
            working={working[server.id] ?? null}
            onToggleOpen={() => toggleExpanded(server.id)}
            onEnabled={(enabled) => void setEnabled(server, enabled)}
            onEdit={() => setEditing({ server })}
            onDelete={() => void remove(server)}
            onRun={(action) => void run(server, action)}
            onAlwaysAllow={(tool, allow) => void setAlwaysAllow(server, tool, allow)}
          />
        ))}
      </div>
      <div className={a.sectionHint}>{t("aiSettings.mcp.hint")}</div>

      {editing && (
        <McpServerDialog
          server={editing.server}
          otherNames={list.filter((x) => x.id !== editing.server?.id).map((x) => x.name)}
          onClose={() => setEditing(null)}
          onSaved={(saved) => {
            patchServer(saved);
            void refreshStatus(saved.id);
          }}
        />
      )}
      {importing && <McpImportDialog onClose={() => setImporting(false)} onImported={() => void load()} />}
    </section>
  );
}

const STATE_COLOR: Record<McpServerState, string> = {
  stopped: "var(--fg3)",
  starting: "var(--orange)",
  running: "var(--green)",
  failed: "var(--red)",
};

function ServerRow({
  server,
  status,
  open,
  working,
  onToggleOpen,
  onEnabled,
  onEdit,
  onDelete,
  onRun,
  onAlwaysAllow,
}: {
  server: McpServerView;
  status: McpServerStatus | undefined;
  open: boolean;
  working: "start" | "stop" | null;
  onToggleOpen: () => void;
  onEnabled: (enabled: boolean) => void;
  onEdit: () => void;
  onDelete: () => void;
  onRun: (action: "start" | "stop") => void;
  onAlwaysAllow: (tool: string | null, allow: boolean) => void;
}) {
  const t = useT();
  const state = serverState(status);
  const off = !server.enabled && state === "stopped";
  const tools = status?.tools ?? [];
  const label = off
    ? t("aiSettings.mcp.state.off")
    : state === "running"
      ? t("aiSettings.mcp.state.runningN", { n: tools.length })
      : t(`aiSettings.mcp.state.${state}` as MessageKey);

  return (
    <div className={cx(s.server, open && s.open)}>
      <div className={s.row}>
        <button type="button" className={s.main} aria-expanded={open} aria-label={t("aiSettings.mcp.row.details", { name: server.name })} onClick={onToggleOpen}>
          <Icon name="caret-right" className={cx(s.caret, open && s.caretOpen)} />
          <span className={s.text}>
            <span className={s.nameLine}>
              <span className={s.dot} data-state={off ? "stopped" : state} style={{ background: STATE_COLOR[off ? "stopped" : state] }} />
              <span className={s.name}>{server.name}</span>
              <span className={s.badge}>{t(`aiSettings.mcp.kind.${server.transport.kind}` as MessageKey)}</span>
              {server.always_ask && <span className={cx(s.badge, s.badgeAsk)}>{t("aiSettings.mcp.alwaysAsk")}</span>}
            </span>
            <span className={cx(s.summary, "selectable")} title={transportSummary(server.transport)}>
              {transportSummary(server.transport)}
            </span>
            <span className={cx(s.state, state === "failed" && s.stateFailed)}>{label}</span>
          </span>
        </button>
        <div className={s.actions}>
          <span title={t("aiSettings.mcp.row.enableTip")}>
            <Switch checked={server.enabled} label={t("aiSettings.mcp.row.enable", { name: server.name })} onChange={onEnabled} />
          </span>
          <IconButton icon="pencil-simple" label={t("aiSettings.mcp.row.edit", { name: server.name })} onClick={onEdit} />
          <IconButton icon="trash" label={t("aiSettings.mcp.row.delete", { name: server.name })} onClick={onDelete} />
        </div>
      </div>
      {open && (
        <ServerDetails
          server={server}
          status={status}
          state={state}
          off={off}
          working={working}
          onRun={onRun}
          onAlwaysAllow={onAlwaysAllow}
        />
      )}
    </div>
  );
}

/** The status area of one server (AI-32) and its Always allow switches (AI-31). */
function ServerDetails({
  server,
  status,
  state,
  off,
  working,
  onRun,
  onAlwaysAllow,
}: {
  server: McpServerView;
  status: McpServerStatus | undefined;
  state: McpServerState;
  off: boolean;
  working: "start" | "stop" | null;
  onRun: (action: "start" | "stop") => void;
  onAlwaysAllow: (tool: string | null, allow: boolean) => void;
}) {
  const t = useT();
  const tools = status?.tools ?? [];
  const stderr = stderrText(status?.stderr ?? []);
  const starting = state === "starting" || working === "start";
  const sentence = off
    ? t("aiSettings.mcp.status.off")
    : state === "running"
      ? t("aiSettings.mcp.status.running", { n: tools.length })
      : t(`aiSettings.mcp.status.${state}` as MessageKey);

  return (
    <div className={s.details}>
      <div className={s.statusBar}>
        <span className={s.statusText}>{sentence}</span>
        {server.enabled && (
          <div className={s.statusButtons}>
            <Button
              size="sm"
              icon={state === "running" || state === "failed" ? "arrow-clockwise" : "play"}
              busy={starting}
              onClick={() => onRun("start")}
            >
              {state === "running" || state === "failed" ? t("aiSettings.mcp.restart") : t("aiSettings.mcp.start")}
            </Button>
            <Button size="sm" icon="stop" busy={working === "stop"} disabled={state === "stopped" || state === "failed"} onClick={() => onRun("stop")}>
              {t("aiSettings.mcp.stop")}
            </Button>
          </div>
        )}
      </div>

      {state === "failed" && (status?.error || stderr) && (
        <div className={s.failure} role="alert">
          {status?.error && (
            <div>
              <div className={s.failureLabel}>{t("aiSettings.mcp.error")}</div>
              <div className={cx(s.failureText, "selectable")}>{status.error}</div>
            </div>
          )}
          {stderr && (
            <div>
              <div className={s.failureLabel}>{t("aiSettings.mcp.stderr")}</div>
              <pre className={cx(s.stderr, "selectable")} tabIndex={0}>
                {stderr}
              </pre>
            </div>
          )}
        </div>
      )}

      <div className={s.allowAll}>
        <div className={s.allowAllText}>
          <span className={s.allowAllLabel}>{t("aiSettings.mcp.allowAll")}</span>
          <span className={s.hint}>{t("aiSettings.mcp.allowAll.hint")}</span>
        </div>
        <Switch checked={server.always_allow} label={t("aiSettings.mcp.allowAll")} onChange={(allow) => onAlwaysAllow(null, allow)} />
      </div>

      <div className={s.toolsHead}>
        <span className={s.toolsTitle}>{t("aiSettings.mcp.tools")}</span>
        {tools.length > 0 && <span className={s.hint}>{tools.length}</span>}
      </div>
      {tools.length === 0 ? (
        state !== "starting" && <div className={s.noTools}>{t("aiSettings.mcp.tools.none")}</div>
      ) : (
        <>
          <div className={s.hint}>{t("aiSettings.mcp.tools.hint")}</div>
          <div className={s.tools}>
            {tools.map((tool) => (
              <ToolItem
                key={tool.name}
                tool={tool}
                allowed={toolAllowed(server, tool)}
                locked={server.always_allow}
                onChange={(allow) => onAlwaysAllow(tool.tool, allow)}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function ToolItem({ tool, allowed, locked, onChange }: { tool: McpToolView; allowed: boolean; locked: boolean; onChange: (allow: boolean) => void }) {
  const t = useT();
  const keys = annotationKeys(tool.annotations);
  return (
    <div className={s.tool}>
      <div className={s.toolHead}>
        <div className={s.toolTitle}>
          <span className={cx(s.toolName, "selectable")} title={tool.name}>
            {tool.tool}
          </span>
          {tool.annotations.title && tool.annotations.title !== tool.tool && <span className={s.toolDisplay}>{tool.annotations.title}</span>}
          {keys.map((key) => (
            <span key={key} className={cx(s.chip, key === "destructive" && s.chipDanger)}>
              {t(`aiSettings.mcp.ann.${key}` as MessageKey)}
            </span>
          ))}
        </div>
        <label className={cx(s.allow, locked && s.allowLocked)} title={locked ? t("aiSettings.mcp.allowAll") : undefined}>
          <span>{t("aiSettings.mcp.allowTool")}</span>
          <Switch checked={allowed} disabled={locked} label={t("aiSettings.mcp.allowToolLabel", { tool: tool.tool })} onChange={onChange} />
        </label>
      </div>
      <div className={cx(s.toolDescription, "selectable", !tool.description && s.toolNone)}>
        {tool.description || t("aiSettings.mcp.noDescription")}
      </div>
    </div>
  );
}
