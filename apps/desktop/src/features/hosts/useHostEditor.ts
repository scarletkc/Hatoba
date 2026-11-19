import { useEffect, useRef } from "react";
import { confirm, toast } from "@hatoba/ui";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { useT } from "@/i18n";
import { toAppError } from "@/ipc/api";

type Tr = ReturnType<typeof useT>;

/** Validates a host's address and TCP port as both SSH and Telnet editors require. */
export function validateAddressAndPort(t: Tr, address: string, port: string): { address?: string; port?: string } {
  const e: { address?: string; port?: string } = {};
  if (!address.trim()) e.address = t("hosts.err.addressRequired");
  const n = Number(port);
  if (!/^\d+$/.test(port.trim()) || n < 1 || n > 65535) e.port = t("hosts.err.portInvalid");
  return e;
}

/** Maps a failed host save to a field error, the oversized-host toast, or a generic error toast. */
export function handleHostSaveError<Field extends string>(
  t: Tr,
  err: unknown,
  fields: Record<string, Field>,
  showErrors: (errors: Partial<Record<Field, string>>) => void,
  errorParams?: Parameters<typeof errorMessage>[2],
) {
  const ae = toAppError(err);
  const field = ae.code === "invalid_input" && ae.field ? fields[ae.field] : undefined;
  if (field) showErrors({ [field]: errorMessage(t, err) } as Partial<Record<Field, string>>);
  else if (ae.code === "invalid_input" && ae.field === "host") toast(t("hosts.err.tooLarge"), "error");
  else toast(errorMessage(t, err, errorParams), "error");
}

/** Ctrl+S saves; Escape leaves an untouched form. Menus and dialogs get the key first. */
export function useHostEditorShortcuts(form: unknown, save: () => Promise<void>, goBack: () => void) {
  const initialJson = useRef(JSON.stringify(form));
  const dirty = JSON.stringify(form) !== initialJson.current;
  const latest = useRef({ save, goBack, dirty });
  latest.current = { save, goBack, dirty };
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.defaultPrevented || document.querySelector('[role="dialog"],[role="alertdialog"],[role="menu"]')) return;
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void latest.current.save();
      } else if (e.key === "Escape" && !latest.current.dirty) {
        latest.current.goBack();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

/** Confirms, deletes the host via `deleteHost`, reloads vault data, shows a toast, and navigates back. */
export function useDeleteHost(
  host: { id: string; name: string } | undefined,
  deleteHost: (id: string) => Promise<void>,
  goBack: () => void,
) {
  const t = useT();
  return async () => {
    if (!host) return;
    const ok = await confirm({
      title: t("hosts.deleteTitle", { name: host.name }),
      body: t("hosts.deleteBody"),
      confirmLabel: t("btn.delete"),
      danger: true,
    });
    if (!ok) return;
    try {
      await deleteHost(host.id);
      await useVaultData.getState().reload();
      toast(t("hosts.deleted", { name: host.name }));
      goBack();
    } catch (err) {
      toast(errorMessage(t, err), "error");
    }
  };
}
