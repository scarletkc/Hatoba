import { Button, Dialog, Icon } from "@hatoba/ui";
import { useT, type MessageKey } from "@/i18n";
import s from "./Dialogs.module.css";

const POINTS: { icon: string; key: MessageKey }[] = [
  { icon: "lock-key", key: "sync.enc.item" },
  { icon: "key", key: "sync.enc.vault" },
  { icon: "cloud", key: "sync.enc.cloud" },
];

/** "How is my data encrypted?" (spec §4: AES-256-GCM per item, vault key wrapped via Argon2id). */
export function EncryptionDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  return (
    <Dialog
      title={t("sync.enc.title")}
      icon="shield-check"
      onClose={onClose}
      actions={
        <Button variant="primary" onClick={onClose}>
          {t("btn.close")}
        </Button>
      }
    >
      <ul className={s.points}>
        {POINTS.map((p) => (
          <li key={p.key} className={s.point}>
            <Icon name={p.icon} className={s.pointIcon} />
            <span>{t(p.key)}</span>
          </li>
        ))}
      </ul>
    </Dialog>
  );
}
