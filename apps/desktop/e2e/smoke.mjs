// End-to-end smoke test of the real Hatoba app (Rust backend + WebView), driven through
// tauri-driver / WebKitWebDriver. See ./README.md; normally started by ./run-smoke.sh.
//
// Usage: node smoke.mjs <app-binary> <screenshot-dir>
// Env:   HATOBA_E2E_SSH_HOST / _PORT / _USER / _PASSWORD — an SSH server with password auth.
import { mkdirSync, writeFileSync } from "node:fs";

const [BIN, OUT = "e2e-shots"] = process.argv.slice(2);
mkdirSync(OUT, { recursive: true });
const WD = "http://127.0.0.1:4444";
const PASSWORD = "Hatoba-E2E-correct-horse-42";
const env = process.env;
const SSH = {
  address: env.HATOBA_E2E_SSH_HOST ?? "127.0.0.1",
  port: Number(env.HATOBA_E2E_SSH_PORT ?? 2299),
  user: env.HATOBA_E2E_SSH_USER ?? "hatobatest",
  password: env.HATOBA_E2E_SSH_PASSWORD ?? "hatoba-pw-123",
};
let sid;
let step = 0;

async function wd(method, path, body) {
  const res = await fetch(`${WD}${path}`, {
    method,
    headers: { "content-type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  const json = await res.json();
  if (json.value && json.value.error) throw new Error(`${path}: ${json.value.error} ${json.value.message ?? ""}`);
  return json.value;
}
const s = (p) => `/session/${sid}${p}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const EID = "element-6066-11e4-a52e-4f735466cecf";

async function find(xpath, timeout = 15000) {
  const end = Date.now() + timeout;
  for (;;) {
    try {
      const el = await wd("POST", s("/element"), { using: "xpath", value: xpath });
      return el[EID];
    } catch (e) {
      if (Date.now() > end) throw new Error(`not found: ${xpath}`);
      await sleep(250);
    }
  }
}
const btn = (text) => `//button[contains(normalize-space(.), "${text}")]`;
const textEl = (text) => `//*[contains(normalize-space(text()), "${text}")]`;
async function click(xpath, timeout) {
  await wd("POST", s(`/element/${await find(xpath, timeout)}/click`), {});
}
async function type(xpath, text) {
  const id = await find(xpath);
  await wd("POST", s(`/element/${id}/clear`), {});
  await wd("POST", s(`/element/${id}/value`), { text });
}
async function js(script, ...args) {
  return wd("POST", s("/execute/sync"), { script, args });
}
async function shot(name) {
  const png = await wd("GET", s("/screenshot"));
  writeFileSync(`${OUT}/${String(++step).padStart(2, "0")}-${name}.png`, Buffer.from(png, "base64"));
  console.log("  screenshot", name);
}
async function keys(...seq) {
  // seq like ["", "", "l"] → press all, release in reverse
  const down = seq.map((k) => ({ type: "keyDown", value: k }));
  const up = [...seq].reverse().map((k) => ({ type: "keyUp", value: k }));
  await wd("POST", s("/actions"), { actions: [{ type: "key", id: "kbd", actions: [...down, ...up] }] });
}
const CTRL = "", SHIFT = "", ENTER = "";

async function main() {
  const session = await wd("POST", "/session", {
    capabilities: { alwaysMatch: { "tauri:options": { application: BIN } } },
  });
  sid = session.sessionId;
  console.log("session", sid);

  // 1. First launch → create a vault (VAULT-01/02)
  await find(textEl("Create a New Vault"), 30000);
  await shot("onboarding");
  await click(btn("Continue"));
  const pw = await find("(//input[@type='password'])[1]");
  await wd("POST", s(`/element/${pw}/value`), { text: PASSWORD });
  const pw2 = await find("(//input[@type='password'])[2]");
  await wd("POST", s(`/element/${pw2}/value`), { text: PASSWORD });
  await sleep(1200); // strength meter loads lazily
  await click(btn("Create Vault"));
  await find(textEl("Recovery Code"), 30000);
  const groups = await js(`return Array.from(document.querySelectorAll('[role=group] *'))
      .filter(e => e.childElementCount === 0).map(e => e.textContent.trim())
      .filter(t => /^[0-9A-Z]{4}$/.test(t));`);
  console.log("  recovery code groups:", groups.length);
  if (groups.length !== 8) throw new Error("expected 8 recovery groups, got " + JSON.stringify(groups));
  await shot("recovery-code");
  await click(`//*[@role="checkbox"]`);
  await click(btn("Finish Setup"));
  await find(textEl("No hosts yet"), 20000);
  await shot("empty-hosts");

  // 2. New host with password auth (HOST-01, HOST-08)
  await click(btn("New Host"));
  await type(`//*[@id="host-name"]`, "e2e-local");
  await type(`//*[@id="host-address"]`, SSH.address);
  await type(`//*[@id="host-port"]`, String(SSH.port));
  await type(`//*[@id="host-username"]`, SSH.user);
  await click(`//button[@role="radio" and contains(normalize-space(.), "Password")]`);
  await type(`//*[@id="host-password"]`, SSH.password);
  await shot("host-edit");
  await click(btn("Save"));
  await find(`//*[normalize-space(text())="e2e-local"]`, 10000);
  const hostView = await js(`return document.body.innerText.includes("hatobatest@") || document.body.innerText.includes("127.0.0.1")`);
  console.log("  host row rendered:", hostView);
  await shot("host-list");

  // 3. Connect: TOFU host-key prompt (SSH-04), terminal I/O over the binary channel (§10.3)
  const row = await find(`//*[normalize-space(text())="e2e-local"]`);
  await wd("POST", s("/actions"), {
    actions: [{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", origin: { [EID]: row }, x: 0, y: 0 },
      { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
      { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
    ] }],
  });
  await find(textEl("First connection to"), 20000);
  await shot("hostkey-prompt");
  const fp = await js(`return document.body.innerText.match(/SHA256:[A-Za-z0-9+/]+/)?.[0] ?? null`);
  console.log("  fingerprint shown:", fp);
  await click(btn("Trust and Connect"));
  await find(textEl("Connected"), 20000);
  await sleep(1500);
  // type a command into xterm's helper textarea
  const ta = await find(`//textarea[contains(@class, "xterm-helper-textarea")]`);
  await wd("POST", s(`/element/${ta}/value`), { text: "echo e2e-$((6*7)) && printf '\\345\\244\\232\\350\\250\\200\\350\\252\\236\\n'\n" });
  await sleep(1500);
  await shot("terminal");

  // 4. SFTP panel (SFTP-01)
  await click(btn("SFTP"));
  await find(`//*[contains(text(), " item")]`, 15000); // "N items" in the SFTP path bar
  await shot("sftp");

  // 5. Lock and unlock (SEC-02 manual lock, VAULT-03)
  await keys(CTRL, SHIFT, "l");
  await find(textEl("Hatoba is locked"), 10000);
  await shot("locked");
  const unlock = await find(`//input[@type='password']`);
  await wd("POST", s(`/element/${unlock}/value`), { text: "wrong-password" + ENTER });
  await find(textEl("Incorrect master password"), 15000);
  await shot("wrong-password");
  await type(`//input[@type='password']`, PASSWORD + ENTER);
  await find(textEl("All Hosts"), 20000);
  await shot("unlocked");

  console.log("E2E OK");
}

main()
  .catch(async (e) => {
    console.error("E2E FAILED:", e.message);
    try { await shot("failure"); } catch {}
    process.exitCode = 1;
  })
  .finally(async () => {
    if (sid) await wd("DELETE", `/session/${sid}`).catch(() => {});
  });
