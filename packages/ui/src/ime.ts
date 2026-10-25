/** The parts of a React or DOM `KeyboardEvent` that tell an input method's key from a plain one. */
export interface KeyEventLike {
  keyCode: number;
  isComposing?: boolean;
  nativeEvent?: { isComposing: boolean };
}

/**
 * True when a keydown belongs to an input method (IME) rather than to the app, so an Enter handler
 * must not act on it. `isComposing` covers Chromium. In WKWebView (macOS) the keydown that commits a
 * candidate arrives after `compositionend`, so `isComposing` is already false and only `keyCode` 229
 * ("processed by an IME") gives it away.
 */
export function isImeEvent(e: KeyEventLike): boolean {
  return !!(e.isComposing ?? e.nativeEvent?.isComposing) || e.keyCode === 229;
}
