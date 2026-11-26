import { hostPort, reachableAddress } from "@/lib/address";
import { sameTarget } from "@/session/quickConnect";
import { withCommand } from "@/session/suggest";
import { appError, type FrameHandler, type HatobaApi } from "../api";
import type {
  AppError,
  ConnectionsEvent,
  EventMap,
  ForwardView,
  HostView,
  QuickTarget,
  RunningForwardView,
  SshErrorKind,
} from "../types";
import { FRAME_CLOSED, FRAME_DATA, FRAME_SESSION } from "../types";
import * as D from "./data";
import { mockStats } from "./data";
import { delay, fail, frame, type MockDeps } from "./shared";
import { FakeShell } from "./shell";

/** `?ssh=<kind>`: every connection fails with this SSH error kind and a detail like the real one. */
const SSH_DEMO_DETAIL: Partial<Record<SshErrorKind, (address: string, port: number) => string>> &
  Record<"timeout", (address: string, port: number) => string> = {
  timeout: (a, p) => `connect to ${a}:${p}: operation timed out`,
  refused: (a, p) => `connect to ${a}:${p}: connection refused`,
  dns: (a) => `failed to lookup address information: ${a}: nodename nor servname provided, or not known`,
  unreachable: (a, p) => `connect to ${a}:${p}: network is unreachable`,
  auth_failed: () => "authentication failed: no method succeeded (tried publickey, password)",
  disconnected: () => "the server closed the connection during the handshake",
  proxy_unreachable: () => "cannot reach the proxy 127.0.0.1:7890: connect to 127.0.0.1:7890: connection refused",
  proxy_auth: () => "the proxy proxy.corp.example.com:3128 rejected the username or password (HTTP 407)",
  proxy: (a, p) => `the proxy could not connect to ${a}:${p}: connection not allowed by its rules (0x02)`,
};

type SshApi = Pick<
  HatobaApi,
  | "ssh_connect"
  | "ssh_connect_target"
  | "command_history_list"
  | "command_history_add"
  | "command_history_remove"
  | "command_history_clear"
  | "recent_targets_list"
  | "recent_target_remove"
  | "ssh_write"
  | "ssh_write_bytes"
  | "ssh_resize"
  | "ssh_disconnect"
  | "ssh_stats_start"
  | "ssh_stats_stop"
  | "ssh_suggestions"
  | "ssh_test"
  | "hostkey_respond"
  | "auth_prompt_respond"
  | "forwards_list_all"
  | "forward_save"
  | "forward_delete"
  | "forward_start"
  | "forward_stop"
  | "connections_list"
>;

export interface SshMockDeps extends Pick<MockDeps, "state" | "emit" | "needUnlocked" | "id" | "touch"> {
  shells: Map<string, FakeShell>;
  q: URLSearchParams;
  demo: string | null;
  sshDemo: string | null;
}

