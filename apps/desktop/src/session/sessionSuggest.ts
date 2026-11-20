import type { Terminal } from "@xterm/xterm";
import { api, toAppError } from "@/ipc/api";
import {
  addToHistory,
  dropServerLists,
  dropTabHistory,
  historyOf,
  listedHistory,
  onHistoryChange,
  removeFromHistory,
  serverListsOf,
  setServerLists,
  type HistoryOwner,
} from "./history";
import { getInfo, patchInfo } from "./info";
import { PathLister } from "./pathSuggest";
import { InputTracker, commandNameFor, nextWord, suggestionFor } from "./suggest";
import { useTermSettings } from "./theme";

/** TERM-14: the dimmed rest of a command after the cursor, at the cursor's cell of the view. */
export interface Suggestion {
  rest: string;
  x: number;
  y: number;
}

/** A screen cell in window coordinates. */
export interface CellBox {
  left: number;
  top: number;
  width: number;
  height: number;
  /** The right edge of the terminal's screen. */
  screenRight: number;
}

export interface SuggestHost {
  readonly tabId: string;
  readonly term: Terminal;
  history(): HistoryOwner;
  isConnected(): boolean;
  isOpened(): boolean;
  isDisposed(): boolean;
  isSsh(): boolean;
  sessionId(): string | null;
  write(data: string): void;
}

/** Command suggestions, history tracking, and server-list fetching for one terminal tab (TERM-14..16). */
export class SessionSuggestions {
  /** TERM-14, TERM-15: where the input starts on the current prompt. */
  private readonly tracker = new InputTracker();
  /** The suggestion showing after the cursor; null while none does. */
  current: Suggestion | null = null;
  private readonly listeners = new Set<() => void>();
  private frame = 0;
  private composing = false;
  /** Input went to the shell and its echo has not come back: the screen does not show it yet. */
  private awaitingEcho = false;
  private readonly stopHistory: () => void;
  /** TERM-16: false once the server refused the exec channel. */
  private asksServer = true;
  /** TERM-16: the server names a system that is not asked (HOST-11), through SFTP neither. */
  private otherSystem = false;
  private readonly paths = new PathLister(() => this.schedule());

  constructor(private readonly host: SuggestHost) {
    this.stopHistory = onHistoryChange(() => this.schedule());
  }

  /** Calls `listener` when the suggestion changes. Returns the unsubscribe function. */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** The commands the list shows (TERM-14, TERM-16), the same list until it changes. */
  commandHistory = (): readonly string[] => listedHistory(this.host.tabId, this.host.history());

  onComposition = (e: CompositionEvent) => {
    this.composing = e.type === "compositionstart";
    this.schedule();
  };

  resetPrompt() {
    this.tracker.reset();
  }

  onConnected(sid: string) {
    historyOf(this.host.history()); // TERM-15: read before the first suggestion is needed
    this.readServerLists(sid);
  }

  onWriteParsed() {
    this.awaitingEcho = false;
    this.schedule();
  }

  /** Records submitted commands and suppresses stale ghost text until the shell echoes. */
  onInput(data: string) {
    const { command } = this.tracker.onInput(data, this.host.term.buffer.active);
    if (command !== null) addToHistory(this.host.history(), command);
  }

  afterInputSent() {
    // Until the shell echoes the input, a suggestion read from the screen would be stale: another → would
    // type the same rest again.
    this.awaitingEcho = true;
    if (this.current) {
      this.current = null;
      this.listeners.forEach((listener) => listener());
    }
    this.schedule();
  }

