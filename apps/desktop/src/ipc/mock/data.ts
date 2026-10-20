import { mockLocale } from "./visitor";
import type {
  AvailableUpdate,
  ConflictView,
  DeviceView,
  FileEntry,
  ForwardView,
  GroupView,
  HostView,
  ImportCandidate,
  ImportSource,
  KeyView,
  LocalPrefs,
  ProxyView,
  QuickTarget,
  ServerStatsView,
  SettingsView,
  SyncStatus,
} from "../types";

/** Sample data from the design file (`Hatoba.dc.html`), used when running in a plain browser. */

const zh = mockLocale() === "zh-CN";
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
  { id: "g-tokyo", name: zh ? "东京生产" : "Tokyo Prod", parent_id: null, sort: 0, dashboard: true },
  { id: "g-osaka", name: zh ? "大阪数据库" : "Osaka DB", parent_id: null, sort: 1, dashboard: true },
  { id: "g-staging", name: "Staging", parent_id: null, sort: 2, dashboard: true },
  { id: "g-infra", name: zh ? "基础设施" : "Infrastructure", parent_id: null, sort: 3, dashboard: true },
  { id: "g-home", name: zh ? "家庭实验室" : "Homelab", parent_id: null, sort: 4, dashboard: true },
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
    proxy_mode: "device_default",
    proxy_id: null,
    note: "",
    ai_notes: "",
    updated_at: ago(3 * DAY),
    last_connected_at: last,
    os: null,
    show_stats: false,
    dashboard: true,
    env: [],
    ...extra,
  };
}

export const HOSTS: HostView[] = [
  host("h-api-tokyo", "prod-api-tokyo", "deploy", "43.206.118.27", 22, "g-tokyo", ["production", "api"], ago(2 * MIN), {
    os: "ubuntu",
    show_stats: true,
    favorite: true,
    key_id: "k-deploy",
    // SSH-14: sent when a terminal opens.
    env: [
      { name: "TZ", value: "Asia/Tokyo" },
      { name: "LANG", value: "en_US.UTF-8" },
    ],
    note: zh
      ? "主 API 节点，部署走 GitHub Actions。重启 api.service 前先在 #ops 频道说一声。"
      : "Primary API node, deployed via GitHub Actions. Post in #ops before restarting api.service.",
  }),
  host("h-bastion", "bastion-tokyo", "ops", "bastion.tky.example.net", 2222, "g-tokyo", ["production", "edge"], ago(2 * MIN), {
    os: "debian",
    favorite: true,
    key_id: "k-bastion",
  }),
  host("h-db-osaka-01", "db-osaka-01", "postgres", "10.24.3.11", 22, "g-osaka", ["production", "database"], ago(18 * MIN), {
    os: "debian",
    favorite: true,
    key_id: "k-dbops",
    jump_host_id: "h-bastion",
    // AI-37: what the assistant is told about this host.
    ai_notes: zh
      ? "PostgreSQL 16 主库，数据目录 /srv/pgdata。工作时间（09:00–19:00）不要重启 postgresql。"
      : "PostgreSQL 16 primary, data in /srv/pgdata. Don’t restart postgresql during business hours (09:00–19:00).",
  }),
  host("h-api-tokyo-02", "prod-api-tokyo-02", "deploy", "43.206.118.31", 22, "g-tokyo", ["production", "api"], ago(40 * MIN), {
    os: "ubuntu",
    key_id: "k-deploy",
  }),
  host("h-staging-web", "staging-web-02", "ubuntu", "172.31.40.8", 22, "g-staging", ["staging"], ago(70 * MIN), {
    os: "ubuntu",
    key_id: "k-deploy",
  }),
  host("h-db-osaka-02", "db-osaka-02", "postgres", "10.24.3.12", 22, "g-osaka", ["production", "database"], ago(DAY + 2 * MIN), {
    key_id: "k-dbops",
    jump_host_id: "h-bastion",
  }),
  host("h-staging-worker", "staging-worker-01", "ubuntu", "172.31.40.15", 22, "g-staging", ["staging"], at(10, 5), {
    os: "ubuntu",
    key_id: "k-deploy",
  }),
  host("h-ci", "ci-runner-01", "runner", "192.168.50.21", 22, "g-infra", ["ci"], at(10, 3), {
    os: "windows",
    auth_kind: "password",
    has_password: true,
  }),
  host("h-edge-sg", "edge-sg-cache", "root", "159.89.204.73", 22, "g-infra", ["edge"], at(9, 24), {
    auth_kind: "ask",
    proxy_mode: "proxy",
    proxy_id: "p-office",
  }),
  host("h-nas", "homelab-nas", "admin", "nas.local", 22, "g-home", ["personal"], at(9, 12), {
    os: "freebsd",
    favorite: true,
    key_id: "k-homelab",
  }),
  host("h-pihole", "pi-hole", "pi", "192.168.1.53", 22, "g-home", ["personal"], at(8, 30), {
    os: "raspbian",
    auth_kind: "password",
    has_password: true,
  }),
];

