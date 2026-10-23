import { useState } from "react";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { Button, TextField } from "@/components/controls";
import { Dialog, confirm, toast } from "@/components/overlay";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { GroupView } from "@/ipc/types";

/** Names a new group, or renames `group`. */
export function GroupDialog({
  group,
  count,
  onClose,
}: {
  group: GroupView | null;
  count: number;
  onClose: () => void;
}) {
  const t = useT();
  const [name, setName] = useState(group?.name ?? "");
  const [busy, setBusy] = useState(false);
  const save = async () => {
    if (busy || !name.trim()) return;
    setBusy(true);
    try {
      await api.group_save({
        id: group?.id ?? null,
        name: name.trim(),
        parent_id: group?.parent_id ?? null,
        sort: group?.sort ?? count,
      });
      await useVaultData.getState().reload();
      onClose();
    } catch (e) {
      toast(errorMessage(t, e), "error");
      setBusy(false);
    }
  };
  return (
    <Dialog
      title={group ? t("sidebar.renameGroup") : t("sidebar.newGroup")}
      icon="folder-simple"
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>{t("btn.cancel")}</Button>
          <Button variant="primary" busy={busy} disabled={!name.trim()} onClick={() => void save()}>
            {t("btn.save")}
          </Button>
        </>
      }
    >
      <form
        style={{ marginTop: 14 }}
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <TextField
          large
          autoFocus
          value={name}
          placeholder={t("sidebar.groupName")}
          aria-label={t("sidebar.groupName")}
          maxLength={64}
          onChange={(e) => setName(e.target.value)}
        />
      </form>
    </Dialog>
  );
}

/** Deletes `group` after asking; its hosts stay, without a group. Resolves to whether it was deleted. */
export async function deleteGroup(t: ReturnType<typeof useT>, group: GroupView): Promise<boolean> {
  if (
    !(await confirm({
      title: t("sidebar.deleteGroup"),
      body: t("sidebar.deleteGroupConfirm", { name: group.name }),
      confirmLabel: t("btn.delete"),
      danger: true,
    }))
  )
    return false;
  try {
    await api.group_delete(group.id);
    await useVaultData.getState().reload();
    return true;
  } catch (e) {
    toast(errorMessage(t, e), "error");
    return false;
  }
}
