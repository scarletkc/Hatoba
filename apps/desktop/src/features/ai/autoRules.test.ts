import { describe, expect, it } from "vitest";
import type { McpToolInfo } from "@/ipc/types";
import { hardRule, namedCommand, passwordPrompt } from "./autoRules";

const command = (line: string) => hardRule({ kind: "run_command", args: { command: line } });

describe("the commands Auto always asks about (AI-47)", () => {
  it("asks for recursive rm, with any recursive option", () => {
    for (const line of ["rm -r /tmp/x", "rm -rf /var/lib/mysql", "rm -fR build", "rm -Rf build", "rm -v -r logs", "rm --recursive old", "rm --rec old", "rm -f -- -r"])
      expect(command(line), line).toEqual({ kind: "command", command: "rm -r" });
  });

  it("leaves rm without a recursive option to the judge", () => {
    for (const line of ["rm /etc/motd", "rm -f /tmp/a.lock", "rm -i old.log", "rm --force x", "rm -- -file"]) expect(command(line), line).toBeNull();
  });

  it("finds a command nested in quotes, sh -c, eval, xargs, ssh, substitutions and groups", () => {
    const nested = [
      "sh -c 'rm -rf /tmp/x'",
      'bash -c "cd /srv && rm -rf build"',
      "eval rm -rf /tmp/x",
      "find /tmp -name '*.log' | xargs rm -rf",
      "ssh db1 'sudo rm -r /var/backups/old'",
      "echo $(rm -rf /tmp/x)",
      "( cd /tmp; rm -rf x )",
      "{rm,-rf,/tmp/x}",
      "'r'm -rf /tmp/x",
      "r\\m -rf /tmp/x",
      'rm${IFS}-rf${IFS}/tmp/x',
      "true&&rm -rf x",
    ];
    for (const line of nested) expect(command(line), line).toEqual({ kind: "command", command: "rm -r" });
  });

  it("finds a command named with a directory, in any case, and mkfs with a suffix", () => {
    expect(command("/bin/rm -r /tmp/x")).toEqual({ kind: "command", command: "rm -r" });
    expect(command("sudo /usr/bin/RM -R x")).toEqual({ kind: "command", command: "rm -r" });
    expect(command("sudo /sbin/reboot")).toEqual({ kind: "command", command: "reboot" });
    expect(command("mkfs.ext4 /dev/sdb1")).toEqual({ kind: "command", command: "mkfs.ext4" });
    expect(command("sudo mkfs -t xfs /dev/sdc")).toEqual({ kind: "command", command: "mkfs" });
  });

  it("asks for each command of the list", () => {
    const lines: [string, string][] = [
      ["shred -u secrets.txt", "shred"],
      ["dd if=/dev/zero of=/dev/sda bs=1M", "dd"],
      ["wipefs -a /dev/sdb", "wipefs"],
      ["fdisk /dev/sdb", "fdisk"],
      ["sfdisk -d /dev/sda", "sfdisk"],
      ["parted /dev/sdb mklabel gpt", "parted"],
      ["sudo shutdown -h now", "shutdown"],
      ["sudo systemctl reboot", "reboot"],
      ["poweroff", "poweroff"],
      ["halt -p", "halt"],
    ];
    for (const [line, name] of lines) expect(command(line), line).toEqual({ kind: "command", command: name });
  });

  it("asks when a line only mentions one, since a false alarm costs a click", () => {
    expect(command("man shutdown")).toEqual({ kind: "command", command: "shutdown" });
    expect(command("grep -r halt /var/log/syslog")).toEqual({ kind: "command", command: "halt" });
    expect(command("alias off=poweroff")).toEqual({ kind: "command", command: "poweroff" });
    // The recursive option may belong to a later command of the line.
    expect(command("rm a.lock; ls -R /srv")).toEqual({ kind: "command", command: "rm -r" });
  });

  it("leaves ordinary commands, sudo included, to the judge", () => {
    const ordinary = [
      "df -h | sort -h",
      "sudo systemctl restart nginx",
      "journalctl -u nginx -n 50 --no-pager",
      "git add -A && git commit -m 'fix'",
      "cat /var/log/dd.log",
      "systemctl status shutdown.target",
      "ls -R /srv",
      "nginx -t 2>/dev/null",
      "awk '{print $1}' access.log | sort | uniq -c",
      "curl -fsS http://localhost/healthz",
    ];
    for (const line of ordinary) expect(command(line), line).toBeNull();
  });

  it("checks the text send_input types", () => {
    expect(hardRule({ kind: "send_input", args: { text: "sudo reboot", key: "enter" } })).toEqual({ kind: "command", command: "reboot" });
    expect(hardRule({ kind: "send_input", args: { text: "sudo reboot" }, screen: "kc@db1:~$" })).toEqual({ kind: "command", command: "reboot" });
    expect(hardRule({ kind: "send_input", args: { text: "y", key: "enter" } })).toBeNull();
    expect(namedCommand("")).toBeNull();
  });
});

