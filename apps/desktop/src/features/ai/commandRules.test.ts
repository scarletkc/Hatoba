import { describe, expect, it } from "vitest";
import { allowancePrefixes, commandAllowed, dialectOf, readsOnlyCommand, savablePrefixes, splitCommands } from "./commandRules";

describe("splitCommands", () => {
  it("splits at pipes, lists and line breaks, removing quotes", () => {
    expect(splitCommands(`cat '/etc/my file' | grep -i "a b" && echo x\\ y; ls\nuptime`)?.map((c) => c.words)).toEqual([
      ["cat", "/etc/my file"],
      ["grep", "-i", "a b"],
      ["echo", "x y"],
      ["ls"],
      ["uptime"],
    ]);
    expect(splitCommands("ls |\n  wc -l")?.map((c) => c.words)).toEqual([["ls"], ["wc", "-l"]]);
    expect(splitCommands("ls;")?.map((c) => c.words)).toEqual([["ls"]]);
  });

  it("refuses redirection, substitution, background jobs and anything it cannot follow", () => {
    for (const line of [
      "ls > out",
      "ls >> out",
      "cat < in",
      "cat <<EOF\nx\nEOF",
      "echo $(rm -rf /)",
      "echo `id`",
      "diff <(ls a) <(ls b)",
      "echo $HOME",
      'echo "$HOME"',
      "sleep 9 &",
      "ls |& cat",
      "(cd /tmp; ls)",
      "{ ls; }",
      "ls {a,b}",
      "ls # comment",
      "echo 'open",
      'echo "open',
      "ls &&",
      "| ls",
      "; ls",
      "ls ;; ls",
      "",
      "   ",
      "ls‮",
      "ls\r",
    ]) {
      expect(splitCommands(line), JSON.stringify(line)).toBeNull();
    }
  });

  it("marks unquoted globs", () => {
    expect(splitCommands("ls *.log '*.txt'")?.[0].globs).toEqual([false, true, false]);
  });
});