  schedule() {
    if (this.host.isDisposed() || this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      const next = this.compute();
      const now = this.current;
      if (next?.rest === now?.rest && next?.x === now?.x && next?.y === now?.y) return;
      this.current = next;
      this.listeners.forEach((listener) => listener());
    });
  }

  /** TERM-16: what an SSH tab's server offers suggestions, read once per connection. */
  private readServerLists(sid: string) {
    // The previous connection's: the host may since name another server.
    dropServerLists(this.host.tabId);
    this.paths.connected(null);
    if (!this.host.isSsh() || this.otherSystem) return;
    if (!this.asksServer) {
      this.paths.connected(sid);
      return;
    }
    // The answer tells whether the server is one to list paths on as well.
    api.ssh_suggestions(sid).then(
      (lists) => {
        if (this.host.isDisposed() || sid !== this.host.sessionId()) return;
        if (lists) {
          setServerLists(this.host.tabId, lists);
          this.paths.connected(sid);
        } else {
          this.otherSystem = true;
        }
      },
      (e: unknown) => {
        if (this.host.isDisposed() || sid !== this.host.sessionId()) return;
        // A refusal stays for the tab; a timeout or a lost connection says nothing about the next connection.
        if (toAppError(e).ssh_kind === "channel") this.asksServer = false;
        this.paths.connected(sid);
        this.schedule(); // a path typed while waiting
      },
    );
  }

  /**
   * What continues what is typed at a prompt, while one can show: the newest command of the tab's history,
   * or else of its server's history, or else a command name or a path on the server (TERM-16).
   */
  private compute(): Suggestion | null {
    if (
      this.host.isDisposed() ||
      !this.host.isOpened() ||
      !this.host.isConnected() ||
      this.composing ||
      this.awaitingEcho
    ) {
      return null;
    }
    if (!useTermSettings.getState().settings.suggest_commands) return null;
    const buf = this.host.term.buffer.active;
    if (buf.type === "alternate" || buf.viewportY !== buf.baseY) return null;
    const typed = this.tracker.typedAtEnd(buf);
    if (!typed) return null;
    const server = serverListsOf(this.host.tabId);
    const rest =
      suggestionFor(typed, historyOf(this.host.history())) ??
      suggestionFor(typed, server.history) ??
      commandNameFor(typed, server.commands) ??
      this.paths.rest(typed);
    return rest === null ? null : { rest, x: buf.cursorX, y: buf.cursorY };
  }

  /** The screen cell at column `x`, row `y` of the view, in window coordinates; null before the terminal opens. */
  cellBox(x: number, y: number): CellBox | null {
    const screen = this.host.term.element?.querySelector<HTMLElement>(".xterm-screen");
    if (!screen || !this.host.term.cols || !this.host.term.rows) return null;
    const r = screen.getBoundingClientRect();
    const width = r.width / this.host.term.cols;
    const height = r.height / this.host.term.rows;
    return { left: r.left + x * width, top: r.top + y * height, width, height, screenRight: r.right };
  }

  /** What the list of commands opens with (TERM-14): what is typed and the cursor's cell. */
  listContext(): { typed: string; cursor: CellBox | null } {
    const buf = this.host.term.buffer.active;
    return { typed: this.tracker.typedAtEnd(buf) ?? "", cursor: this.cellBox(buf.cursorX, buf.cursorY) };
  }

  /** Types `command` from the list in place of what is typed, without running it. */
  acceptListed(command: string) {
    const right = this.host.term.modes.applicationCursorKeysMode ? "\x1bOC" : "\x1b[C";
    this.host.write(this.tracker.clearKeys(this.host.term.buffer.active, right) + command);
    this.host.term.focus();
  }

  /** Shift+Delete in the list. */
  removeListed(command: string) {
    removeFromHistory(this.host.tabId, this.host.history(), command);
  }

  /**
   * →, End and Ctrl+→ (⌥→ on macOS) take the suggestion while one shows, and Ctrl+Shift+Space
   * (⌘⇧Space) opens the list. True when the key was taken.
   */
  handleKey(e: KeyboardEvent, mac: boolean): boolean {
    if (e.isComposing) return false;
    const down = e.type === "keydown";
    const listKey =
      e.code === "Space" && e.shiftKey && !e.altKey && (mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey);
    if (listKey) {
      e.preventDefault();
      if (down && this.host.isConnected()) {
        patchInfo(this.host.tabId, { suggestTick: getInfo(this.host.tabId).suggestTick + 1 });
      }
      return true;
    }
    const shown = this.current;
    if (!shown || (e.key !== "ArrowRight" && e.key !== "End")) return false;
    const plain = !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey;
    const word =
      e.key === "ArrowRight" && !e.shiftKey && !e.metaKey && (mac ? e.altKey && !e.ctrlKey : e.ctrlKey && !e.altKey);
    if (!plain && !word) return false;
    e.preventDefault();
    if (down) this.host.write(word ? nextWord(shown.rest) : shown.rest);
    return true;
  }

  dispose() {
    cancelAnimationFrame(this.frame);
    this.stopHistory();
    if (!this.host.history().saved) dropTabHistory(this.host.tabId);
    dropServerLists(this.host.tabId);
    this.paths.connected(null);
  }
}