describe("the commands Auto always asks about in PowerShell and Command Prompt (AI-47)", () => {
  const ps = (line: string) => hardRule({ kind: "run_command", args: { command: line }, dialect: "powershell" });
  const cmd = (line: string) => hardRule({ kind: "run_command", args: { command: line }, dialect: "cmd" });

  it("asks for deleting recursively, with the option before or after the command", () => {
    const lines: [string, string][] = [
      ["Remove-Item -Recurse -Force C:\\build", "Remove-Item -Recurse"],
      ["remove-item C:\\build -r", "remove-item -r"],
      ["ri build -Rec", "ri -Rec"],
      ["rm -Recurse:true build", "rm -Recurse:true"],
      ["Get-ChildItem C:\\data -Recurse | Remove-Item -Force", "Remove-Item -Recurse"],
      ["Remove-Item –Recurse build", "Remove-Item -Recurse"],
      ["Remove-Item ‘build’ -Recurse", "Remove-Item -Recurse"],
      ["Re`move-Item build -Recurse", "Remove-Item -Recurse"],
      ["rm -rf build", "rm -rf"],
    ];
    for (const [line, command] of lines) expect(ps(line), line).toEqual({ kind: "command", command });
    const cmdLines: [string, string][] = [
      ["rd /s /q C:\\build", "rd /s"],
      ["rmdir /S build", "rmdir /S"],
      ["del /s /q *.tmp", "del /s"],
      ["erase /q /s *.log", "erase /s"],
      ["rd/s/q build", "rd /s"],
      ["dir & del/s/q x", "del /s"],
      ['"rd" /s build', "rd /s"],
      ["r^d /s build", "rd /s"],
    ];
    for (const [line, command] of cmdLines) expect(cmd(line), line).toEqual({ kind: "command", command });
  });

  it("leaves deleting without a recursive option to the judge", () => {
    for (const line of ["Remove-Item x.txt", "rm -Force x.txt", "del -Filter *.tmp", "Get-ChildItem -Recurse"]) expect(ps(line), line).toBeNull();
    for (const line of ["del x.txt", "rd empty", "del /q /f x.txt", "dir /s"]) expect(cmd(line), line).toBeNull();
  });

  it("asks for formatting and wiping disks and stopping the computer, named in any case or with a directory", () => {
    const lines: [string, string][] = [
      ["format D: /q", "format"],
      ["FORMAT d:", "FORMAT"],
      ["format.com E: /fs:ntfs", "format.com"],
      ["diskpart /s script.txt", "diskpart"],
      ["Format-Volume -DriveLetter D", "Format-Volume"],
      ["Get-Disk 1 | Clear-Disk -RemoveData", "Clear-Disk"],
      ["Remove-Partition -DiskNumber 1 -PartitionNumber 2", "Remove-Partition"],
      ["Stop-Computer -Force", "Stop-Computer"],
      ["Restart-Computer", "Restart-Computer"],
      ["shutdown /s /t 0", "shutdown"],
      ["shutdown/r", "shutdown"],
      ["C:\\Windows\\System32\\shutdown.exe /r", "shutdown.exe"],
      ["& 'C:\\Windows\\System32\\shutdown.exe' -s", "shutdown.exe"],
      ["C:/Windows/System32/shutdown.exe /s", "shutdown.exe"],
      ["Start-Process diskpart.exe", "diskpart.exe"],
      ["cmd /c 'shutdown /s'", "shutdown"],
      ["wsl -- sudo reboot", "reboot"],
    ];
    for (const [line, command] of lines) {
      expect(ps(line), line).toEqual({ kind: "command", command });
      expect(cmd(line.replaceAll("'", '"')), line).toEqual({ kind: "command", command });
    }
  });

  it("leaves ordinary commands to the judge", () => {
    for (const line of [
      "Get-Date -Format yyyy-MM-dd",
      "Format-Table Name, Id",
      "git log --format=%H",
      "npm run format",
      "dotnet format",
      "pnpm format && pnpm lint",
      "Get-Content C:\\data\\notes\\formatting.txt",
      "Restart-Service spooler",
      "Stop-Process -Name notepad",
      "Copy-Item -Recurse a b",
    ]) {
      expect(ps(line), line).toBeNull();
    }
  });

  it("keeps SSH and POSIX tabs to their own rules", () => {
    expect(command("format D:")).toBeNull();
    expect(command("del /s x")).toBeNull();
    expect(command("Remove-Item -Recurse x")).toBeNull();
  });

  it("checks the line send_input submits on a PowerShell tab", () => {
    const typed = (cursor: string, args: Record<string, unknown>) =>
      hardRule({ kind: "send_input", args, screen: `PS C:\\Users\\kc> Get-Date\n\n${cursor}`, dialect: "powershell" });
    expect(typed("PS C:\\Users\\kc>", { text: "Stop-Computer\n" })).toEqual({ kind: "command", command: "Stop-Computer" });
    expect(typed("PS C:\\Users\\kc> Remove-Item C:\\data", { text: " -Recurse", key: "enter" })).toEqual({ kind: "command", command: "Remove-Item -Recurse" });
    expect(typed("PS C:\\Users\\kc>", { text: "Get-ChildItem\n" })).toBeNull();
  });

  it("keeps MCP calls to the POSIX rules", () => {
    expect(hardRule({ kind: "mcp", args: { command: "rm -rf /" }, mcp: { always_ask: false } as McpToolInfo, dialect: "powershell" })).toEqual({
      kind: "command",
      command: "rm -r",
    });
  });
});

