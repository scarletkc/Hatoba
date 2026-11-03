import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";
import { fileIcon } from "./fileIcon";

const file = (name: string, permissions = "-rw-r--r--") => fileIcon({ name, is_dir: false, permissions }).name;

describe("fileIcon", () => {
  it("keeps the folder and the lock of an owner-only file", () => {
    expect(fileIcon({ name: "src", is_dir: true, permissions: "drwxr-xr-x" })).toEqual({
      name: "folder-simple",
      fill: true,
      color: "var(--accent)",
    });
    expect(file("id_ed25519", "-rw-------")).toBe("file-lock");
    expect(file("server.py", "-rw-------")).toBe("file-lock");
  });

  it("tells the kinds apart by extension, in any case", () => {
    expect(file("main.rs")).toBe("file-code");
    expect(file("App.TSX")).toBe("file-code");
    expect(file("deploy.sh")).toBe("file-code");
    expect(file("Cargo.toml")).toBe("file-code");
    expect(file("docker-compose.yml")).toBe("file-code");
    expect(file("backup.tar.gz")).toBe("file-archive");
    expect(file("logo.PNG")).toBe("file-image");
    expect(file("manual.pdf")).toBe("file-pdf");
    expect(file("dump.sql")).toBe("file-sql");
    expect(file("app.sqlite3")).toBe("file-sql");
    expect(file("report.docx")).toBe("file-doc");
    expect(file("users.csv")).toBe("file-csv");
    expect(file("fullchain.pem")).toBe("file-text");
    expect(file("store.p12")).toBe("file-lock");
    expect(file("id_ed25519.pub")).toBe("file-text");
    expect(file("notes.md")).toBe("file-text");
    expect(file("access.log")).toBe("file-text");
  });

  it("knows files by name", () => {
    expect(file("Dockerfile")).toBe("file-code");
    expect(file("Makefile")).toBe("file-code");
    expect(file("README")).toBe("file-text");
    expect(file("LICENSE")).toBe("file-text");
    expect(file("syslog.1")).toBe("file-text");
    expect(file(".bashrc")).toBe("file-code");
    expect(file(".profile")).toBe("file-code");
    expect(file(".gitignore")).toBe("file-code");
    expect(file("authorized_keys")).toBe("file-text");
    expect(file("id_rsa", "-rw-r--r--")).toBe("file-lock");
  });

  it("shows anything else as a blank page, executables too", () => {
    expect(file("node", "-rwxr-xr-x")).toBe("file");
    expect(file("notes.txt", "-rwxrwxrwx")).toBe("file-text");
    expect(file("hosts")).toBe("file");
    expect(file(".bash_history")).toBe("file");
    expect(file("data.bin")).toBe("file");
  });

  it("uses only glyphs the bundled Phosphor font has", () => {
    const require = createRequire(import.meta.url);
    const css = readFileSync(
      require.resolve("@phosphor-icons/web/regular", { paths: [require.resolve("@hatoba/ui")] }),
      "utf8",
    );
    const names = [
      "folder-simple",
      ...[
        "x.rs",
        "x.sh",
        "x.toml",
        "x.zip",
        "x.png",
        "x.mp4",
        "x.mp3",
        "x.pdf",
        "x.sql",
        "x.doc",
        "x.xls",
        "x.csv",
        "x.ppt",
        "x.pem",
        "x.pub",
        "x.txt",
        "x",
      ].map((n) => file(n)),
      file("x", "-rw-------"),
      file("x", "-rwxr-xr-x"),
    ];
    for (const name of names) expect(css, name).toContain(`.ph.ph-${name}:before`);
  });
});