/** Saved proxies (SSH-13); `host_ids` is filled in from the hosts. */
export const PROXIES: ProxyView[] = [
  {
    id: "p-clash",
    name: "Clash",
    kind: "socks5",
    address: "127.0.0.1",
    port: 7890,
    username: "",
    has_password: false,
    host_ids: [],
    updated_at: ago(5 * DAY),
  },
  {
    id: "p-office",
    name: zh ? "办公室代理" : "Office proxy",
    kind: "http",
    address: "proxy.corp.example.com",
    port: 3128,
    username: "kc",
    has_password: true,
    host_ids: [],
    updated_at: ago(12 * DAY),
  },
];

/** TCP probe results (HOST-10). staging-web-02 times out. */
export const PROBE: Record<string, boolean> = {
  "h-edge-sg": false,
  "h-pihole": false,
  "h-staging-web": false,
};

/** Hosts whose connection attempt fails in the mock (design §03b). */
export const FAILING_HOSTS = new Set(["h-staging-web"]);

/** Quick-connect targets (HOST-12) this "device" connected to before. */
export const RECENT_TARGETS: QuickTarget[] = [
  { address: "10.0.8.21", port: 22, username: "root" },
  { address: "raspberrypi.local", port: 22, username: "pi" },
  { address: "build-runner.internal", port: 2222, username: "ci" },
];

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
    dark_scheme: null,
    light_scheme: null,
    cursor_style: "block",
    scrollback: 10000,
    right_click: "copy_paste",
    confirm_multiline_paste: true,
  },
};

export const PREFS: LocalPrefs = {
  language: "system",
  appearance: "system",
  density: "regular",
  host_probe: true,
  auto_update_check: true,
  ai_permission_mode: "manual",
  ai_bypass_confirmed: false,
  ai_notifications: true,
  ai_panel_open: false,
  ai_panel_width: 380,
  default_proxy_id: null,
  auto_lock_minutes: 15,
  lock_on_sleep: true,
  lock_disconnects_sessions: false,
  clipboard_clear_secs: 90,
  dashboard_layout: "cards",
  dashboard_hidden_columns: [],
  dashboard_interval_secs: 3,
  dashboard_cpu_alert: 90,
  dashboard_mem_alert: 90,
  dashboard_disk_alert: 90,
  close_action: "ask",
  local_shell: null,
};

