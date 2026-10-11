import { describe, expect, it } from "vitest";
import { allowancePrefixes, commandAllowed, readsOnlyCommand, splitCommands } from "./commandRules";

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
