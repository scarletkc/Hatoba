import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { api } from "@/ipc/api";
import type { SyncConfigInput } from "@/ipc/types";
import { D1Step } from "./D1Step";
import { DeployStep } from "./DeployStep";
import { MethodStep } from "./MethodStep";
import { PasswordStep } from "./PasswordStep";
import { EMPTY_D1, EMPTY_DEPLOY, EMPTY_WORKER, type D1Form, type Method, type WorkerForm } from "./wizardTypes";
import { WorkerStep } from "./WorkerStep";

/** What step 3 initialises: a Worker or D1 database the user connected, or the Worker the app deployed. */
type Remote = { kind: "config"; config: SyncConfigInput } | { kind: "deployed"; handle: string };

/** Cloud Sync setup (design §05): 接入方式 → 连接 → 主密码. Mounted by the sync page while sync is off. */
export function SyncWizard() {
  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [method, setMethod] = useState<Method>("deploy");
  const [worker, setWorker] = useState<WorkerForm>(EMPTY_WORKER);
  const [d1, setD1] = useState<D1Form>(EMPTY_D1);
  const [deploy, setDeploy] = useState(EMPTY_DEPLOY);
  const [remote, setRemote] = useState<Remote | null>(null);

  // A deployment's tokens live in Rust until the wizard goes away (DEPLOY-07).
  const handle = useRef<string | null>(null);
  handle.current = deploy.start?.handle ?? null;
  useEffect(
    () => () => {
      if (handle.current) void api.deploy_cancel(handle.current);
    },
    [],
  );

  const patch = <F,>(set: Dispatch<SetStateAction<F>>) => (p: Partial<F>) => set((f) => ({ ...f, ...p }));

  if (step === 3 && remote) {
    const submit =
      remote.kind === "config" ? (pw: string) => api.sync_configure(remote.config, pw) : (pw: string) => api.deploy_setup(remote.handle, pw);
    return <PasswordStep submit={submit} onBack={() => setStep(2)} />;
  }
  if (step === 2) {
    if (method === "d1") {
      const next = (config: SyncConfigInput) => {
        setRemote({ kind: "config", config });
        setStep(3);
      };
      return <D1Step form={d1} update={patch(setD1)} onBack={() => setStep(1)} onNext={next} />;
    }
    if (method === "deploy")
      return (
        <DeployStep
          form={deploy}
          set={setDeploy}
          onBack={() => setStep(1)}
          onReady={(h) => {
            setRemote({ kind: "deployed", handle: h });
            setStep(3);
          }}
        />
      );
    return (
      <WorkerStep
        form={worker}
        update={patch(setWorker)}
        onBack={() => setStep(1)}
        onNext={(config) => {
          setRemote({ kind: "config", config });
          setStep(3);
        }}
      />
    );
  }
  return <MethodStep method={method} onChange={setMethod} onNext={() => setStep(2)} />;
}
