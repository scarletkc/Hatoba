import { useEffect, useRef } from "react";
import { Button } from "@hatoba/ui";
import { useT } from "@/i18n";
import { useSessionInfo } from "@/session/info";
import type { LiveSession } from "@/session/session";
import { formatProgress } from "@/session/transfers";
import s from "./ZmodemBar.module.css";

export function ZmodemBar({ session }: { session: LiveSession }) {
  const t = useT();
  const view = useSessionInfo(session.tabId).zmodem;
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const element = input.current;
    const cancel = () => session.zmodem.cancel();
    element?.addEventListener("cancel", cancel);
    return () => element?.removeEventListener("cancel", cancel);
  }, [session, view?.phase]);
  if (!view) return null;
  return (
    <div className={s.bar} role="status">
      <span className={s.label}>
        {view.phase === "upload"
          ? t("terminal.zmodem.upload")
          : view.phase === "offer"
            ? t("terminal.zmodem.offer", { name: view.name })
            : `${view.name} · ${formatProgress(view.bytes, view.total)}`}
      </span>
      {view.phase === "upload" && (
        <>
          <input
            ref={input}
            type="file"
            multiple
            hidden
            onChange={(event) => {
              const files = Array.from(event.target.files ?? []);
              if (files.length) void session.zmodem.upload(files);
              else session.zmodem.cancel();
              event.target.value = "";
            }}
          />
          <Button onClick={() => input.current?.click()}>{t("terminal.zmodem.choose")}</Button>
        </>
      )}
      {view.phase === "offer" && (
        <>
          <Button onClick={() => void session.zmodem.receive()}>{t("terminal.zmodem.receive")}</Button>
          <Button onClick={() => session.zmodem.skip()}>{t("terminal.zmodem.skip")}</Button>
        </>
      )}
      <Button onClick={() => session.zmodem.cancel()}>{t("terminal.zmodem.cancel")}</Button>
    </div>
  );
}
