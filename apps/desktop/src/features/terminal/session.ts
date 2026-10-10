import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import { hostById, useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { isAppShortcut } from "@/app/shortcuts";
import { useApp } from "@/app/store";
import { useTabs, type TabSource, type TabStatus } from "@/app/tabs";
import { toast } from "@/components/overlay";
import { useRecentTargets } from "@/features/hosts/recent";
import { useTransfers } from "@/features/sftp/transfers";
import { t } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import {
  FRAME_CLOSED,
  FRAME_DATA,
  FRAME_ERROR,
  FRAME_SESSION,
  type AppError,
  type HostView,
  type QuickTarget,
  type SessionStateEvent,
} from "@/ipc/types";
import { isMac } from "@/lib/platform";
import { openExternal, readClipboard, writeClipboard } from "./clipboard";
import { dropSessionForwards, syncActiveForwards } from "./forwards";
import { clearInfo, getInfo, patchInfo } from "./info";
import { confirmMultilinePaste } from "./paste";
import { askSecret, cancelSessionPrompts } from "./prompts";
import { resolveMode, SEARCH_DECORATIONS, terminalOptions, useTermSettings } from "./theme";

/** One-off credentials for a connection attempt (SSH-03). They are passed to `ssh_connect` and dropped. */
export interface Credentials {
  password: string | null;
  passphrase: string | null;
}

const decoder = new TextDecoder();

const byTab = new Map<string, LiveSession>();
const bySession = new Map<string, LiveSession>();
/** State events that arrived before `ssh_connect` told us the session id. */
const earlyEvents = new Map<string, SessionStateEvent[]>();

export function getSession(tabId: string): LiveSession | undefined {
  return byTab.get(tabId);
}

/**
 * The live session behind a tab, created on first use. Returns null when the tab is gone, so a
 * late re-render of a closed tab can never resurrect (and reconnect) a session.
 */
export function ensureSession(tabId: string): LiveSession | null {
  const existing = byTab.get(tabId);
  if (existing) return existing;
  const tab = useTabs.getState().tabs.find((x) => x.id === tabId);
  if (!tab) return null;
  const source: TabSource = tab.target !== null ? { hostId: null, target: tab.target } : { hostId: tab.hostId, target: null };
  const session = new LiveSession(tabId, source);
  byTab.set(tabId, session);
  return session;
}

/** Route an `ssh://state` event to the tab that owns the session. */
export function routeStateEvent(ev: SessionStateEvent) {
  const session = bySession.get(ev.session_id);
  if (session) {
    session.onState(ev);
    return;
  }
  const list = earlyEvents.get(ev.session_id) ?? [];
  list.push(ev);
  earlyEvents.set(ev.session_id, list.slice(-8));
  if (earlyEvents.size > 32) earlyEvents.delete(earlyEvents.keys().next().value!);
}

useTermSettings.subscribe(() => byTab.forEach((s) => s.applySettings()));

function needsPassphrase(err: AppError): boolean {
  return err.code === "key_parse" && (err.key_kind === "passphrase_required" || err.key_kind === "wrong_passphrase");
}

/**
 * Everything about one terminal tab that must outlive React renders: the xterm instance, the
 * backend session and its frame handler. React only attaches/detaches the DOM (so StrictMode's
 * double mount and tab switches never reconnect).
 */
export class LiveSession {
  readonly term: Terminal;
  readonly search = new SearchAddon();
  private readonly fit = new FitAddon();
  private webgl: WebglAddon | null = null;

  sessionId: string | null = null;
  status: TabStatus = "connecting";

  private lastSessionId: string | null = null;
  /** The backend session id of the connection attempt in progress (FRAME_SESSION). */
  private pendingSessionId: string | null = null;
  private creds: Credentials | null = null;
  private started = false;
  private opened = false;
  private disposed = false;
  private active = false;
  private gen = 0;
  private attempts = 0;
  private sent = { cols: 0, rows: 0 };
  private container: HTMLElement | null = null;
  private observer: ResizeObserver | null = null;
  private fitTimer: number | undefined;
  private readonly outputListeners = new Set<(text: string) => void>();
  private readonly outputDecoder = new TextDecoder();

  constructor(
    readonly tabId: string,
    private source: TabSource,
  ) {
    this.term = new Terminal({
      ...terminalOptions(useTermSettings.getState().settings),
      allowProposedApi: true, // search decorations
      scrollOnUserInput: true,
      macOptionIsMeta: true,
    });
    this.term.loadAddon(this.fit);
    this.term.loadAddon(this.search);
    this.term.loadAddon(
      new WebLinksAddon((event, uri) => {
        // TERM-08: links open on Ctrl+click only, so a stray click while selecting never navigates.
        if (event.ctrlKey || event.metaKey) void openExternal(uri);
      }),
    );
    this.term.attachCustomKeyEventHandler(this.handleKey);
    this.term.onData((data) => this.write(data));
    this.term.onResize(() => this.sendResize());
  }

  /** The saved host; null for a quick connection. */
  get hostId(): string | null {
    return this.source.hostId;
  }

  /** What a quick connection connects to (HOST-12); null for a saved host. */
  get target(): QuickTarget | null {
    return this.source.target;
  }

  /** A quick connection whose target is now a saved host connects as that host from here on. */
  adoptHost(host: HostView) {
    this.source = { hostId: host.id, target: null };
    useTabs.getState().adoptHost(this.tabId, host.id, host.name);
  }

  // ───────────── DOM attachment (called by the React view) ─────────────

  attach(container: HTMLElement) {
    if (this.disposed) return;
    this.container = container;
    if (!this.opened) {
      this.term.open(container);
      this.opened = true;
      // Capture phase: we own pasting so multi-line content can be confirmed first (TERM-04).
      this.term.element?.addEventListener("paste", this.onPasteEvent, true);
      void document.fonts?.ready.then(() => this.fitNow());
    } else if (this.term.element && this.term.element.parentElement !== container) {
      container.appendChild(this.term.element);
    }
    this.observer?.disconnect();
    this.observer = new ResizeObserver(() => this.scheduleFit());
    this.observer.observe(container);
    this.fitNow();
    if (this.active) this.loadWebgl();
    if (!this.started) void this.start();
  }

  detach(container: HTMLElement) {
    if (this.container !== container) return;
    this.observer?.disconnect();
    this.observer = null;
    this.container = null;
    window.clearTimeout(this.fitTimer);
  }

  /** The tab became visible / hidden. Only the visible terminal pays for a WebGL context. */
  setActive(active: boolean) {
    this.active = active;
    if (this.disposed || !this.opened) return;
    if (active) {
      this.loadWebgl();
      this.fitNow();
      this.term.focus();
    } else {
      this.unloadWebgl();
    }
  }

  focus() {
    this.term.focus();
  }

  applySettings() {
    if (this.disposed) return;
    Object.assign(this.term.options, terminalOptions(useTermSettings.getState().settings));
    requestAnimationFrame(() => this.fitNow());
  }

  private scheduleFit() {
    window.clearTimeout(this.fitTimer);
    this.fitTimer = window.setTimeout(() => this.fitNow(), 50);
  }

  /** TERM-03: refit to the container; `onResize` then tells the PTY, only when cols/rows changed. */
  fitNow() {
    if (this.disposed || !this.opened || !this.container?.clientWidth) return;
    const dims = this.fit.proposeDimensions();
    if (!dims || !Number.isFinite(dims.cols) || !Number.isFinite(dims.rows)) return;
    if (dims.cols !== this.term.cols || dims.rows !== this.term.rows) this.fit.fit();
  }

  private loadWebgl() {
    if (this.webgl || !this.opened) return;
    const gl = new WebglAddon();
    try {
      gl.onContextLoss(() => {
        gl.dispose();
        if (this.webgl === gl) this.webgl = null; // falls back to the DOM renderer
      });
      this.term.loadAddon(gl);
      this.webgl = gl;
    } catch {
      gl.dispose();
    }
  }

  private unloadWebgl() {
    this.webgl?.dispose();
    this.webgl = null;
  }

  // ───────────── connection lifecycle ─────────────

  setCredentials(creds: Credentials | null) {
    this.creds = creds;
  }

  private get isConnecting(): boolean {
    return this.status === "connecting";
  }

  private setStatus(status: TabStatus) {
    this.status = status;
    useTabs.getState().update(this.tabId, { status, sessionId: this.sessionId });
  }

  private async start() {
    this.started = true;
    const creds = this.creds;
    this.creds = null;
    await this.connect(creds);
  }

  private async connect(creds: Credentials | null) {
    const gen = ++this.gen;
    this.attempts++;
    patchInfo(this.tabId, { error: null, errorAt: null, reason: null, latencyMs: null, attempts: this.attempts });
    this.sessionId = null;
    this.pendingSessionId = null;
    this.term.write("\x1b[?25h"); // show the cursor again (hidden while failed / disconnected)
    this.setStatus("connecting");

    let passphrase = creds?.passphrase ?? null;
    const password = creds?.password ?? null;
    for (let round = 0; ; round++) {
      const sentSize = { cols: this.term.cols, rows: this.term.rows };
      const options = { cols: sentSize.cols, rows: sentSize.rows, password, passphrase };
      const onFrame = (frame: Uint8Array) => this.onFrame(frame, gen);
      const source = this.source;
      try {
        // A quick connection asks for its password while authenticating (an `ssh://auth-prompt`).
        const sid =
          source.target !== null
            ? await api.ssh_connect_target(source.target, options, onFrame)
            : await api.ssh_connect(source.hostId, options, onFrame);
        if (gen !== this.gen || this.disposed) {
          void api.ssh_disconnect(sid).catch(() => {});
          return;
        }
        this.onConnected(sid, sentSize);
        return;
      } catch (e) {
        if (gen !== this.gen || this.disposed) return;
        const err = toAppError(e);
        if (needsPassphrase(err) && round < 3) {
          // The backend has no (or a wrong) saved passphrase: ask once, use it, forget it.
          const host = hostById(this.hostId);
          const answer = await askSecret({
            kind: "passphrase",
            hostName: host?.name ?? "",
            target: host ? `${host.username}@${host.address}:${host.port}` : "",
            wrong: err.key_kind === "wrong_passphrase",
          });
          if (gen !== this.gen || this.disposed) return;
          if (answer !== null) {
            passphrase = answer;
            continue;
          }
        }
        this.fail(err);
        return;
      }
    }
  }

  private onConnected(sid: string, sentSize: { cols: number; rows: number }) {
    this.releaseSession();
    this.sessionId = sid;
    this.lastSessionId = sid;
    this.sent = sentSize;
    bySession.set(sid, this);
    this.setStatus("connected");
    // A saved host's last connected time and server OS; a quick connection's recent targets.
    if (this.target) void useRecentTargets.getState().load();
    else void useVaultData.getState().reloadHosts().catch(() => {});
    void syncActiveForwards(sid); // auto-start forwards (FWD-02) may be running already
    this.sendResize(); // the view may have been resized while connecting
    const early = earlyEvents.get(sid);
    earlyEvents.delete(sid);
    early?.forEach((ev) => this.onState(ev));
    if (this.active && this.status === "connected") this.term.focus();
  }

  private fail(err: AppError) {
    patchInfo(this.tabId, { error: err, errorAt: Date.now() });
    const atLineStart = this.term.buffer.active.cursorX === 0;
    this.term.write(`${atLineStart ? "" : "\r\n"}ssh: ${err.detail}\r\n\x1b[?25l`);
    this.setStatus("failed");
  }

  /** Connection ended (remote closed, network drop, or the user disconnected). */
  private markDisconnected(reason: string | null, weak = false) {
    if (this.status === "disconnected") {
      // A later state event may know better than the bare "closed" frame.
      if (reason && !weak) patchInfo(this.tabId, { reason });
      return;
    }
    if (this.status !== "connected") return;
    const sid = this.sessionId;
    this.sessionId = null;
    if (sid) {
      useTransfers.getState().purge(sid);
      dropSessionForwards(sid);
    }
    patchInfo(this.tabId, { reason });
    this.term.write("\x1b[?25l");
    this.setStatus("disconnected");
  }

  onState(ev: SessionStateEvent) {
    if (this.disposed) return;
    if (ev.latency_ms != null) patchInfo(this.tabId, { latencyMs: ev.latency_ms });
    if (ev.state === "disconnected" || ev.state === "failed") this.markDisconnected(this.reasonFrom(ev));
  }

  private reasonFrom(ev: SessionStateEvent): string | null {
    const clean = (s: string) => s.replace(/[。.]\s*$/, "");
    if (ev.error) return ev.error.ssh_kind === "disconnected" ? null : clean(errorMessage(t, ev.error));
    if (ev.exit_status === 0) return t("terminal.sessionEnded");
    if (ev.exit_status != null) return t("terminal.exitStatus", { code: ev.exit_status });
    return null;
  }

  private onFrame(frame: Uint8Array, gen: number) {
    if (gen !== this.gen || this.disposed || frame.length === 0) return;
    switch (frame[0]) {
      case FRAME_DATA:
        if (frame.length > 1) {
          const data = frame.subarray(1);
          if (this.outputListeners.size > 0) {
            const text = this.outputDecoder.decode(data, { stream: true });
            this.outputListeners.forEach((listener) => listener(text));
          }
          this.term.write(data);
        }
        break;
      case FRAME_CLOSED:
        this.markDisconnected(decoder.decode(frame.subarray(1)).trim() || null, true);
        break;
      case FRAME_ERROR:
        this.term.write(`\r\n\x1b[31m${decoder.decode(frame.subarray(1))}\x1b[0m\r\n`);
        break;
      case FRAME_SESSION:
        this.pendingSessionId = decoder.decode(frame.subarray(1));
        break;
    }
  }

  /** Forget the previous backend session id (after a disconnect, before reconnecting or disposing). */
  private releaseSession() {
    if (this.lastSessionId) {
      bySession.delete(this.lastSessionId);
      earlyEvents.delete(this.lastSessionId);
      this.lastSessionId = null;
    }
  }

  /** Reconnect in this tab (SSH-07: always user-initiated, never an automatic loop). */
  async reconnect(collect: () => Promise<Credentials | null>) {
    if (this.disposed || this.isConnecting) return;
    if (!this.target && !hostById(this.hostId)) {
      toast(t("terminal.hostMissing"), "error");
      return;
    }
    const creds = await collect();
    if (!creds || this.disposed || this.isConnecting) return;
    const old = this.sessionId;
    this.gen++; // drop frames and results from the old connection
    if (old) {
      this.sessionId = null;
      useTransfers.getState().purge(old);
      dropSessionForwards(old);
      void api.ssh_disconnect(old).catch(() => {});
    }
    this.releaseSession();
    const atLineStart = this.term.buffer.active.cursorX === 0;
    this.term.write(`${atLineStart ? "" : "\r\n"}\x1b[90m── ${t("terminal.reconnecting")} ──\x1b[0m\r\n`);
    await this.connect(creds);
  }

  /** User-requested disconnect; the tab stays open showing the "reconnect" banner. */
  async disconnect() {
    const sid = this.sessionId;
    if (!sid) return;
    this.gen++;
    this.markDisconnected(null);
    await api.ssh_disconnect(sid).catch(() => {});
  }

  /** Close everything: backend session, xterm, listeners. */
  async dispose() {
    if (this.disposed) return;
    this.disposed = true;
    // Host-key and login prompts of an attempt still in progress would outlive the tab.
    if (this.isConnecting && this.pendingSessionId) cancelSessionPrompts(this.pendingSessionId);
    this.gen++;
    const sid = this.sessionId;
    this.sessionId = null;
    const closing = sid ? api.ssh_disconnect(sid).catch(() => {}) : null;
    if (sid) {
      useTransfers.getState().purge(sid);
      dropSessionForwards(sid);
    }
    this.releaseSession();
    window.clearTimeout(this.fitTimer);
    this.observer?.disconnect();
    this.term.element?.removeEventListener("paste", this.onPasteEvent, true);
    this.unloadWebgl();
    this.term.dispose();
    byTab.delete(this.tabId);
    clearInfo(this.tabId);
    await closing;
  }

  // ───────────── input, resize, clipboard ─────────────

  private write(data: string) {
    const sid = this.sessionId;
    if (sid && this.status === "connected") void api.ssh_write(sid, data).catch(() => {});
  }

  /** Types `data` into the shell through the same path as the keyboard (AI-13). Rejects when not connected. */
  async sendInput(data: string): Promise<void> {
    const sid = this.sessionId;
    if (!sid || this.status !== "connected") throw toAppError(new Error("the session is not connected"));
    this.term.scrollToBottom();
    await api.ssh_write(sid, data);
  }

  /** Calls `listener` with the output as it arrives, decoded to text (AI-13). Returns the unsubscribe function. */
  onOutput(listener: (text: string) => void): () => void {
    this.outputListeners.add(listener);
    return () => {
      this.outputListeners.delete(listener);
    };
  }

  private sendResize() {
    const sid = this.sessionId;
    if (!sid || this.status !== "connected") return;
    const { cols, rows } = this.term;
    if (cols === this.sent.cols && rows === this.sent.rows) return;
    this.sent = { cols, rows };
    void api.ssh_resize(sid, cols, rows).catch(() => {});
  }

  async copySelection(clearAfter = false): Promise<boolean> {
    const text = this.term.getSelection();
    if (!text) return false;
    try {
      await writeClipboard(text);
    } catch {
      toast(t("terminal.clipboardDenied"), "error");
      return false;
    }
    if (clearAfter) this.term.clearSelection();
    return true;
  }

  async pasteFromClipboard() {
    try {
      await this.paste(await readClipboard());
    } catch {
      toast(t("terminal.clipboardDenied"), "error");
    }
  }

  /** Paste text as the user's input (bracketed when the remote asked for it), confirming multi-line pastes. */
  async paste(text: string) {
    if (!text || this.status !== "connected") return;
    if (/[\r\n]/.test(text) && useTermSettings.getState().settings.confirm_multiline_paste) {
      const ok = await confirmMultilinePaste(text);
      if (!ok) {
        this.term.focus();
        return;
      }
    }
    // Reading the clipboard or confirming can outlast the session, or the vault being unlocked.
    if (this.status !== "connected" || useApp.getState().phase !== "unlocked") return;
    this.term.paste(text);
    this.term.focus();
  }

  private onPasteEvent = (e: ClipboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    void this.paste(e.clipboardData?.getData("text/plain") ?? "");
  };

  /** Right click should go to the remote app, not to us, while it tracks the mouse (vim, tmux…). */
  get remoteTracksMouse(): boolean {
    return this.term.modes.mouseTrackingMode !== "none";
  }

  // ───────────── search ─────────────

  find(query: string, direction: "next" | "prev", incremental = false): boolean {
    if (!query) {
      this.search.clearDecorations();
      return false;
    }
    const options = {
      incremental,
      decorations: SEARCH_DECORATIONS[resolveMode(useTermSettings.getState().settings.theme)],
    };
    return direction === "next" ? this.search.findNext(query, options) : this.search.findPrevious(query, options);
  }

  requestFind() {
    patchInfo(this.tabId, { findTick: getInfo(this.tabId).findTick + 1 });
  }

  // ───────────── keyboard (WIN-04, WIN-05) ─────────────

  /**
   * Decides whether xterm handles a key. Returning false leaves the key to the browser / app.
   * Plain Ctrl+letter always reaches the remote; app shortcuts (Ctrl+Shift+…) never do.
   */
  private handleKey = (e: KeyboardEvent): boolean => {
    const platform = useApp.getState().info.platform;
    if (isAppShortcut(e, platform)) return false;
    const down = e.type === "keydown";
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;

    if (isMac(platform)) {
      if (!e.metaKey || e.ctrlKey || e.altKey) return true;
      if (key === "c") {
        if (down) void this.copySelection();
        return false;
      }
      if (key === "v") return false; // the native paste event does the work
      if (key === "f") {
        if (down) this.requestFind();
        return false;
      }
      if (key === "k" && !e.shiftKey) {
        // matchShortcut leaves ⌘K to the terminal while it has focus; elsewhere, and with Shift, it
        // searches hosts.
        if (down) this.term.clear();
        return false;
      }
      return true;
    }

    if (e.ctrlKey && !e.altKey && !e.metaKey) {
      if (key === "c" && !e.shiftKey) {
        // With a selection Ctrl+C copies; otherwise it is ^C for the remote (WIN-05).
        if (!this.term.hasSelection()) return true;
        if (down) void this.copySelection(true);
        return false;
      }
      if (key === "c" && e.shiftKey) {
        e.preventDefault(); // Ctrl+Shift+C opens the inspector in some webviews
        if (down) void this.copySelection();
        return false;
      }
      if (key === "v") return false; // Ctrl+V / Ctrl+Shift+V: the native paste event does the work
      if (key === "f" && e.shiftKey) {
        e.preventDefault();
        if (down) this.requestFind();
        return false;
      }
      if (key === "Insert" && !e.shiftKey) {
        if (down) void this.copySelection();
        return false;
      }
    }
    if (key === "Insert" && e.shiftKey && !e.ctrlKey && !e.altKey) return false; // Shift+Insert pastes
    return true;
  };
}
