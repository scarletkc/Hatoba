import { detectLocale } from "@/i18n";
import type {
  ConflictView,
  DeviceView,
  FileEntry,
  ForwardView,
  GroupView,
  HostView,
  KeyView,
  LocalPrefs,
  SettingsView,
  SyncStatus,
} from "../types";

/** Sample data from the design file (`Hatoba.dc.html`), used when running in a plain browser. */

const zh = detectLocale() === "zh-CN";
const MIN = 60_000;
const DAY = 86_400_000;

function ago(ms: number) {
  return Date.now() - ms;
}

function at(month: number, day: number) {
  const d = new Date();
  return new Date(d.getFullYear(), month - 1, day, 10, 0).getTime();
}

export const GROUPS: GroupView[] = [
  { id: "g-tokyo", name: zh ? "东京生产" : "Tokyo Prod", parent_id: null, sort: 0 },
  { id: "g-osaka", name: zh ? "大阪数据库" : "Osaka DB", parent_id: null, sort: 1 },
  { id: "g-staging", name: "Staging", parent_id: null, sort: 2 },
  { id: "g-infra", name: zh ? "基础设施" : "Infrastructure", parent_id: null, sort: 3 },
  { id: "g-home", name: zh ? "家庭实验室" : "Homelab", parent_id: null, sort: 4 },
];

function host(
  id: string,
  name: string,
  username: string,
  address: string,
  port: number,
  group_id: string,
  tags: string[],
  last: number | null,
  extra: Partial<HostView> = {},
): HostView {
  return {
    id,
    name,
    address,
    port,
    username,
    auth_kind: "key",
    has_password: false,
    key_id: null,
    group_id,
    tags,
    favorite: false,
    jump_host_id: null,
    note: "",
    updated_at: ago(3 * DAY),
    last_connected_at: last,
    ...extra,
  };
}

export const HOSTS: HostView[] = [
  host("h-api-tokyo", "prod-api-tokyo", "deploy", "43.206.118.27", 22, "g-tokyo", ["production", "api"], ago(2 * MIN), {
    favorite: true,
    key_id: "k-deploy",
    note: zh
      ? "主 API 节点，部署走 GitHub Actions。重启 api.service 前先在 #ops 频道说一声。"
      : "Primary API node, deployed via GitHub Actions. Post in #ops before restarting api.service.",
  }),
  host("h-bastion", "bastion-tokyo", "ops", "bastion.tky.example.net", 2222, "g-tokyo", ["production", "edge"], ago(2 * MIN), {
    favorite: true,
    key_id: "k-bastion",
  }),
  host("h-db-osaka-01", "db-osaka-01", "postgres", "10.24.3.11", 22, "g-osaka", ["production", "database"], ago(18 * MIN), {
    favorite: true,
    key_id: "k-dbops",
    jump_host_id: "h-bastion",
  }),
  host("h-api-tokyo-02", "prod-api-tokyo-02", "deploy", "43.206.118.31", 22, "g-tokyo", ["production", "api"], ago(40 * MIN), {
    key_id: "k-deploy",
  }),
  host("h-staging-web", "staging-web-02", "ubuntu", "172.31.40.8", 22, "g-staging", ["staging"], ago(70 * MIN), {
    key_id: "k-deploy",
  }),
  host("h-db-osaka-02", "db-osaka-02", "postgres", "10.24.3.12", 22, "g-osaka", ["production", "database"], ago(DAY + 2 * MIN), {
    key_id: "k-dbops",
    jump_host_id: "h-bastion",
  }),
  host("h-staging-worker", "staging-worker-01", "ubuntu", "172.31.40.15", 22, "g-staging", ["staging"], at(10, 5), {
    key_id: "k-deploy",
  }),
  host("h-ci", "ci-runner-01", "runner", "192.168.50.21", 22, "g-infra", ["ci"], at(10, 3), {
    auth_kind: "password",
    has_password: true,
  }),
  host("h-edge-sg", "edge-sg-cache", "root", "159.89.204.73", 22, "g-infra", ["edge"], at(9, 24), { auth_kind: "ask" }),
  host("h-nas", "homelab-nas", "admin", "nas.local", 22, "g-home", ["personal"], at(9, 12), {
    favorite: true,
    key_id: "k-homelab",
  }),
  host("h-pihole", "pi-hole", "pi", "192.168.1.53", 22, "g-home", ["personal"], at(8, 30), {
    auth_kind: "password",
    has_password: true,
  }),
];

