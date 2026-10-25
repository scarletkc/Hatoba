import { useEffect, useState } from "react";
import { useApp } from "@/app/store";
import { SyncConflictsView } from "./SyncConflictsView";
import { SyncStatusView } from "./SyncStatusView";
import { SyncWizard } from "./SyncWizard";
import s from "./Sync.module.css";

/**
 * Cloud Sync: the setup wizard while sync is off, otherwise the status page, with a
 * review screen for automatically resolved conflicts (spec §6.4).
 */
export function SyncPage() {
  const sync = useApp((st) => st.sync);
  const [reviewing, setReviewing] = useState(false);
  const kind = sync?.kind;

  // Leaving sync (disconnect) must not keep us on the conflicts screen next time.
  useEffect(() => {
    if (kind === "none") setReviewing(false);
  }, [kind]);

  if (!sync) return <div className={s.page} />;
  if (sync.kind === "none") return <SyncWizard />;
  if (reviewing) return <SyncConflictsView onClose={() => setReviewing(false)} />;
  return <SyncStatusView status={sync} onReview={() => setReviewing(true)} />;
}