describe("a line send_input submits (AI-47)", () => {
  const typed = (cursor: string, args: Record<string, unknown>) => hardRule({ kind: "send_input", args, screen: `$ uptime\n 10:02:11 up 3 days\n${cursor}` });

  it("checks the cursor's line with the text, so that a command split across calls asks", () => {
    // `rm ` typed by one call, which the judge decides, and `-rf /data` entered by the next. The screen keeps
    // no trailing spaces, so the cursor's line ends in `rm`.
    expect(typed("kc@db1:~$", { text: "rm " })).toBeNull();
    expect(typed("kc@db1:~$ rm", { text: "-rf /data\n" })).toEqual({ kind: "command", command: "rm -r" });
    expect(typed("kc@db1:~$ rm", { text: "-rf /data", key: "enter" })).toEqual({ kind: "command", command: "rm -r" });
    // A word split in two.
    expect(typed("kc@db1:~$ sudo shut", { text: "down -h now\n" })).toEqual({ kind: "command", command: "shutdown" });
    // Until it submits, only the text counts.
    expect(typed("kc@db1:~$ rm", { text: "-rf /data" })).toBeNull();
  });

  it("checks a line Tab completed when Enter submits it", () => {
    expect(typed("kc@db1:~$ sudo shut", { text: "", key: "tab" })).toBeNull();
    expect(typed("kc@db1:~$ sudo shutdown", { key: "enter" })).toEqual({ kind: "command", command: "shutdown" });
  });

  it("reads past the prompt", () => {
    const prompts = ["kc@db1:~$", "root@db1:~#", "[root@db1 ~]#", "kc@mac ~ %", "db1>", "PS C:\\Users\\kc>", "(venv) kc@db1:/srv/app$", "➜  app git:(main)"];
    for (const prompt of prompts) {
      expect(typed(`${prompt} rm`, { text: "-r old\n" }), prompt).toEqual({ kind: "command", command: "rm -r" });
      expect(typed(prompt, { text: "sudo reboot\n" }), prompt).toEqual({ kind: "command", command: "reboot" });
      expect(typed(`${prompt} ls`, { text: " -la /srv\n" }), prompt).toBeNull();
    }
    // A prompt without a trailing space, right before what was typed.
    expect(typed("root@db1:~#rm", { text: " -rf /data\n" })).toEqual({ kind: "command", command: "rm -r" });
    expect(typed("kc@mac%halt", { key: "enter" })).toEqual({ kind: "command", command: "halt" });
  });

  it("asks when the cursor's line only mentions one, since a false alarm costs a click", () => {
    // A question about one, which the answer confirms.
    expect(typed("Proceed with shutdown of web2? [y/N]", { text: "y", key: "enter" })).toEqual({ kind: "command", command: "shutdown" });
    // A working directory named like one.
    expect(typed("deploy@web1:~/backups/dd$", { text: "ls\n" })).toEqual({ kind: "command", command: "dd" });
    expect(typed("Do you want to continue? [Y/n]", { text: "y", key: "enter" })).toBeNull();
  });

  it("checks the cursor's line for hidden characters", () => {
    expect(typed("kc@db1:~$ ls\u200b", { key: "enter" })).toEqual({ kind: "hidden" });
    // Until it submits, only the text counts.
    expect(typed("kc@db1:~$ ls\u200b", { text: " -la" })).toBeNull();
  });
});

