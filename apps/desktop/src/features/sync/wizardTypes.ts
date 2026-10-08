import type { AppError, D1Database, DeployPlan, DeployStart, DeployStep, SyncTestResult } from "@/ipc/types";

/** Deploy the Worker from the app (§6.7), connect a deployed Worker, or D1 direct mode (§6.1). */
export type Method = "deploy" | "worker" | "d1";

/** Progress of a connection test or token verification. */
export type Probe =
  | { status: "idle" }
  | { status: "testing" }
  | { status: "ok"; result: SyncTestResult }
  | { status: "failed"; message: string; offline?: boolean };

export interface WorkerForm {
  url: string;
  token: string;
  test: Probe;
}

export interface D1Form {
  accountId: string;
  apiToken: string;
  databaseId: string;
  databases: D1Database[];
  /** Token verification. */
  verify: Probe;
  /** The chosen database already holds a vault (flow C is not in the MVP). */
  initialized: boolean;
  /** Checking the chosen database for an existing vault. */
  checking: boolean;
}

export const EMPTY_WORKER: WorkerForm = { url: "", token: "", test: { status: "idle" } };
export const EMPTY_D1: D1Form = {
  accountId: "",
  apiToken: "",
  databaseId: "",
  databases: [],
  verify: { status: "idle" },
  initialized: false,
  checking: false,
};

/** In-app deployment (spec §6.7), kept by the wizard so going back and forth keeps its progress. */
export interface DeployForm {
  phase: "token" | "review" | "run";
  /** Cleared as soon as Rust accepts it (DEPLOY-07). */
  apiToken: string;
  accountId: string;
  /** Step 1 passed: the deployment's handle and what the token reaches. */
  start: DeployStart | null;
  verifying: boolean;
  tokenError: AppError | null;
  workerName: string;
  databaseName: string;
  subdomain: string;
  namesOpen: boolean;
  plan: DeployPlan | null;
  inspecting: boolean;
  planError: AppError | null;
  steps: Partial<Record<DeployStep, StepState>>;
  run: "idle" | "running" | "failed" | "waiting" | "ready" | "removing" | "removed";
  runError: AppError | null;
  url: string | null;
}

export type StepState = "running" | "done" | "skipped" | "failed";

/** Steps 1 to 8, in the order the spec numbers them. */
export const DEPLOY_STEPS: DeployStep[] = ["verify", "inspect", "create_database", "migrate", "upload", "setup_token", "route", "wait"];

export const EMPTY_DEPLOY: DeployForm = {
  phase: "token",
  apiToken: "",
  accountId: "",
  start: null,
  verifying: false,
  tokenError: null,
  workerName: "",
  databaseName: "",
  subdomain: "",
  namesOpen: false,
  plan: null,
  inspecting: false,
  planError: null,
  steps: {},
  run: "idle",
  runError: null,
  url: null,
};

/** "hatoba-sync.me.workers.dev/" → "https://hatoba-sync.me.workers.dev" */
export function normalizeWorkerUrl(raw: string): string {
  const trimmed = raw.trim().replace(/\/+$/, "");
  if (!trimmed) return "";
  return /^https?:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
}
