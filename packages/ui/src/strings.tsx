import { createContext, useContext, type ReactNode } from "react";

/** The few words the components show themselves. An app passes them in its own language. */
export interface UiStrings {
  /** The reveal button of a secret field, while the text is hidden. */
  show: string;
  /** The same button while the text shows. */
  hide: string;
  /** The remove button of a tag. */
  removeTag: (name: string) => string;
  /** The cancel button of `confirm()`, unless the caller names its own. */
  cancel: string;
}

const ENGLISH: UiStrings = {
  show: "Show",
  hide: "Hide",
  removeTag: (name) => `Remove tag ${name}`,
  cancel: "Cancel",
};

const UiStringsContext = createContext<UiStrings>(ENGLISH);

/** Gives the components below it their words. Without it they speak English. */
export function UiStringsProvider({ strings, children }: { strings: UiStrings; children: ReactNode }) {
  return <UiStringsContext.Provider value={strings}>{children}</UiStringsContext.Provider>;
}

export const useUiStrings = () => useContext(UiStringsContext);
