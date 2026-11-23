const { spawn } = require("child_process");
const fs = require("fs");
const path = require("path");
const assert = require("assert");
const { ZmodemTerminal } = require(process.argv[2] || "./controller.cjs");
async function test(direction, names) {
  const root = fs.mkdtempSync("/tmp/hatoba-zmodem-");
  const payload = Buffer.from(Array.from({ length: 262144 }, (_, i) => i % 256));
  const expected = new Map(names.map((name, i) => [name, i === 1 ? Buffer.alloc(0) : payload]));
  if (direction === "download") for (const [name, data] of expected) fs.writeFileSync(path.join(root, name), data);
  const child = spawn(
    direction === "download" ? "sz" : "rz",
    direction === "download" ? ["-b", ...names] : ["-b", "-y"],
    { cwd: root },
  );
  let log = "";
  child.stderr.on("data", (d) => (log += d.toString()));
  let active = false;
  const actual = new Map();
  let offers = 0;
  let ended;
  const result = new Promise((r, j) => {
    ended = r;
    child.on("error", j);
    child.on("exit", (c) => (c !== 0 ? j(Error("lrzsz exit " + c + " " + log)) : r()));
  });
  const controller = new ZmodemTerminal({
    allowed: () => true,
    output: () => {},
    send: (bytes) => new Promise((r, j) => child.stdin.write(Buffer.from(bytes), (e) => (e ? j(e) : r()))),
    view: (v) => {
      if (!v) return;
      if (v.phase === "upload" && !active) {
        active = true;
        queueMicrotask(() =>
          controller.upload(
            [...expected].map(([name, data]) => ({
              name,
              size: data.length,
              slice: (a, b) => ({ arrayBuffer: async () => Uint8Array.from(data.subarray(a, b)).buffer }),
            })),
          ),
        );
      }
      if (v.phase === "offer") {
        offers++;
        queueMicrotask(() => controller.receive());
      }
    },
    begin: async (name) => {
      actual.set(name, []);
      return name;
    },
    write: async (id, data) => actual.get(id).push(Buffer.from(data)),
    finish: async (id, commit) => (commit ? id : null),
    saved: () => {},
    error: (e) => {
      console.error(e);
      child.kill();
    },
  });
  child.stdout.on("data", (d) => controller.consume(new Uint8Array(d)));
  const timer = setTimeout(() => {
    console.error("timeout", log);
    controller.cancel();
    child.kill();
  }, 15000);
  try {
    await result;
    await new Promise((r) => setTimeout(r, 200));
    for (const [name, data] of expected)
      assert.deepStrictEqual(
        direction === "download" ? Buffer.concat(actual.get(name)) : fs.readFileSync(path.join(root, name)),
        data,
      );
    console.log(direction + " PASS: binary + empty + Unicode, " + names.length + " files");
  } finally {
    clearTimeout(timer);
    controller.cancel();
    fs.rmSync(root, { recursive: true, force: true });
  }
}
(async () => {
  await test("download", ["binary.bin", "empty.txt", "中文.bin"]);
  await test("upload", ["binary.bin", "empty.txt", "中文.bin"]);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
