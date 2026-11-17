import type { ProxyMode } from "@/ipc/types";

/** What a new host's form carries over when its protocol is switched (TELNET-01). */
export interface HostDraft {
  name: string;
  address: string;
  /** The port typed, unless it was the old protocol's default, which the new one replaces with its own. */
  port?: string;
  username: string;
  /** A password typed for the new host, which it keeps as the form did; never a saved one. */
  password: string;
  tags: string[];
  proxyMode: ProxyMode;
  proxyId: string | null;
  note: string;
  aiNotes: string;
  favorite: boolean;
}
