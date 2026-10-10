import { describe, expect, it, vi } from "vitest";
import { messages } from "./locales";
import { detectLocale, formatBytes, formatDateTime, formatMessageTime, formatRelative, startOfDay, translate } from "./index";

const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

describe("message tables", () => {
  const zh = messages["zh-CN"];

  it.each(["en", "ja"] as const)("%s has every zh-CN key", (locale) => {
    const missing = Object.keys(zh).filter((k) => !(k in messages[locale]));
    expect(missing).toEqual([]);
  });

  it.each(["en", "ja"] as const)("%s uses the same placeholders as zh-CN", (locale) => {
    const table = messages[locale];
    const mismatched = Object.keys(zh).filter(
      (k) => k in table && placeholders(zh[k]).join() !== placeholders(table[k]).join(),
    );
    expect(mismatched).toEqual([]);
  });

  it("has no empty strings", () => {
    for (const locale of ["zh-CN", "en", "ja"] as const) {
      const empty = Object.entries(messages[locale]).filter(([, v]) => !v.trim()).map(([k]) => k);
      expect(empty).toEqual([]);
    }
  });
});

describe("translate", () => {
  it("interpolates and picks English singulars", () => {
    expect(translate("en", "sync.footer.offline_sub", { n: 3 })).toBe("3 changes pending");
    expect(translate("en", "sync.footer.offline_sub", { n: 1 })).toBe("1 change pending");
    expect(translate("zh-CN", "sync.footer.offline_sub", { n: 1 })).toBe("1 项更改待上传");
  });
});

describe("locale helpers", () => {
  it("detects the system language", () => {
    expect(detectLocale(["zh-TW"])).toBe("zh-CN");
    expect(detectLocale(["ja-JP"])).toBe("ja");
    expect(detectLocale(["fr-FR", "en-GB"])).toBe("en");
    expect(detectLocale(["fr-FR"])).toBe("en");
  });

  it("formats relative times like the design", () => {
    const now = new Date(2026, 9, 8, 9, 43).getTime();
    expect(formatRelative("zh-CN", now - 2 * 60_000, now)).toBe("2 分钟前");
    expect(formatRelative("en", now - 30_000, now)).toBe("Just now");
    expect(formatRelative("zh-CN", new Date(2026, 9, 8, 9, 12).getTime() - 60 * 60_000, now)).toBe("今天 08:12");
  });

  it("formats message times by how long ago the day was", () => {
    const now = new Date(2026, 9, 8, 9, 43).getTime();
    const at = (...d: [number, number, number, number, number]) => new Date(...d).getTime();
    expect(formatMessageTime("en", at(2026, 9, 8, 9, 5), now)).toBe("09:05");
    expect(formatMessageTime("zh-CN", at(2026, 9, 7, 23, 59), now)).toBe("昨天 23:59");
    expect(formatMessageTime("en", at(2026, 9, 5, 14, 3), now)).toBe("Oct 5, 14:03");
    expect(formatMessageTime("zh-CN", at(2025, 11, 31, 8, 0), now)).toBe("2025年12月31日 08:00");
    expect(formatMessageTime("en", at(2026, 9, 9, 1, 0), now)).toBe("Oct 9, 01:00");
  });

  it("counts days by the calendar across a DST change", () => {
    const tz = process.env.TZ;
    process.env.TZ = "America/New_York";
    try {
      // Clocks went forward on March 8, 2026, so that day had 23 hours.
      const now = new Date(2026, 2, 9, 0, 30).getTime();
      expect(startOfDay(now, -1)).toBe(new Date(2026, 2, 8).getTime());
      expect(now - startOfDay(now, -1)).toBe(23.5 * 3_600_000);
      // 24 hours before midnight would fall in March 7. The clock is in the new zone too, not the one
      // the cached formatters were made in.
      expect(formatMessageTime("en", new Date(2026, 2, 7, 23, 30).getTime(), now)).toBe("Mar 7, 23:30");
    } finally {
      if (tz === undefined) delete process.env.TZ;
      else process.env.TZ = tz;
    }
  });

  it("formats in the new time zone when the system's changes to one with the same offset", () => {
    const tz = process.env.TZ;
    vi.useFakeTimers({ now: Date.UTC(2026, 0, 15, 12) });
    try {
      const july = Date.UTC(2026, 6, 1, 18);
      process.env.TZ = "America/Denver";
      expect(formatDateTime("en", july)).toContain("12:00:00");
      // Phoenix is also UTC−7 in January, but keeps it in summer.
      process.env.TZ = "America/Phoenix";
      vi.advanceTimersByTime(1000);
      expect(formatDateTime("en", july)).toContain("11:00:00");
    } finally {
      vi.useRealTimers();
      if (tz === undefined) delete process.env.TZ;
      else process.env.TZ = tz;
    }
  });

  it("formats sizes", () => {
    expect(formatBytes(612)).toBe("612 B");
    expect(formatBytes(2150)).toBe("2.1 KB");
    expect(formatBytes(84 * 1024)).toBe("84 KB");
  });
});
