import { SearchAddon } from "@xterm/addon-search";
import type { Terminal } from "@xterm/xterm";
import { getInfo, patchInfo } from "./info";
import { searchColors } from "./schemes";
import { resolveScheme, useTermSettings } from "./theme";

/** Inline terminal search state and scheme-aware highlight redraw (TERM-07). */
export class SessionFinder {
  readonly addon = new SearchAddon();
  /** The find bar's query while it highlights matches, and the colours they were drawn with (JSON). */
  private query = "";
  private drawn = "";

  constructor(
    private readonly tabId: string,
    private readonly term: Terminal,
  ) {}

  private colors() {
    return searchColors(resolveScheme(useTermSettings.getState().settings));
  }

  find(query: string, direction: "next" | "prev", incremental = false): boolean {
    this.query = query;
    if (!query) {
      this.addon.clearDecorations();
      return false;
    }
    const decorations = this.colors();
    this.drawn = JSON.stringify(decorations);
    const options = { incremental, decorations };
    return direction === "next" ? this.addon.findNext(query, options) : this.addon.findPrevious(query, options);
  }

  /** The find bar closed: drop the highlights. */
  end() {
    this.query = "";
    this.addon.clearDecorations();
  }

  request() {
    patchInfo(this.tabId, { findTick: getInfo(this.tabId).findTick + 1 });
  }

  /**
   * The search addon keeps the colours it drew matches with, so a new scheme redraws them. Searching
   * again scrolls to a match; the view stays where the user left it.
   */
  redrawOnSchemeChange() {
    if (!this.query || JSON.stringify(this.colors()) === this.drawn) return;
    const top = this.term.buffer.active.viewportY;
    this.addon.clearDecorations();
    this.find(this.query, "next", true);
    this.term.scrollToLine(top);
  }
}