describe("read-only commands (§13.5)", () => {
  it("runs reading commands without asking", () => {
    for (const line of [
      "ls -la /etc",
      "cat /etc/os-release",
      "head -n 20 /var/log/syslog",
      "tail -n 100 /var/log/nginx/error.log",
      "grep -rn 'listen' /etc/nginx",
      "rg -n TODO src",
      "find /var/log -name '*.gz' -mtime +7",
      "wc -l /etc/passwd",
      "stat /etc/hosts",
      "file /bin/ls",
      "du -sh /var/*",
      "df -h",
      "pwd",
      "whoami",
      "id deploy",
      "uname -a",
      "hostname -I",
      "uptime",
      "date +%F",
      "date -u -d tomorrow",
      "date -Iseconds",
      "which nginx",
      "ps aux",
      "free -m",
      "lsblk",
      "ss -tlnp",
      "netstat -tulpn",
      "ip a",
      "ip -br addr show dev eth0",
      "ip route",
      "ip r get 1.1.1.1",
      "ip -s link",
      "systemctl status nginx --no-pager",
      "systemctl --failed",
      "systemctl is-active nginx",
      "journalctl -u nginx -n 50 --no-pager",
      "journalctl --since today -p err",
      "dmesg -T",
      "git status",
      "git -C /srv/app log --oneline -5",
      "git diff --stat HEAD~1",
      "git show HEAD:README.md",
      "git branch -vv",
      "git branch --list 'feature/*'",
      "git branch -l",
      "git remote -v",
      "git rev-parse HEAD",
      "git ls-files",
      "git blame src/main.rs",
      "git describe --tags",
      "git tag",
      "git tag -l 'v1.*'",
      "git stash list",
      "docker ps -a",
      "docker images",
      "docker logs --tail 100 web",
      "docker container ls",
      "kubectl get pods -n web -o wide",
      "kubectl logs deploy/web --tail=50",
      "cd /etc && cat hostname",
      "ps aux | grep nginx | sort -k3 -nr | head",
      "du -sh * | sort -h",
      "ls /etc || true",
      "echo hi; uptime",
    ]) {
      expect(readsOnlyCommand(line), line).toBe(true);
    }
  });

  it("asks for anything that writes, runs other programs, follows output or is unclear", () => {
    for (const line of [
      "rm -rf /tmp/x",
      "touch a",
      "sudo cat /etc/shadow",
      "LANG=C ls",
      "env rm x",
      "xargs ls",
      "sh -c ls",
      "find . -name x -delete",
      "find . -exec rm {} \\;",
      "find . -execdir rm x ;",
      "find . -ok rm ;",
      "find . -fprint /tmp/out",
      "find . -fls /tmp/out",
      "find . -name *.log",
      "find . '-del'ete",
      "rg --pre sh x",
      // Runs the program it names to look up the host name.
      "rg --hostname-bin=/tmp/helper x",
      "rg --hostname-bin /tmp/helper x",
      "rg x *",
      "tail -f /var/log/syslog",
      "tail -fn 20 x",
      "tail --follow=name x",
      "tail --fol x",
      "journalctl -f",
      "journalctl -fu nginx",
      "journalctl --vacuum-size=100M",
      "journalctl --vac=1",
      "journalctl --rotate",
      // Writes the last entry's cursor to the file.
      "journalctl -n 1 --cursor-file=/tmp/cursor",
      "journalctl -n 1 --cursor-file /tmp/cursor",
      "date -s '2020-01-01'",
      "date 010100002020",
      "date --set=x",
      "hostname newname",
      "hostname -F /etc/hostname",
      "ss -K dst 1.2.3.4",
      "sort -o out in",
      "sort --output=out in",
      "sort *",
      "uniq in out",
      "file -C -m magic",
      "dmesg -c",
      "ip addr add 10.0.0.1/24 dev eth0",
      "ip link set eth0 down",
      "ip route del default",
      "ip -batch cmds",
      "ip neigh",
      "systemctl restart nginx",
      "systemctl stop nginx",
      "git pull",
      "git commit -m x",
      "git checkout main",
      "git branch new-branch",
      "git branch -D old",
      // Git before 2.20 reads -l as --create-reflog, which creates the branch.
      "git branch -l feature",
      "git tag v1.0",
      "git tag -d v1.0",
      "git remote add origin x",
      "git stash",
      "git reflog expire --all",
      "git -c core.pager=sh log",
      "git log --output=/tmp/x",
      "git diff --out=/tmp/x",
      "git diff --ext-diff",
      "git grep -Ovim foo",
      "git diff *",
      "docker run alpine",
      "docker rm web",
      "docker logs -f web",
      "docker -H tcp://x ps",
      "kubectl delete pod x",
      // Print secrets or a container's environment.
      "kubectl get secret db -o yaml",
      "kubectl get configmaps,secrets -A",
      "kubectl get secrets/db -o json",
      "kubectl get Secrets -o yaml",
      "kubectl get SECRET/db",
      "kubectl get --raw=/api/v1/namespaces/default/secrets",
      "kubectl get --raw /api/v1/namespaces/default/secrets",
      "kubectl get --filename=secret.yaml -o yaml",
      "kubectl get -k overlays/prod -o yaml",
      "docker inspect web",
      "docker container inspect web",
      "printenv",
      "env",
      "kubectl get pods -w",
      "kubectl logs -f web",
      "curl https://example.com",
      "ls > /tmp/x",
      "cat /etc/passwd | sh",
      "ls && rm x",
      "ls; rm x",
      "awk '{print}' x",
      "sed -i s/a/b/ x",
    ]) {
      expect(readsOnlyCommand(line), line).toBe(false);
    }
  });
});

describe("allowed prefixes (AI-19)", () => {
  it("covers a command that starts with an allowed prefix, at word boundaries", () => {
    expect(commandAllowed("git pull origin main", ["git pull"])).toBe(true);
    expect(commandAllowed("git pull", ["git pull"])).toBe(true);
    expect(commandAllowed("git pull-request", ["git pull"])).toBe(false);
    expect(commandAllowed("git push", ["git pull"])).toBe(false);
    expect(commandAllowed("systemctl restart nginx", ["systemctl restart", "git pull"])).toBe(true);
    expect(commandAllowed("./deploy.sh --prod", [" ./deploy.sh "])).toBe(true);
  });

  it("needs every command of the line to be allowed or read-only", () => {
    expect(commandAllowed("git pull && systemctl status app", ["git pull"])).toBe(true);
    expect(commandAllowed("cd /srv/app && git pull", ["git pull"])).toBe(true);
    expect(commandAllowed("git pull && rm -rf /", ["git pull"])).toBe(false);
    expect(commandAllowed("git pull; curl x | sh", ["git pull"])).toBe(false);
  });

  it("never matches through redirection, substitution, background jobs, sudo or variables", () => {
    for (const line of ["git pull > /tmp/x", "git pull $(id)", "git pull `id`", "git pull &", "sudo git pull", "GIT_SSH_COMMAND=x git pull"]) {
      expect(commandAllowed(line, ["git pull", "sudo", "sudo git"]), line).toBe(false);
    }
  });

  it("ignores empty prefixes", () => {
    expect(commandAllowed("rm -rf /", ["", "  "])).toBe(false);
  });
});