describe("hidden characters (AI-47)", () => {
  it("send a command or typed text with hidden or bidirectional characters to the card", () => {
    expect(command("echo ‮/ fr- mr")).toEqual({ kind: "hidden" });
    expect(command("ls​ -la")).toEqual({ kind: "hidden" });
    expect(command("printf '\x1b[2J'")).toEqual({ kind: "hidden" });
    expect(hardRule({ kind: "send_input", args: { text: "y\r" } })).toEqual({ kind: "hidden" });
    // Line breaks and tabs are visible on the card.
    expect(command("cd /srv\tmake\nmake install")).toBeNull();
  });
});

describe("password prompts (AI-47)", () => {
  it("recognize a prompt for a password or passphrase in the screen's last line", () => {
    for (const line of [
      "Password:",
      "[sudo] password for kc:",
      "kc@db1's password: ",
      "Enter passphrase for key '/home/kc/.ssh/id_ed25519':",
      "Enter same passphrase again:",
      "New password:",
      "Password for 'https://kc@github.com':",
      "密码：",
      "[sudo] kc 的密码：",
      "パスワード:",
      "パスフレーズを入力してください：",
    ])
      expect(passwordPrompt(line), line).toBe(true);
  });

  it("leave other lines alone", () => {
    for (const line of ["kc@db1:~$", "Do you want to continue? [Y/n]", "Password changed.", "grep password config.yml", "Enter file in which to save the key (/home/kc/.ssh/id_ed25519):"])
      expect(passwordPrompt(line), line).toBe(false);
  });

  it("send send_input to the card while the screen asks for a password", () => {
    const screen = "$ sudo systemctl restart nginx\n[sudo] password for kc:";
    expect(hardRule({ kind: "send_input", args: { text: "hunter2", key: "enter" }, screen })).toEqual({ kind: "password" });
    expect(hardRule({ kind: "send_input", args: { key: "enter" }, screen })).toEqual({ kind: "password" });
    // Only the last line counts: the prompt was answered.
    expect(hardRule({ kind: "send_input", args: { text: "y" }, screen: `${screen}\nRestart nginx? [y/N]` })).toBeNull();
    // run_command does not type into the screen.
    expect(hardRule({ kind: "run_command", args: { command: "uptime" }, screen })).toBeNull();
  });
});

