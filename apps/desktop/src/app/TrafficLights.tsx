import { useEffect, useState } from "react";
import { isTauri } from "@/ipc/api";
import type { Platform } from "@/lib/platform";
import s from "./TrafficLights.module.css";

/**
 * The macOS traffic lights, drawn for the browser mock (`pnpm dev`, the website's demo) with
 * `?platform=macos`, where no native window draws them. They sit where tao puts the native ones
 * in the window (platform/window.rs), so the title bar can be checked without a Mac. They do
 * nothing when clicked; the Tauri app never shows them.
 */
export function TrafficLights({ platform }: { platform: Platform }) {
  const [focused, setFocused] = useState(() => document.hasFocus());

  useEffect(() => {
    const onFocus = () => setFocused(true);
    const onBlur = () => setFocused(false);
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  if (platform !== "macos" || isTauri()) return null;
  return (
    <div className={s.lights} aria-hidden data-inactive={focused ? undefined : true}>
      <span className={s.close}>
        <svg viewBox="0 0 14 14">
          <path d="M4.5 4.5l5 5M9.5 4.5l-5 5" />
        </svg>
      </span>
      <span className={s.minimize}>
        <svg viewBox="0 0 14 14">
          <path d="M3.75 7h6.5" />
        </svg>
      </span>
      <span className={s.zoom}>
        <svg viewBox="0 0 14 14">
          <path className={s.fill} d="M4.25 4.25h4.25l-4.25 4.25zM9.75 9.75H5.5l4.25-4.25z" />
        </svg>
      </span>
    </div>
  );
}