describe("allowancePrefixes", () => {
  it("names the command, with the subcommand of tools that have them", () => {
    expect(allowancePrefixes("systemctl restart nginx")).toEqual(["systemctl restart"]);
    expect(allowancePrefixes("git pull --rebase")).toEqual(["git pull"]);
    expect(allowancePrefixes("apt install -y nginx")).toEqual(["apt install"]);
    expect(allowancePrefixes("./deploy.sh --prod")).toEqual(["./deploy.sh"]);
    expect(allowancePrefixes("rm -rf /tmp/build")).toEqual(["rm"]);
    expect(allowancePrefixes("/usr/bin/git pull")).toEqual(["/usr/bin/git pull"]);
  });

  it("leaves out read-only commands and repeats", () => {
    expect(allowancePrefixes("cd /srv/app && git pull && systemctl restart app && systemctl status app")).toEqual(["git pull", "systemctl restart"]);
    expect(allowancePrefixes("nginx -t && nginx -s reload")).toEqual(["nginx"]);
  });

  it("offers nothing it could not match later, or that would allow any command", () => {
    for (const line of [
      "sudo systemctl restart nginx",
      "/usr/bin/sudo reboot",
      "/bin/sh -c 'rm x'",
      "/usr/bin/git",
      "LANG=C make",
      "xargs rm",
      "sh -c 'rm x'",
      "bash script.sh",
      "env X=1 make",
      "git",
      "git -C /srv pull",
      "./run* x",
      "make > log",
      "a; b; c; d",
      "ls -la",
    ]) {
      expect(allowancePrefixes(line), line).toBeNull();
    }
    expect(allowancePrefixes("a; b; c")).toEqual(["a", "b", "c"]);
  });

  it("matches what it offers", () => {
    for (const line of ["systemctl restart nginx", "cd /x && make install && ./post.sh", "npm run build -- --prod"]) {
      expect(commandAllowed(line, allowancePrefixes(line)!), line).toBe(true);
    }
  });
});

describe("the shell dialect of a tab (§13.5, §7.5)", () => {
  it("reads PowerShell and Command Prompt tabs in their own syntax, and every other tab as POSIX", () => {
    expect(dialectOf("powershell")).toBe("powershell");
    expect(dialectOf("cmd")).toBe("cmd");
    for (const kind of ["unix", "git_bash", "wsl", null, undefined] as const) expect(dialectOf(kind)).toBe("posix");
  });
});

