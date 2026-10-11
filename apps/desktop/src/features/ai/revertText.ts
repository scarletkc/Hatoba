import type { MessageKey, Params } from "@/i18n";
import type { AiRevertFailure } from "@/ipc/types";

type Tr = (key: MessageKey, params?: Params) => string;

const FAILURES: Record<AiRevertFailure, MessageKey> = {
  disconnected: "ai.revert.failed.disconnected",
  other_server: "ai.revert.failed.otherServer",
  permission_denied: "ai.revert.failed.permissionDenied",
  not_text: "ai.revert.failed.notText",
  symbolic_link: "ai.revert.failed.symbolicLink",
  timed_out: "ai.revert.failed.timedOut",
  unfinished: "ai.revert.failed.unfinished",
  failed: "ai.revert.failed.failed",
};

/** Why a revert (AI-41) did not happen, as Rust reported it. `detail` names the server for `other_server`. */
export function revertFailure(t: Tr, reason: AiRevertFailure, detail: string | null): string {
  return t(FAILURES[reason], { server: detail ?? "" });
}
