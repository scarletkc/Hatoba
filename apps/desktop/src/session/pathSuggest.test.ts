import { beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { appError } from "@/ipc/api";
import { PathLister, pathRest, pathWord } from "./pathSuggest";

const backend = vi.hoisted(() => ({ home: vi.fn(), list: vi.fn() }));

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: { sftp_home: backend.home, sftp_list: backend.list },
}));

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const file = (name: string) => ({ name, is_dir: false });
const dir = (name: string) => ({ name, is_dir: true });

describe("pathWord", () => {
  it("takes the last word when it is a path with part of a name", () => {
    expect(pathWord("cat /etc/ho")).toEqual({ folder: "/etc/", prefix: "ho" });
    expect(pathWord("~/bi")).toEqual({ folder: "~/", prefix: "bi" });
    expect(pathWord("cat /my\\ dir/no")).toEqual({ folder: "/my dir/", prefix: "no" });
  });

  it("leaves other words to the shell", () => {
    for (const typed of [
      "",
      "cat",
      "cat src/ma",
      "cat /etc/",
      'cat "/etc/ho',
      "cat /etc/*",
      "cat $HOME/x",
      "cat ~root/x",
    ]) {
      expect(pathWord(typed)).toBeNull();
    }
  });

  it("tells an open quote from a closed one", () => {
    expect(pathWord('git commit -m "fix /etc/ho')).toBeNull();
    expect(pathWord("echo 'it\\ /etc/ho")).toBeNull();
    expect(pathWord('cat "a b" /etc/ho')).toEqual({ folder: "/etc/", prefix: "ho" });
    expect(pathWord("echo it\\'s /etc/ho")).toEqual({ folder: "/etc/", prefix: "ho" });
  });
});

describe("pathRest", () => {
  it("completes the shortest name, with a slash after a folder", () => {
    expect(pathRest("ho", [file("hosts.allow"), file("hosts")])).toBe("sts");
    expect(pathRest("lo", [dir("local")])).toBe("cal/");
    expect(pathRest("local", [dir("local")])).toBe("/");
    expect(pathRest("hosts", [file("hosts")])).toBeNull();
    expect(pathRest("x", [file("hosts")])).toBeNull();
  });

  it("escapes what the shell would split, and shows hidden names only for a dot", () => {
    expect(pathRest("my", [file("my notes (1).txt")])).toBe("\\ notes\\ \\(1\\).txt");
    expect(pathRest("b", [file(".bashrc")])).toBeNull();
    expect(pathRest(".b", [file(".bashrc")])).toBe("ashrc");
    expect(pathRest("a", [file("a\nb")])).toBeNull();
  });
});

describe("PathLister", () => {
  let onChange: Mock<() => void>;
  let lister: PathLister;
  beforeEach(() => {
    vi.clearAllMocks();
    backend.home.mockResolvedValue("/home/kc");
    backend.list.mockResolvedValue([dir("bin")]);
    onChange = vi.fn<() => void>();
    lister = new PathLister(onChange);
    lister.connected("s1");
  });

  const read = async (typed: string) => {
    lister.rest(typed);
    await settle();
    lister.rest(typed);
    await settle();
    return lister.rest(typed);
  };

  it("lists the folder once and resolves ~ against the home folder", async () => {
    expect(await read("ls ~/bi")).toBe("n/");
    expect(lister.rest("ls ~/b")).toBe("in/");
    expect(backend.list.mock.calls).toEqual([["s1", "/home/kc"]]);
    expect(onChange).toHaveBeenCalled();
  });

  it("asks once per connection when the server does not answer", async () => {
    backend.home.mockRejectedValueOnce(appError("ssh", "SFTP request timed out", { ssh_kind: "sftp" }));
    expect(await read("ls /bi")).toBeNull();
    expect(backend.home).toHaveBeenCalledTimes(1);
    expect(backend.list).not.toHaveBeenCalled();
    lister.connected("s2");
    expect(await read("ls /bi")).toBe("n/");
  });

  it("suggests nothing from a folder it cannot read, and does not read it again at once", async () => {
    backend.list.mockRejectedValue(appError("ssh", "denied", { ssh_kind: "sftp" }));
    expect(await read("ls /root/x")).toBeNull();
    expect(backend.list).toHaveBeenCalledTimes(1);
  });

  it("drops an answer of the previous connection", async () => {
    await read("ls /bi");
    backend.list.mockClear();
    let answer: (entries: unknown[]) => void = () => {};
    backend.list.mockReturnValueOnce(new Promise((resolve) => (answer = resolve)));
    lister.rest("ls /usr/bi");
    lister.connected(null);
    answer([dir("bin")]);
    await settle();
    expect(lister.rest("ls /usr/bi")).toBeNull();
  });
});