describe("PowerShell command lines (§13.5)", () => {
  const ps = (line: string) => readsOnlyCommand(line, "powershell");

  it("splits at pipes, lists, statements and line breaks, and arguments at commas, removing quotes", () => {
    expect(
      splitCommands(`Get-ChildItem -Path 'C:\\Program Files' -Recurse | Select-String "a b" && Get-Date; gci\nGet-Process`, "powershell")?.map((c) => c.words),
    ).toEqual([["Get-ChildItem", "-Path", "C:\\Program Files", "-Recurse"], ["Select-String", "a b"], ["Get-Date"], ["gci"], ["Get-Process"]]);
    expect(splitCommands("Select-Object Name,Id , CPU", "powershell")?.[0].words).toEqual(["Select-Object", "Name", "Id", "CPU"]);
    expect(splitCommands(`Write-Output 'it''s' "say ""hi"""`, "powershell")?.[0].words).toEqual(["Write-Output", "it's", 'say "hi"']);
    expect(splitCommands("gci |\n  Measure-Object", "powershell")?.map((c) => c.words)).toEqual([["gci"], ["Measure-Object"]]);
    expect(splitCommands("ls *.log", "powershell")?.[0].globs).toEqual([false, false]);
  });

  it("refuses variables, expressions, redirection and anything else it cannot follow", () => {
    for (const line of [
      "Get-Content $env:USERPROFILE\\.ssh\\config",
      'Get-Content "$HOME\\x"',
      "Write-Output $(Remove-Item x)",
      "Get-ChildItem > out.txt",
      "Get-Process 2>&1",
      "& 'C:\\x.exe'",
      "Start-Sleep 9 &",
      "Get-ChildItem | ForEach-Object { Remove-Item $_ }",
      "(Get-Item x).Delete()",
      "Get-ChildItem @params",
      "@(1,2)",
      "[IO.File]::Delete('x')",
      'Write-Output "a`nb"',
      "Get-ChildItem `\n -Recurse",
      "Get-ChildItem # comment",
      "git --% log",
      "Get-Content ‘x’",
      "Remove-Item –Recurse x",
      "Get-Content 'open",
      'Get-Content "open',
      "gci &&",
      "; gci",
      "",
    ]) {
      expect(splitCommands(line, "powershell"), JSON.stringify(line)).toBeNull();
    }
  });

  it("runs cmdlets and programs that only read, by name or alias, in any case", () => {
    for (const line of [
      "Get-ChildItem -Recurse -Filter *.log",
      "gci C:\\Users",
      "ls",
      "dir /s",
      "Get-Content -Tail 50 C:\\logs\\app.log",
      "cat README.md",
      "type package.json",
      "gc -Raw x.json | ConvertFrom-Json",
      "Select-String -Path *.ts -Pattern TODO",
      "sls TODO src\\main.ts",
      "Get-Process | Sort-Object CPU -Descending | Select-Object -First 10",
      "Get-Process -Name explorer | Format-List *",
      "ps | Where-Object CPU -GT 10",
      "Get-Service | Where-Object Status -EQ Running | Measure-Object",
      "Get-Item C:\\Windows | Get-Member",
      "Test-Path C:\\x",
      "Resolve-Path .",
      "Get-Location",
      "pwd",
      "cd C:\\src; git status",
      "Get-FileHash setup.exe -Algorithm SHA256",
      "Get-Date -Format o",
      "Get-ComputerInfo",
      "Get-CimInstance Win32_OperatingSystem",
      "Get-WinEvent -LogName System -MaxEvents 20",
      "Test-NetConnection example.com -Port 443",
      "tnc 10.0.0.1",
      "Resolve-DnsName example.com -Type MX",
      "Get-NetIPAddress -AddressFamily IPv4",
      "Get-NetTCPConnection -State Listen",
      "echo hi",
      "Write-Host done",
      "GET-CHILDITEM",
      "get-content x",
      "whoami /all",
      "hostname",
      "ipconfig /all",
      "netstat -ano",
      "tasklist /v",
      "systeminfo",
      "route print",
      "arp -a",
      "findstr /s /i todo *.cs",
      "where.exe git",
      "tree /f",
      "git status",
      "git.exe log --oneline -5",
      "GIT diff --stat",
      "rg -n TODO src",
      "docker ps -a",
      "kubectl get pods",
    ]) {
      expect(ps(line), line).toBe(true);
    }
  });

  it("asks for anything that writes, runs other programs, follows output, reaches another computer or is unclear", () => {
    for (const line of [
      "Remove-Item x",
      "rm -Recurse -Force build",
      "Set-Content x.txt hi",
      "Out-File x.txt",
      "New-Item x -ItemType Directory",
      "Copy-Item a b",
      "Stop-Process -Name notepad",
      "Start-Process notepad",
      "Invoke-Expression 'gci'",
      "iex gci",
      "Invoke-WebRequest https://example.com",
      "curl https://example.com",
      "Get-ChildItem | Remove-Item",
      "Get-Process | ForEach-Object Kill",
      "Get-Process | % Kill",
      "Get-Content -Wait app.log",
      "gc app.log -Tail 10 -Wait",
      "Get-Content app.log -wa",
      "Get-Content -Wait:true app.log",
      "Get-Content \\\\server\\share\\x.txt",
      "gci //server/share",
      "Get-Help gci -Online",
      "ipconfig /release",
      "ipconfig /flushdns",
      "netstat 5",
      "arp -d *",
      "route -f print",
      "route add 10.0.0.0 mask 255.0.0.0 10.0.0.1",
      "hostname newname",
      "git pull",
      "git -c core.pager=sh log",
      "C:\\Windows\\System32\\whoami.exe",
      "Get-ChildItem; Remove-Item x",
      "sc stop spooler",
      ". .\\profile.ps1",
      ".\\build.ps1",
      "cmd /c dir",
      "powershell -c gci",
    ]) {
      expect(ps(line), line).toBe(false);
    }
  });
});

