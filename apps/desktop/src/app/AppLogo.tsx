import { cx } from "@/lib/cx";
import s from "./AppLogo.module.css";

/** The app icon (`src-tauri/app-icon.svg`). `className` sets its size. */
export function AppLogo({ className }: { className?: string }) {
  return <div aria-hidden className={cx(s.appLogo, className)} />;
}