/** The release that `?update=available` finds; its notes are shaped like the ones `release.mjs publish` writes. */
export const AVAILABLE_UPDATE: AvailableUpdate = {
  version: "0.2.0",
  notes: [
    "## Signed updates",
    "Hatoba now downloads and installs new versions from **Settings → About**, and checks each installer's signature before it runs.",
    "## Changelog",
    "Changes since v0.1.0:",
    "### Features",
    "- **desktop:** install signed updates from the About page ([#58](https://github.com/scarletkc/Hatoba/pull/58))\n- **desktop:** quick connect from the hosts search field ([#53](https://github.com/scarletkc/Hatoba/pull/53))",
    "### Fixes",
    "- **ai:** record tools/list_changed before the answer that follows it ([#54](https://github.com/scarletkc/Hatoba/pull/54))",
    "[Full diff](https://github.com/scarletkc/Hatoba/compare/v0.1.0...v0.2.0)",
  ].join("\n\n"),
  published_at: ago(3 * DAY),
  release_url: "https://github.com/scarletkc/Hatoba/releases/tag/v0.2.0",
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
    worker_update: null,
    too_large: [],
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
      { field: "jump_host", local: "host:bastion-tokyo", remote: null },
      { field: "proxy", local: zh ? "proxy:办公室代理" : "proxy:Office proxy", remote: "deleted" },
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

const GIB = 1024 ** 3;

/** A server the mock resource usage pretends to be: its size, how busy it runs, and its filesystems in GiB. */
export interface StatsProfile {
  cpus: number;
  memGib: number;
  /** Where the CPU share starts and the share of memory used. */
  cpu: number;
  mem: number;
  /** Mount point, used and total GiB; the first is `/`. */
  disks: [string, number, number][];
  uptimeDays: number;
  /** Seconds between readings. */
  interval: number;
}

const DEFAULT_PROFILE: StatsProfile = {
  cpus: 4,
  memGib: 8,
  cpu: 18,
  mem: 0.46,
  disks: [
    ["/", 31, 80],
    ["/boot", 0.2, 1],
  ],
  uptimeDays: 12,
  interval: 2,
};

/** A random walk of a Linux server's resource usage, by default a 4-CPU, 8 GB one, one reading per call (TERM-12, HOST-14). */
export function mockStats(profile: Partial<StatsProfile> = {}): (first: boolean) => ServerStatsView {
  const p = { ...DEFAULT_PROFILE, ...profile };
  const walk = (v: number, step: number, min: number, max: number) => Math.min(max, Math.max(min, v + (Math.random() - 0.5) * step));
  let cpu = p.cpu;
  let mem = p.mem;
  let rx = 180_000;
  let tx = 42_000;
  let rxTotal = 412 * GIB;
  let txTotal = 96 * GIB;
  let uptime = p.uptimeDays * 86_400 + 3 * 3600 + 17 * 60;
  const mounts = p.disks.map(([mount_point, used, total], i) => ({
    source: i === 0 ? "/dev/sda1" : `/dev/sd${String.fromCharCode(98 + i)}1`,
    mount_point,
    total: total * GIB,
    used: used * GIB,
    available: Math.max(0, total * 0.95 - used) * GIB,
  }));
  return (first) => {
    cpu = walk(cpu, 16, 2, 97);
    mem = walk(mem, 0.012, Math.max(0.05, p.mem - 0.1), Math.min(0.99, p.mem + 0.1));
    rx = walk(rx, 120_000, 8_000, 900_000);
    tx = walk(tx, 30_000, 2_000, 200_000);
    if (!first) {
      uptime += p.interval;
      rxTotal += rx * p.interval;
      txTotal += tx * p.interval;
    }
    // Now and then a download: a burst of traffic and CPU.
    const burst = Math.random() < 0.08;
    const load = (cpu / 100) * p.cpus;
    return {
      cpu_percent: first ? null : Math.round((burst ? Math.min(99, cpu + 35) : cpu) * 10) / 10,
      cpus: p.cpus,
      load: [Math.round(load * 100) / 100, Math.round(load * 80) / 100, Math.round(load * 60) / 100],
      mem_total: p.memGib * GIB,
      mem_used: Math.round(mem * p.memGib * GIB),
      swap_total: 2 * GIB,
      swap_used: Math.round(0.06 * GIB),
      net_rx_rate: first ? null : Math.round(burst ? rx * 9 : rx),
      net_tx_rate: first ? null : Math.round(tx),
      net_interfaces: ["eth0"],
      disk_total: mounts[0].total,
      disk_used: mounts[0].used,
      disk_available: mounts[0].available,
      uptime_secs: uptime,
      net_rx_total: Math.round(rxTotal),
      net_tx_total: Math.round(txTotal),
      mounts: mounts.map((m) => ({ ...m })),
    };
  };
}

type Candidate = Omit<ImportCandidate, "exists">;

const candidate = (c: Partial<Candidate> & Pick<Candidate, "id" | "address">): Candidate => ({
  name: c.id,
  port: 22,
  username: "root",
  folder: null,
  identity_file: null,
  identity_file_found: false,
  kept_key: null,
  jump: null,
  proxy: null,
  skipped_proxy: null,
  ...c,
});

/** What each import source holds in the demo. */
export function importCandidates(source: ImportSource): Candidate[] {
  const prod = zh ? "生产" : "Production";
  switch (source) {
    case "ssh_config":
      return [
        candidate({ id: "github-runner", address: "10.0.4.20", username: "runner", identity_file: "~/.ssh/id_ed25519", identity_file_found: true }),
        candidate({ id: "bastion-tokyo", address: "bastion.tky.example.net", port: 2222, username: "ops", identity_file: "~/.ssh/id_ed25519", identity_file_found: true }),
        candidate({ id: "minecraft", address: "mc.example.org", username: "mc", identity_file: "~/.ssh/mc_rsa", jump: "bastion-tokyo" }),
        candidate({ id: "build-arm", address: "10.0.4.31", username: "ci", identity_file: "~/.ssh/build_ed25519", identity_file_found: true }),
        candidate({ id: "lab-gpu", address: "gpu.lab.internal", username: "kc", skipped_proxy: "ProxyCommand nc -X 5 -x 127.0.0.1:7890 %h %p" }),
      ];
    case "xshell":
    case "finalshell":
      return [
        candidate({ id: "web-01", address: "10.10.1.11", folder: `${prod} / Web` }),
        candidate({ id: "web-02", address: "10.10.1.12", folder: `${prod} / Web` }),
        candidate({ id: "mysql-master", address: "10.10.2.5", port: 2222, username: "dba", folder: `${prod} / DB`, kept_key: source === "xshell" ? "id_rsa_2048" : "" }),
        candidate({ id: "staging", address: "staging.example.cn", username: "deploy", folder: zh ? "测试" : "Staging" }),
        candidate({ id: "bastion-tokyo", address: "bastion.tky.example.net", port: 2222, username: "ops", skipped_proxy: source === "xshell" ? "Xshell proxy office" : null }),
      ];
    case "mobaxterm":
      return [
        candidate({ id: "Lab\\gpu-01", name: "gpu-01", address: "gpu01.lab.internal", username: "kc", folder: "Lab", identity_file: "C:\\Users\\demo\\.ssh\\id_ed25519", identity_file_found: true, jump: "jump@bastion.lab.internal:22" }),
        candidate({ id: "Lab\\gpu-02", name: "gpu-02", address: "gpu02.lab.internal", username: "kc", folder: "Lab", identity_file: "C:\\Users\\demo\\.ssh\\id_ed25519", identity_file_found: true, jump: "jump@bastion.lab.internal:22" }),
        candidate({ id: "nas", address: "192.168.1.20", username: "admin", proxy: "SOCKS5 127.0.0.1:1080" }),
      ];
    case "putty":
      return [
        candidate({ id: "router", address: "192.168.1.1", username: "admin" }),
        candidate({ id: "vps-la", address: "203.0.113.40", identity_file: "C:\\Users\\demo\\keys\\vps.ppk", identity_file_found: true }),
        candidate({ id: "old-box", address: "10.0.9.9", skipped_proxy: "SOCKS4 proxy 10.0.0.1:1080" }),
      ];
    case "securecrt":
      return [
        candidate({ id: "Jump/bastion", name: "bastion", address: "bastion.example.net", username: "ops", folder: "Jump" }),
        candidate({ id: "HQ/Core/core-sw1", name: "core-sw1", address: "10.1.1.1", username: "admin", folder: "HQ / Core", jump: "bastion" }),
        candidate({ id: "HQ/Core/core-sw2", name: "core-sw2", address: "10.1.1.2", username: "admin", folder: "HQ / Core", jump: "bastion" }),
        candidate({ id: "HQ/Servers/web-01", name: "web-01", address: "10.1.2.11", username: "deploy", folder: "HQ / Servers", identity_file: "C:\\Users\\demo\\Documents\\keys\\web.ppk", identity_file_found: true }),
        candidate({ id: "Lab/lab-rtr", name: "lab-rtr", address: "192.168.50.1", folder: "Lab", skipped_proxy: "SecureCRT firewall Lab SOCKS" }),
      ];
    case "tabby":
      return [
        candidate({ id: "ssh:custom:bastion:1", name: "bastion", address: "bastion.example.org", username: "jump" }),
        candidate({ id: "ssh:custom:api:1", name: "api", address: "10.20.0.8", username: "deploy", folder: "Work / API", jump: "bastion" }),
        candidate({ id: "ssh:custom:home:1", name: "home-server", address: "home.example.net", port: 2200, username: "kc", identity_file: "~/.ssh/id_ed25519", identity_file_found: true }),
      ];
  }
}

/** Where each import source is read from in the demo. */
export function importLocation(source: ImportSource, platform: string): string | null {
  const windows = platform === "windows";
  switch (source) {
    case "ssh_config":
      return windows ? "C:\\Users\\demo\\.ssh\\config" : "/home/demo/.ssh/config";
    case "xshell":
      return windows ? "C:\\Users\\demo\\Documents\\NetSarang Computer\\8\\Xshell\\Sessions" : null;
    case "finalshell":
      return windows ? "C:\\Users\\demo\\AppData\\Local\\finalshell\\conn" : "/home/demo/.finalshell/conn";
    case "mobaxterm":
      return windows ? "C:\\Users\\demo\\AppData\\Roaming\\MobaXterm\\MobaXterm.ini" : null;
    case "putty":
      return windows ? "HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions" : null;
    case "securecrt":
      return windows ? "C:\\Users\\demo\\AppData\\Roaming\\VanDyke\\Config\\Sessions" : "/home/demo/.vandyke/SecureCRT/Config/Sessions";
    case "tabby":
      return windows ? "C:\\Users\\demo\\AppData\\Roaming\\tabby\\config.yaml" : "/home/demo/.config/tabby/config.yaml";
  }
}
