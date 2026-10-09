import ai from "./ai";
import aiSettings from "./aiSettings";
import common from "./common";
import hosts from "./hosts";
import keys from "./keys";
import settings from "./settings";
import sync from "./sync";
import terminal from "./terminal";
import vault from "./vault";

const tables = [common, hosts, terminal, keys, sync, vault, settings, ai, aiSettings];

export type Locale = "zh-CN" | "en" | "ja";

type Zh<T extends { "zh-CN": object }> = T["zh-CN"];
type AllZh = Zh<typeof common> &
  Zh<typeof hosts> &
  Zh<typeof terminal> &
  Zh<typeof keys> &
  Zh<typeof sync> &
  Zh<typeof vault> &
  Zh<typeof settings> &
  Zh<typeof ai> &
  Zh<typeof aiSettings>;

export type MessageKey = keyof AllZh & string;

function merge(locale: Locale): Record<string, string> {
  return Object.assign({}, ...tables.map((t) => t[locale]));
}

export const messages: Record<Locale, Record<string, string>> = {
  "zh-CN": merge("zh-CN"),
  en: merge("en"),
  ja: merge("ja"),
};
