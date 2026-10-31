import { FormRow, Segmented } from "@hatoba/ui";
import { useT } from "@/i18n";
import type { ProxyMode } from "@/ipc/types";

export type HostProtocol = "ssh" | "telnet";

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

/** A new host's protocol, at the top of the host editors: SSH, or Telnet (TELNET-01). */
export function ProtocolRow({ value, onChange }: { value: HostProtocol; onChange(protocol: HostProtocol): void }) {
  const t = useT();
  return (
    <FormRow label={t("hosts.f.protocol")}>
      <Segmented
        ariaLabel={t("hosts.f.protocol")}
        value={value}
        options={[
          { value: "ssh" as const, label: "SSH", icon: "terminal-window" },
          { value: "telnet" as const, label: "Telnet", icon: "network" },
        ]}
        onChange={onChange}
      />
    </FormRow>
  );
}