/** TCP probe results (HOST-10). staging-web-02 times out. */
export const PROBE: Record<string, boolean> = {
  "h-edge-sg": false,
  "h-pihole": false,
  "h-staging-web": false,
};

/** Hosts whose connection attempt fails in the mock (design §03b). */
export const FAILING_HOSTS = new Set(["h-staging-web"]);

export const KEYS: KeyView[] = [
  {
    id: "k-deploy",
    name: "deploy-ed25519",
    algorithm: "ed25519",
    bits: 256,
    public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHk2pQ8vXfR3mN0sLwT7eJ1cYb9GdUa4KoZq5hVr2Rz9 deploy@hatoba",
    fingerprint: "SHA256:4f9KxPq2mW7vZt1nRcL8yE3hJ0aDsUoB6gNiTkVfXw",
    comment: "deploy@hatoba",
    has_passphrase: true,
    created_at: at(3, 14),
    updated_at: at(3, 14),
    used_by: [],
  },
  {
    id: "k-dbops",
    name: "db-ops-ed25519",
    algorithm: "ed25519",
    bits: 256,
    public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIM3TzR8vWcQe1pYk7dLx0aN2bG5hJfU9sVr4Kq1La1Lw db-ops@hatoba",
    fingerprint: "SHA256:Qm3TzR8vWcYk7dLx0aN2bG5hJfU9sVr4Kqa1Lw0E",
    comment: "db-ops@hatoba",
    has_passphrase: false,
    created_at: at(5, 2),
    updated_at: at(5, 2),
    used_by: [],
  },
  {
    id: "k-bastion",
    name: "bastion-rsa",
    algorithm: "rsa",
    bits: 4096,
    public_key: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQC9hLpWd2cJe… ops@bastion",
    fingerprint: "SHA256:9hLpWd2cJeR4tXa8Vn3QmK0sB7uZyE6oYx7MZo",
    comment: "ops@bastion",
    has_passphrase: false,
    created_at: new Date(2025, 10, 20).getTime(),
    updated_at: new Date(2025, 10, 20).getTime(),
    used_by: [],
  },
  {
    id: "k-homelab",
    name: "homelab-ecdsa",
    algorithm: "ecdsa",
    bits: 256,
    public_key: "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBc0Vn5RsEu admin@nas",
    fingerprint: "SHA256:c0Vn5RsEuBq2Lm8Ty4Wd1Zx7Pa3Hj6Kq4Ghd",
    comment: "admin@nas",
    has_passphrase: false,
    created_at: new Date(2024, 7, 9).getTime(),
    updated_at: new Date(2024, 7, 9).getTime(),
    used_by: [],
  },
  {
    id: "k-github",
    name: "github-signing",
    algorithm: "ed25519",
    bits: 256,
    public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIUe7bJ2xNoLpT3fRk mika@github",
    fingerprint: "SHA256:Ue7bJ2xNoL8Qw5Rt1Yh4Mz9Vc2Bn6XpT3fRk",
    comment: "mika@github",
    has_passphrase: false,
    created_at: at(9, 30),
    updated_at: at(9, 30),
    used_by: [],
  },
];

export const FORWARDS: ForwardView[] = [
  { id: "f-pg", host_id: "h-db-osaka-01", bind_address: "127.0.0.1", bind_port: 15432, dest_host: "127.0.0.1", dest_port: 5432, auto_start: true },
  { id: "f-grafana", host_id: "h-api-tokyo", bind_address: "127.0.0.1", bind_port: 3000, dest_host: "grafana.internal", dest_port: 3000, auto_start: false },
];

export const SETTINGS: SettingsView = {
  terminal: {
    font_family: "Cascadia Mono",
    font_size: 13,
    theme: "dark",
    cursor_style: "block",
    scrollback: 10000,
    right_click: "copy_paste",
    confirm_multiline_paste: true,
  },
  auto_lock_minutes: 15,
  lock_disconnects_sessions: false,
};