export function createSshMock(ctx: SshMockDeps): SshApi & { onHostDeleted(hostId: string): void } {
  const { state, emit, needUnlocked, id, touch, shells, q, demo, sshDemo } = ctx;

  let forwards: ForwardView[] = demo === "empty" ? [] : D.FORWARDS.map((f) => ({ ...f }));
  let recentTargets: QuickTarget[] = demo === "empty" ? [] : D.RECENT_TARGETS.map((r) => ({ ...r }));
  // TERM-15: each host's and local shell's commands, newest first; the demo starts every one with a few.
  const commandHistory = new Map<string, string[]>();
  let historySeeded = demo !== "empty";
  const historyOf = (owner: string) => commandHistory.get(owner) ?? (historySeeded ? [...D.COMMAND_HISTORY] : []);
  const trustedQuickHosts = new Set(recentTargets.map((r) => `${r.address}:${r.port}`));
  /**
   * SSH-16: each host's shared connection, with its tabs and forwards. `?tunnel=lost` drops a host's connection 5 s
   * after it opens, as a network failure would; `?fwd=busy` fails the auto-start forwards, as a port in use would.
   */
  const lostDemo = q.get("tunnel") === "lost";
  const busyDemo = q.get("fwd") === "busy";
  type MockConnection = {
    name: string;
    tabs: Set<string>;
    forwards: Map<string, Omit<RunningForwardView, "forward_id">>;
    failed: Map<string, AppError>;
  };
  const connections = new Map<string, MockConnection>();
  let connectionsVersion = 0;
  const connectionList = (): ConnectionsEvent => ({
    version: connectionsVersion,
    connections: [...connections.entries()].map(([host_id, c]) => ({
      host_id,
      host_name: c.name,
      forwards: [...c.forwards.entries()]
        .map(([forward_id, f]) => ({ forward_id, ...f }))
        .sort((a, b) => a.port - b.port),
      failed: [...c.failed.entries()].map(([forward_id, error]) => ({ forward_id, error })),
    })),
  });
  const connectionsChanged = () => {
    connectionsVersion++;
    emit("ssh://connections", connectionList());
  };
  /** The connection closes once no tab and no forward is left on it. */
  const closeIfUnused = (hostId: string) => {
    const c = connections.get(hostId);
    if (c && c.tabs.size === 0 && c.forwards.size === 0) connections.delete(hostId);
  };
  const runForward = (c: MockConnection, f: ForwardView): number => {
    const port = f.bind_port || 49152 + Math.floor(Math.random() * 1000);
    c.forwards.set(f.id, {
      name: f.name,
      kind: f.kind,
      port,
      address: f.kind === "remote" ? hostPort(f.bind_address, port) : reachableAddress(f.bind_address, port),
      dest: f.kind === "dynamic" ? "" : hostPort(f.dest_host, f.dest_port),
    });
    c.failed.delete(f.id);
    return port;
  };
  /** Opens the host's connection and starts its auto-start forwards on it (FWD-02). */
  const openConnection = (hostId: string, name: string): MockConnection => {
    const c: MockConnection = { name, tabs: new Set(), forwards: new Map(), failed: new Map() };
    connections.set(hostId, c);
    state.hosts = state.hosts.map((x) => (x.id === hostId ? { ...x, last_connected_at: Date.now() } : x));
    for (const f of forwards.filter((x) => x.host_id === hostId && x.auto_start)) {
      if (busyDemo) {
        c.failed.set(
          f.id,
          appError("ssh", `cannot listen on ${f.bind_address}:${f.bind_port}: address already in use`, {
            ssh_kind: "io",
          }),
        );
      } else runForward(c, f);
    }
    if (lostDemo) {
      setTimeout(() => {
        if (connections.get(hostId) !== c) return;
        connections.delete(hostId);
        for (const sid of c.tabs) {
          shells.get(sid)?.stop();
          shells.delete(sid);
          emit("ssh://state", {
            session_id: sid,
            host_id: hostId,
            state: "disconnected",
            latency_ms: null,
            error: appError("ssh", "keepalive timeout: no reply from the server", { ssh_kind: "disconnected" }),
            exit_status: null,
          });
        }
        if (c.forwards.size > 0) {
          emit("ssh://forwards-lost", {
            host_id: hostId,
            host_name: c.name,
            error: appError("ssh", "keepalive timeout: no reply from the server", { ssh_kind: "disconnected" }),
          });
        }
        connectionsChanged();
      }, 5000);
    }
    return c;
  };
  const leaveConnection = (sid: string) => {
    for (const [hostId, c] of connections) {
      if (!c.tabs.delete(sid)) continue;
      closeIfUnused(hostId);
      connectionsChanged();
    }
  };
  const stopForward = (hostId: string, forwardId: string) => {
    const c = connections.get(hostId);
    if (!c) return;
    const found = c.forwards.delete(forwardId) || c.failed.delete(forwardId);
    if (!found) return;
    closeIfUnused(hostId);
    connectionsChanged();
  };
  /** TERM-12: the mock resource usage of each session that shows it, by the id of its start. */
  const statsTimers = new Map<string, { id: number; timer: ReturnType<typeof setInterval> }>();
  let lastStatsId = 0;
  const stopStats = (sid: string) => {
    clearInterval(statsTimers.get(sid)?.timer);
    statsTimers.delete(sid);
  };

  return {
    onHostDeleted(hid) {
      // Its forwards go with it, and stop where they run; its tabs stay connected.
      forwards = forwards.filter((f) => f.host_id !== hid);
      const c = connections.get(hid);
      if (c && (c.forwards.size > 0 || c.failed.size > 0)) {
        c.forwards.clear();
        c.failed.clear();
        closeIfUnused(hid);
        connectionsChanged();
      }
    },
    ssh_connect: async (hostId, _options, onFrame: FrameHandler) => {
      const h = state.hosts.find((x) => x.id === hostId);
      if (!h) fail("not_found");
      const sid = id("s");
      onFrame(frame(FRAME_SESSION, new TextEncoder().encode(sid)));
      const stateEvt = (s: EventMap["ssh://state"]["state"], extra: Partial<EventMap["ssh://state"]> = {}) =>
        emit("ssh://state", {
          session_id: sid,
          host_id: hostId,
          state: s,
          latency_ms: null,
          error: null,
          exit_status: null,
          ...extra,
        });
      setTimeout(() => stateEvt("connecting"), 0);
      if (D.FAILING_HOSTS.has(hostId) || sshDemo) {
        const enc = new TextEncoder();
        for (let i = 1; i <= 3; i++) {
          await delay(350);
          onFrame(
            frame(FRAME_DATA, enc.encode(`\x1b[90mConnecting to ${h.address}:${h.port} (attempt ${i})…\x1b[0m\r\n`)),
          );
        }
        await delay(400);
        const kind = sshDemo && sshDemo in SSH_DEMO_DETAIL ? (sshDemo as SshErrorKind) : "timeout";
        const detail = (SSH_DEMO_DETAIL[kind] ?? SSH_DEMO_DETAIL.timeout)(h.address, h.port);
        const error = appError("ssh", detail, { ssh_kind: kind });
        setTimeout(() => stateEvt("failed", { error }), 0);
        throw error;
      }
      const shared = connections.get(hostId);
      if (h.name === "homelab-nas" && !shared) {
        const accepted = await new Promise<boolean>((resolve) => {
          const rid = id("hk");
          pendingHostKeys.set(rid, resolve);
          emit("ssh://hostkey-prompt", {
            request_id: rid,
            session_id: sid,
            host: h.address,
            port: h.port,
            key_type: "ssh-ed25519",
            fingerprint: "SHA256:pX2v8KqLm4Tn7Wc1Rz9Ye3Hb6Ud0Sa5Jf2Gk8Qw",
            kind: "new",
            known_fingerprint: null,
            known_key_type: null,
          });
        });
        if (!accepted) {
          const error = appError("ssh", "host key rejected", { ssh_kind: "host_key_rejected" });
          setTimeout(() => stateEvt("failed", { error }), 0);
          throw error;
        }
      }
      await delay(500);
      const shell = new FakeShell(h, (bytes) => onFrame(frame(FRAME_DATA, bytes)));
      shells.set(sid, shell);
      // SSH-16: the tab joins the host's connection, or opens it.
      const c = shared ?? openConnection(hostId, h.name);
      c.tabs.add(sid);
      connectionsChanged();
      state.hosts = state.hosts.map((x) => (x.id === hostId ? { ...x, last_connected_at: Date.now() } : x));
      setTimeout(() => {
        stateEvt("connected", { latency_ms: 38 });
        shell.start();
      }, 0);
      shell.onExit = () => {
        onFrame(frame(FRAME_CLOSED, new TextEncoder().encode("exit")));
        stateEvt("disconnected", { exit_status: 0 });
        shells.delete(sid);
        leaveConnection(sid);
        stopStats(sid);
      };
      return sid;
    },
    ssh_connect_target: async (target, _options, onFrame: FrameHandler) => {
      needUnlocked();
      const sid = id("s");
      onFrame(frame(FRAME_SESSION, new TextEncoder().encode(sid)));
      const stateEvt = (s: EventMap["ssh://state"]["state"], extra: Partial<EventMap["ssh://state"]> = {}) =>
        emit("ssh://state", {
          session_id: sid,
          host_id: null,
          state: s,
          latency_ms: null,
          error: null,
          exit_status: null,
          ...extra,
        });
      const failWith = (error: AppError): never => {
        setTimeout(() => stateEvt("failed", { error }), 0);
        throw error;
      };
      setTimeout(() => stateEvt("connecting"), 0);
      await delay(400);
      if (target.address.includes("timeout"))
        failWith(appError("ssh", SSH_DEMO_DETAIL.timeout(target.address, target.port), { ssh_kind: "timeout" }));
      const endpoint = `${target.address}:${target.port}`;
      if (!trustedQuickHosts.has(endpoint)) {
        const accepted = await new Promise<boolean>((resolve) => {
          const rid = id("hk");
          pendingHostKeys.set(rid, resolve);
          emit("ssh://hostkey-prompt", {
            request_id: rid,
            session_id: sid,
            host: target.address,
            port: target.port,
            key_type: "ssh-ed25519",
            fingerprint: "SHA256:Qm3cT8vLk2Hs9Xa4Pz7Wd1Nf6Ry0Ub5Ej2Gi8Ko",
            kind: "new",
            known_fingerprint: null,
            known_key_type: null,
          });
        });
        if (!accepted) failWith(appError("ssh", "host key rejected", { ssh_kind: "host_key_rejected" }));
        trustedQuickHosts.add(endpoint);
      }
      const host = target.address.includes(":") ? `[${target.address}]` : target.address;
      if (target.address.endsWith(".ts.net")) {
        // Tailscale SSH's check mode: the server says where to sign in and holds the login until then.
        const rid = id("ab");
        const signedIn = await new Promise<boolean>((resolve) => {
          pendingAuth.set(rid, () => resolve(false));
          emit("ssh://auth-banner", {
            request_id: rid,
            session_id: sid,
            target: `${target.username}@${host}:${target.port}`,
            text: "# Tailscale SSH requires an additional check.\n# To authenticate, visit: https://login.tailscale.com/a/l7c4b9e2f1a3",
          });
          setTimeout(() => resolve(true), 8000);
        });
        if (pendingAuth.delete(rid)) emit("ssh://auth-prompt-closed", { request_id: rid });
        if (!signedIn) failWith(appError("ssh", "operation cancelled", { ssh_kind: "cancelled" }));
      } else {
        // No ssh-agent here: the server asks for the password.
        const answers = await new Promise<string[] | null>((resolve) => {
          const rid = id("ap");
          pendingAuth.set(rid, resolve);
          emit("ssh://auth-prompt", {
            request_id: rid,
            session_id: sid,
            name: target.address,
            instructions: "",
            prompts: [{ prompt: "Password: ", echo: false }],
            password: true,
            sudo: false,
            wrong: false,
            target: `${target.username}@${host}:${target.port}`,
          });
        });
        if (!answers) failWith(appError("ssh", "operation cancelled", { ssh_kind: "cancelled" }));
        if (answers![0] === "wrong")
          failWith(
            appError("ssh", "the server rejected the password (server accepts: publickey, password)", {
              ssh_kind: "auth_failed",
            }),
          );
      }
      await delay(300);
      const shellHost: HostView = {
        ...D.HOSTS[0],
        id: "",
        name: target.address,
        address: target.address,
        port: target.port,
        username: target.username,
      };
      const shell = new FakeShell(shellHost, (bytes) => onFrame(frame(FRAME_DATA, bytes)));
      shells.set(sid, shell);
      recentTargets = [target, ...recentTargets.filter((r) => !sameTarget(r, target))].slice(0, 8);
      setTimeout(() => {
        stateEvt("connected", { latency_ms: 21 });
        shell.start();
      }, 0);
      shell.onExit = () => {
        onFrame(frame(FRAME_CLOSED, new TextEncoder().encode("exit")));
        stateEvt("disconnected", { exit_status: 0 });
        shells.delete(sid);
        stopStats(sid);
      };
      return sid;
    },
    command_history_list: async (owner) => {
      needUnlocked();
      return historyOf(owner);
    },
    command_history_add: async (owner, command) => {
      needUnlocked();
      commandHistory.set(owner, withCommand(historyOf(owner), command));
    },
    command_history_remove: async (owner, command) => {
      needUnlocked();
      commandHistory.set(
        owner,
        historyOf(owner).filter((c) => c !== command),
      );
    },
    command_history_clear: async () => {
      needUnlocked();
      commandHistory.clear();
      historySeeded = false;
    },
    recent_targets_list: async () => {
      needUnlocked();
      return recentTargets.map((r) => ({ ...r }));
    },
    recent_target_remove: async (target) => {
      needUnlocked();
      recentTargets = recentTargets.filter((r) => !sameTarget(r, target));
      return recentTargets.map((r) => ({ ...r }));
    },
    ssh_write: async (sid, data) => {
      shells.get(sid)?.input(data);
    },
    // The sample shell never turns on mouse reporting, so no binary input reaches it.
    ssh_write_bytes: async () => {},
    ssh_resize: async () => {},
    ssh_disconnect: async (sid) => {
      shells.get(sid)?.stop();
      shells.delete(sid);
      leaveConnection(sid);
      stopStats(sid);
    },
    ssh_stats_start: async (sid, onEvent) => {
      const shell = shells.get(sid) ?? fail("not_found", "session");
      const statsId = ++lastStatsId;
      await delay(300);
      if (shell.os === "freebsd" || shell.os === "windows") {
        onEvent({ kind: "unsupported", system: shell.os === "freebsd" ? "FreeBSD" : "Windows" });
        return statsId;
      }
      // Like the backend: a start that a later one overtook does not run.
      if ((statsTimers.get(sid)?.id ?? 0) > statsId) return statsId;
      stopStats(sid);
      const sample = mockStats();
      onEvent({ kind: "stats", stats: sample(true) });
      statsTimers.set(sid, {
        id: statsId,
        timer: setInterval(() => onEvent({ kind: "stats", stats: sample(false) }), 2000),
      });
      return statsId;
    },
    ssh_stats_stop: async (sid, statsId) => {
      if (statsTimers.get(sid)?.id === statsId) stopStats(sid);
    },
    ssh_suggestions: async (sid) => {
      needUnlocked();
      const shell = shells.get(sid) ?? fail("not_found", "session");
      await delay(200);
      if (shell.os === "freebsd" || shell.os === "windows") return null;
      return { history: D.SERVER_HISTORY, commands: D.SERVER_COMMANDS };
    },
    ssh_test: async (input) => {
      await delay(800);
      if (input.address === "172.31.40.8")
        return {
          ok: false,
          latency_ms: null,
          host_key_verified: false,
          error: appError("ssh", "timed out", { ssh_kind: "timeout" }),
        };
      return { ok: true, latency_ms: 38, host_key_verified: true, error: null };
    },
    hostkey_respond: async (rid, accept) => {
      pendingHostKeys.get(rid)?.(accept);
      pendingHostKeys.delete(rid);
    },
    auth_prompt_respond: async (rid, answers) => {
      pendingAuth.get(rid)?.(answers);
      pendingAuth.delete(rid);
    },

    forwards_list_all: async () => [...forwards],
    forward_save: async (input) => {
      const f: ForwardView = {
        ...input,
        id: input.id ?? id("f"),
        name: input.name.trim(),
        bind_address: input.bind_address || (input.kind === "remote" ? "localhost" : "127.0.0.1"),
        ...(input.kind === "dynamic" && { dest_host: "", dest_port: 0 }),
      };
      forwards = input.id ? forwards.map((x) => (x.id === f.id ? f : x)) : [...forwards, f];
      touch();
      // A running forward takes its rule's name.
      const running = connections.get(f.host_id)?.forwards.get(f.id);
      if (running && running.name !== f.name) {
        running.name = f.name;
        connectionsChanged();
      }
      return f;
    },
    forward_delete: async (fid) => {
      const f = forwards.find((x) => x.id === fid);
      forwards = forwards.filter((x) => x.id !== fid);
      touch();
      if (f) stopForward(f.host_id, fid);
    },
    forward_start: async (hostId, fid, options) => {
      needUnlocked();
      const h = state.hosts.find((x) => x.id === hostId) ?? fail("not_found", "host not found");
      const f = forwards.find((x) => x.id === fid && x.host_id === hostId) ?? fail("not_found", "forward not found");
      let c = connections.get(hostId);
      if (!c) {
        if (h.auth_kind === "ask" && !options.password)
          fail("invalid_input", "a password is required for this host", { field: "password" });
        await delay(600);
        c = openConnection(hostId, h.name);
      }
      const port = c.forwards.get(fid)?.port ?? runForward(c, f);
      connectionsChanged();
      return port;
    },
    forward_stop: async (hostId, fid) => stopForward(hostId, fid),
    connections_list: async () => connectionList(),
  };
}

const pendingHostKeys = new Map<string, (accept: boolean) => void>();
const pendingAuth = new Map<string, (answers: string[] | null) => void>();
