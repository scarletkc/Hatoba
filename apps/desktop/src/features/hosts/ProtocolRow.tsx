import { FormRow, Segmented } from "@hatoba/ui";
import { useT } from "@/i18n";

export type HostProtocol = "ssh" | "telnet";

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
