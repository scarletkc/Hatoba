import { useEffect } from "react";
import { launchAction } from "@/features/hosts/launchAction";
import { connectHost, connectTarget } from "@/features/terminal/connect";
import { api } from "@/ipc/api";
import { useVaultData } from "./data";
import { useApp } from "./store";
import { useTabs } from "./tabs";

/** Connects what a command line names, or shows it in the search box of All Hosts. */
function runLaunch(args: string[]) {
  const action = launchAction(args, useVaultData.getState().hosts);
  switch (action?.kind) {
    case "host":
      void connectHost(action.hostId);
      break;
    case "target":
      void connectTarget(action.target);
      break;
    case "search":
      useApp.getState().navigate({ kind: "hosts", filter: { kind: "all" } });
      useTabs.getState().activate("home");
      useApp.getState().focusSearch(action.text);
      break;
  }
}

/**
 * Handles `hatoba …` command lines (HOST-13) while the vault is unlocked: the one Hatoba started
 * with, and those later launches hand over. Launches while locked wait in the backend until unlock.
 */
export function useLaunches(unlocked: boolean) {
  useEffect(() => {
    if (!unlocked) return;
    let active = true;
    // The backend hands launches over only while the vault is unlocked, so what this takes runs
    // even if the effect has ended since.
    const take = () => {
      if (!active) return;
      api
        .launch_take()
        .then((launches) => launches.forEach(runLaunch))
        .catch(() => {});
    };
    // Taken once the listener is up, so a launch between the two is not left waiting.
    const sub = api.listen("app://launch", take).then((un) => {
      take();
      return un;
    });
    return () => {
      active = false;
      void sub.then((un) => un());
    };
  }, [unlocked]);
}
