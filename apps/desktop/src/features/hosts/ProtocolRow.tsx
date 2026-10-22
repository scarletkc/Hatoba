import { Segmented } from "@/components/controls";
import { FormRow } from "@/components/layout";
import { useT } from "@/i18n";

export type HostProtocol = "ssh" | "telnet";

/** What a new host's form carries over when its protocol is switched (TELNET-01). */
export interface HostDraft {
  name: string;
  address: string;
  /** The port typed, unless it was the old protocol's default, which the new one replaces with its own. */
  port?: string;
  username: string;
  tags: string[];
  note: string;
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