export const PREFS: LocalPrefs = {
  language: "system",
  appearance: "system",
  density: "regular",
  host_probe: true,
  auto_update_check: false,
};

export function syncStatus(): SyncStatus {
  return {
    kind: "worker",
    endpoint: "hatoba-sync.mika-dev.workers.dev",
    database: "hatoba · APAC",
    state: "idle",
    last_synced_at: ago(2 * MIN),
    pending: 0,
    conflicts: 0,
    auto_sync: true,
    message: null,
    counts: { hosts: HOSTS.length, keys: KEYS.length, groups: GROUPS.length },
  };
}

export const DEVICES: DeviceView[] = [
  { device_id: "d-1", name: "ThinkPad X1 Carbon", platform: "Windows 11", created_at: at(3, 1), last_seen: ago(0), current: true },
  { device_id: "d-2", name: zh ? "Mac mini · 办公室" : "Mac mini · Office", platform: "macOS 15.3", created_at: at(3, 2), last_seen: ago(2 * 60 * MIN), current: false },
  { device_id: "d-3", name: "MacBook Pro 14″", platform: "macOS 15.4", created_at: at(4, 2), last_seen: ago(3 * DAY), current: false },
  { device_id: "d-4", name: "fedora-ws", platform: "Fedora 41", created_at: at(5, 2), last_seen: at(9, 21), current: false },
];

export const CONFLICTS: ConflictView[] = [
  {
    id: 1,
    item_id: "h-api-tokyo",
    item_type: "host",
    item_name: "prod-api-tokyo",
    resolution: "local_won",
    local_updated_at: ago(30 * MIN),
    remote_updated_at: ago(56 * MIN),
    local_deleted: false,
    remote_deleted: false,
    fields: [
      { field: "port", local: "22", remote: "2222" },
      { field: "jump_host", local: "bastion-tokyo", remote: null },
    ],
    created_at: ago(20 * MIN),
  },
  {
    id: 2,
    item_id: "k-dbops",
    item_type: "key",
    item_name: "db-ops-2026",
    resolution: "modified_won",
    local_updated_at: ago(DAY - 3 * 60 * MIN),
    remote_updated_at: ago(DAY + 2 * 60 * MIN),
    local_deleted: false,
    remote_deleted: true,
    fields: [{ field: "name", local: "db-ops-2026", remote: null }],
    created_at: ago(DAY),
  },
];

export const HOME_FILES: FileEntry[] = [
  { name: ".cache", dir: true },
  { name: ".ssh", dir: true },
  { name: "backups", dir: true },
  { name: ".bashrc", size: 3771 },
  { name: ".profile", size: 807 },
  { name: "notes.md", size: 1290 },
].map((f) => file("/home/deploy", f));

export const API_FILES: FileEntry[] = [
  { name: ".github", dir: true, date: "10/02" },
  { name: "migrations", dir: true, date: "10/06" },
  { name: "src", dir: true, date: "10/08" },
  { name: "target", dir: true, date: "10/08" },
  { name: ".env.production", size: 612, date: "09/28" },
  { name: "Cargo.lock", size: 84 * 1024, date: "10/07" },
  { name: "Cargo.toml", size: 2150, date: "10/07" },
  { name: "Dockerfile", size: 1330, date: "09/30" },
  { name: "README.md", size: 3480, date: "08/19" },
  { name: "wrangler.toml", size: 418, date: "10/01" },
].map((f) => file("/srv/api", f));

function file(dir: string, f: { name: string; dir?: boolean; size?: number; date?: string }): FileEntry {
  const [m, d] = (f.date ?? "10/08").split("/").map(Number);
  return {
    name: f.name,
    path: `${dir}/${f.name}`,
    is_dir: !!f.dir,
    is_symlink: false,
    size: f.size ?? 4096,
    modified: at(m, d),
    permissions: f.dir ? "drwxr-xr-x" : f.name.startsWith(".env") ? "-rw-------" : "-rw-r--r--",
  };
}