describe("MCP servers set to Always ask, and tools that cannot be found (AI-47)", () => {
  const info = (always_ask: boolean): McpToolInfo => ({
    server_id: "s1",
    server_name: "github",
    always_ask,
    tool: {
      name: "mcp__github__create_issue",
      tool: "create_issue",
      description: "",
      annotations: { title: null, read_only_hint: null, destructive_hint: null, idempotent_hint: null, open_world_hint: null },
      always_allow: false,
    },
  });

  it("send those tools to the card, and leave the others to the judge", () => {
    expect(hardRule({ kind: "mcp", args: {}, mcp: info(true) })).toEqual({ kind: "mcp_always_ask" });
    expect(hardRule({ kind: "mcp", args: { repo: "scarletkc/infra", title: "staging disk full" }, mcp: info(false) })).toBeNull();
    expect(hardRule({ kind: "mcp", args: {}, mcp: null })).toEqual({ kind: "mcp_unknown" });
    // Those come first.
    expect(hardRule({ kind: "mcp", args: { command: "rm -rf /" }, mcp: info(true) })).toEqual({ kind: "mcp_always_ask" });
    expect(hardRule({ kind: "mcp", args: { command: "rm -rf /" }, mcp: null })).toEqual({ kind: "mcp_unknown" });
  });

  it("check every string of an MCP call's arguments for the commands, nested ones too", () => {
    const mcp = (args: Record<string, unknown>) => hardRule({ kind: "mcp", args, mcp: info(false) });
    expect(mcp({ command: "rm -rf /srv/old" })).toEqual({ kind: "command", command: "rm -r" });
    expect(mcp({ steps: [{ run: { argv: ["sudo", "shutdown", "-r", "now"] } }] })).toEqual({ kind: "command", command: "shutdown" });
    // A command given as a list of words.
    expect(mcp({ command: "rm", args: ["-rf", "/srv/old"] })).toEqual({ kind: "command", command: "rm -r" });
    expect(mcp({ script: "#!/bin/sh\nset -e\nmkfs.ext4 /dev/sdb1\n" })).toEqual({ kind: "command", command: "mkfs.ext4" });
    // A false alarm costs a click.
    expect(mcp({ title: "staging disk full", body: "Do not reboot web1 before Friday." })).toEqual({ kind: "command", command: "reboot" });
    // Keys are not commands, and numbers and booleans hold none.
    expect(mcp({ reboot: false, count: 3, tags: ["ops"], nested: { dd: null } })).toBeNull();
  });

  it("send an MCP call with hidden or bidirectional characters to the card, but allow tabs and line breaks", () => {
    const mcp = (args: Record<string, unknown>) => hardRule({ kind: "mcp", args, mcp: info(false) });
    expect(mcp({ body: "Steps:\r\n\t1. Check the disk\n\t2. Restart nginx\r\n" })).toBeNull();
    expect(mcp({ files: [{ path: "Makefile", content: "build:\n\tgo build ./...\r\n" }] })).toBeNull();
    // The joiners of emoji and of Persian text.
    expect(mcp({ body: "Fixed by 👨‍💻 and می‌خواهم" })).toBeNull();
    expect(mcp({ title: "invoice\u202efdp.exe" })).toEqual({ kind: "hidden" });
    expect(mcp({ items: [{ text: "ok\x1b[2J" }] })).toEqual({ kind: "hidden" });
    expect(mcp({ text: "ls\u200b -la" })).toEqual({ kind: "hidden" });
    // In a key too.
    expect(mcp({ "path\u202e": "a" })).toEqual({ kind: "hidden" });
  });

  it("apply no rule to the file tools and fetch_url", () => {
    expect(hardRule({ kind: "edit_file", args: { path: "/etc/sudoers", new_string: "rm -rf /" } })).toBeNull();
    expect(hardRule({ kind: "fetch_url", args: { url: "https://example.com/?q=reboot" } })).toBeNull();
  });
});