describe("Command Prompt command lines (§13.5)", () => {
  const cmd = (line: string) => readsOnlyCommand(line, "cmd");

  it("splits at pipes and lists, and arguments at commas, semicolons and equal signs, removing quotes", () => {
    expect(splitCommands('dir "C:\\Program Files" /s | findstr /i x & ver && type a.txt || echo no', "cmd")?.map((c) => c.words)).toEqual([
      ["dir", "C:\\Program Files", "/s"],
      ["findstr", "/i", "x"],
      ["ver"],
      ["type", "a.txt"],
      ["echo", "no"],
    ]);
    expect(splitCommands("dir a,b;c=d", "cmd")?.[0].words).toEqual(["dir", "a", "b", "c", "d"]);
  });

  it("refuses variables, escapes, redirection, blocks and anything else it cannot follow", () => {
    for (const line of [
      "echo %PATH%",
      'type "%USERPROFILE%\\x"',
      "echo !x!",
      "dir ^& del x",
      "dir > out.txt",
      "type < in",
      "(dir)",
      "@echo off",
      "dir\nver",
      'type "open',
      "dir &",
      "& dir",
      "",
    ]) {
      expect(splitCommands(line, "cmd"), JSON.stringify(line)).toBeNull();
    }
  });

  it("runs Command Prompt's own commands and programs that only read, in any case", () => {
    for (const line of [
      "dir /s /b *.log",
      "DIR",
      "type README.md",
      "echo hello",
      "cd",
      "cd C:\\src",
      "ver",
      "vol C:",
      "date /t",
      "time /T",
      "where git",
      "findstr /n TODO *.c",
      'find /c "x" a.txt',
      "sort names.txt",
      "fc a.txt b.txt",
      "tree /f /a",
      "whoami",
      "ipconfig /displaydns",
      "tasklist | findstr chrome",
      "systeminfo",
      "git status",
      "git log --oneline -5",
      "dir & ver",
    ]) {
      expect(cmd(line), line).toBe(true);
    }
  });

  it("asks for anything that writes or runs other programs, and for cd with other commands", () => {
    for (const line of [
      "del x.txt",
      "rd /s /q build",
      "copy a b",
      "date 2020-01-01",
      "date",
      "time 12:00",
      "sort /o out.txt in.txt",
      "sort /output out.txt in.txt",
      "ipconfig /renew",
      "type \\\\server\\share\\x",
      "start notepad",
      "call x.bat",
      "git pull",
      "cmd /c dir",
      // Command Prompt runs a program of the current folder before one on PATH.
      "cd C:\\src & git status",
      "cd C:\\src && dir",
      "dir/s",
    ]) {
      expect(cmd(line), line).toBe(false);
    }
  });
});

