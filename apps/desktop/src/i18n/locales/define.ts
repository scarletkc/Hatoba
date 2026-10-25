/** A feature's message table: zh-CN is the source of truth; the other languages must provide every key. */
export type Table<T extends Record<string, string>> = {
  "zh-CN": T;
  en: { [K in keyof T]: string } & { [K in `${string & keyof T}_one`]?: string };
  ja: { [K in keyof T]: string };
  "zh-TW": { [K in keyof T]: string };
};

export function defineMessages<T extends Record<string, string>>(table: Table<T>): Table<T> {
  return table;
}
