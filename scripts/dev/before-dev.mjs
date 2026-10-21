// `beforeDevCommand` of `tauri dev` (apps/desktop/src-tauri/tauri.conf.json): the frontend dev
// server, with its arguments, and on Windows also `hatoba-cli.exe`, the program behind the
// `hatoba` command (HOST-15), which Settings → General copies from beside the app. `tauri dev`
// builds only the app. The two run side by side: Tauri stops waiting for the dev server long
// before a cold Rust build finishes.
import { spawn } from "node:child_process";

let cargo;
if (process.env.TAURI_ENV_PLATFORM === "windows") {
  // The profile and features of the app `tauri dev` builds, so the two share one build of the lib.
  const args = ["build", "-p", "hatoba-desktop", "--bin", "hatoba-cli", "--no-default-features"];
  if (process.env.TAURI_ENV_DEBUG === "false") args.push("--release");
  cargo = spawn("cargo", args, { stdio: "inherit" });
}

// One command line: Node warns about arguments passed beside `shell`, which pnpm needs on Windows.
const vite = spawn(["pnpm", "dev", ...process.argv.slice(2)].join(" "), { stdio: "inherit", shell: true });
vite.on("exit", (code) => {
  // A build left running would hold cargo's lock on the next `tauri dev`.
  cargo?.kill();
  process.exit(code ?? 1);
});