describe("allowed prefixes in PowerShell and Command Prompt (AI-19)", () => {
  it("matches the command's name in any case, and its subcommand as written", () => {
    expect(commandAllowed("npm run build", ["npm run"], "powershell")).toBe(true);
    expect(commandAllowed("NPM run build", ["npm run"], "powershell")).toBe(true);
    expect(commandAllowed("npm RUN build", ["npm run"], "powershell")).toBe(false);
    expect(commandAllowed("Restart-Service spooler", ["restart-service"], "powershell")).toBe(true);
    expect(commandAllowed("net stop spooler", ["net stop"], "cmd")).toBe(true);
    expect(commandAllowed("NPM run build", ["npm run"])).toBe(false);
    expect(commandAllowed("sudo npm run build", ["npm run", "sudo"], "powershell")).toBe(false);
  });

  it("names the command, with the subcommand of tools that have them", () => {
    expect(allowancePrefixes("Restart-Service spooler -Force", "powershell")).toEqual(["Restart-Service"]);
    expect(allowancePrefixes("Get-ChildItem | Remove-Item", "powershell")).toEqual(["Remove-Item"]);
    expect(allowancePrefixes("cd C:\\src; npm run build", "powershell")).toEqual(["npm run"]);
    expect(allowancePrefixes("winget install Git.Git", "powershell")).toEqual(["winget install"]);
    expect(allowancePrefixes("net stop spooler", "cmd")).toEqual(["net stop"]);
    expect(allowancePrefixes("cd C:\\src & msbuild app.sln", "cmd")).toEqual(["cd", "msbuild"]);
  });

  it("offers nothing it could not match later, or that would allow any command", () => {
    for (const [line, dialect] of [
      ["Start-Process notepad", "powershell"],
      ["saps notepad", "powershell"],
      ["Invoke-Expression x", "powershell"],
      ["iex x", "powershell"],
      ["Invoke-Command -ScriptBlock x", "powershell"],
      ["Get-Process | ForEach-Object Kill", "powershell"],
      ["Get-Process | % Kill", "powershell"],
      ["ii setup.exe", "powershell"],
      [". .\\x.ps1", "powershell"],
      ["powershell -c x", "powershell"],
      ["pwsh.exe -c x", "powershell"],
      ["cmd /c x", "cmd"],
      ["start notepad", "cmd"],
      ["call x.bat", "cmd"],
      ["sudo net stop x", "cmd"],
      ["net /?", "cmd"],
      ["Get-Content $x", "powershell"],
      ["echo %x%", "cmd"],
    ] as const) {
      expect(allowancePrefixes(line, dialect), line).toBeNull();
    }
  });

  it("matches what it offers", () => {
    for (const [line, dialect] of [
      ["Restart-Service spooler", "powershell"],
      ["cd C:\\src; npm run build", "powershell"],
      ["cd C:\\src & make install", "cmd"],
      ["net stop spooler && net start spooler", "cmd"],
    ] as const) {
      expect(commandAllowed(line, allowancePrefixes(line, dialect)!, dialect), line).toBe(true);
    }
  });

  it("never saves Windows' interpreters, deleting, permissions, processes, power or downloads for good", () => {
    for (const prefix of [
      "Remove-Item",
      "ri",
      "del",
      "rd",
      "Format-Volume",
      "diskpart",
      "icacls",
      "Stop-Process",
      "taskkill",
      "Restart-Computer",
      "Invoke-WebRequest",
      "iwr",
      "certutil",
      "C:\\Windows\\System32\\shutdown.exe",
      "powershell",
      "cmd.exe",
      "RM",
    ]) {
      expect(savablePrefixes([prefix]), prefix).toBe(false);
    }
    expect(savablePrefixes(["Restart-Service", "npm run", "git pull"])).toBe(true);
  });
});

describe("a run_command call's workdir (AI-12, §13.5)", () => {
  it("counts as a cd before the line", () => {
    expect(readsOnlyCommand("git status", "posix", "/srv/app")).toBe(true);
    expect(readsOnlyCommand("Get-ChildItem", "powershell", "C:/src")).toBe(true);
    expect(commandAllowed("git pull", ["git pull"], "posix", "/srv/app")).toBe(true);
    expect(allowancePrefixes("npm test", "posix", "/srv/app")).toEqual(["npm test"]);
    // Command Prompt runs a program of the folder before one on PATH; PowerShell reaches another computer.
    expect(readsOnlyCommand("dir", "cmd", "C:\\src")).toBe(false);
    for (const share of ["\\\\server\\share", "//server/share", "/\\server\\share", "\\/server/share"]) {
      expect(readsOnlyCommand("Get-ChildItem", "powershell", share), share).toBe(false);
    }
  });

  it("offers its cd for that folder alone", () => {
    expect(allowancePrefixes("msbuild app.sln", "cmd", "C:\\src")).toEqual(["cd C:\\src", "msbuild"]);
    expect(allowancePrefixes("Get-ChildItem", "powershell", "\\\\server\\share")).toEqual(["cd \\\\server\\share"]);
    expect(commandAllowed("git status", ["cd C:\\src"], "cmd", "C:\\src")).toBe(true);
    expect(commandAllowed("git status", ["cd C:\\src"], "cmd", "D:\\other")).toBe(false);
    expect(commandAllowed("msbuild app.sln", allowancePrefixes("msbuild app.sln", "cmd", "C:\\src")!, "cmd", "C:\\src")).toBe(true);
    // A prefix cannot name a folder with a space.
    expect(allowancePrefixes("msbuild app.sln", "cmd", "C:\\My Projects")).toBeNull();
  });

  it("asks when the directory hides characters", () => {
    expect(readsOnlyCommand("ls", "posix", "/srv/\u202eppa")).toBe(false);
    expect(allowancePrefixes("npm test", "posix", "/srv/\u202eppa")).toBeNull();
  });

  it("reads an empty directory as none", () => {
    expect(readsOnlyCommand("dir", "cmd", "")).toBe(true);
  });
});
