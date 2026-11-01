//! The `hatoba` side of a request: it connects to the running app, sends the request, and turns
//! the frames that come back into output and an exit status.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};

use crate::App;
use crate::args::{Command, Place, SWITCH};
use crate::endpoint;
use crate::protocol::{End, Frame, HostInfo, Op, Request, VERSION};

/// The exit status of `exec` when Hatoba could not run the command, as `ssh` has it.
const EXEC_FAILED: i32 = 255;
/// The exit status of `hosts` and `cp` when they fail.
const FAILED: i32 = 1;

pub fn run(app: &App, command: Command) -> i32 {
    match command {
        Command::Help => {
            print!("{}", crate::args::HELP);
            0
        }
        Command::Version => {
            println!("hatoba {}", app.version);
            0
        }
        Command::Hosts { json } => hosts(app, json),
        Command::Exec {
            host,
            command,
            cwd,
            prompt,
            sudo,
        } => exec(
            app,
            Op::Exec {
                host,
                command,
                cwd,
                prompt,
                sudo,
                stdin: passes_stdin(),
            },
        ),
        Command::Copy {
            source,
            target,
            prompt,
            sudo,
            mode,
        } => copy(app, source, target, prompt, sudo, mode),
        Command::Open(args) => open(app, args),
    }
}

fn hosts(app: &App, json: bool) -> i32 {
    let mut stream = match request(app, Op::Hosts) {
        Ok(stream) => stream,
        Err(message) => return fail(&message, FAILED),
    };
    loop {
        match Frame::read(&mut stream) {
            Ok(Some(Frame::Hosts(hosts))) => {
                let text = if json {
                    let mut text = serde_json::to_string_pretty(&hosts).unwrap_or_default();
                    text.push('\n');
                    text
                } else {
                    table(&hosts)
                };
                if io::stdout().write_all(text.as_bytes()).is_err() {
                    return FAILED;
                }
                if hosts.is_empty() && !json {
                    eprintln!("hatoba: no SSH hosts are saved in Hatoba yet");
                }
            }
            Ok(Some(Frame::End(end))) => return ended(end, FAILED),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return fail(ENDED_EARLY, FAILED),
        }
    }
}

const ENDED_EARLY: &str = "Hatoba closed the connection before it answered";

fn exec(app: &App, op: Op) -> i32 {
    let mut stream = match request(app, op) {
        Ok(stream) => stream,
        Err(message) => return fail(&message, EXEC_FAILED),
    };
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr();
    loop {
        match Frame::read(&mut stream) {
            Ok(Some(Frame::Stdout(bytes))) => {
                // The reader went away, as `| head` does: the command stops with the connection.
                if write_out(&mut stdout, &bytes).is_err() {
                    return EXEC_FAILED;
                }
            }
            Ok(Some(Frame::Stderr(bytes))) => {
                let _ = write_out(&mut stderr, &bytes);
            }
            Ok(Some(Frame::Notice(text))) => eprintln!("hatoba: {text}"),
            Ok(Some(Frame::SendStdin(token))) => send_stdin(*app, token),
            Ok(Some(Frame::End(end))) => return ended(end, EXEC_FAILED),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                return fail(
                    "the connection to Hatoba ended before the command did",
                    EXEC_FAILED,
                );
            }
        }
    }
}

/// Whether `exec` passes its stdin to the command: a pipe or a file, as a heredoc, `<` and `|`
/// give it, but not a terminal, mintty's included, which is a pipe on Windows, nor a device such
/// as `/dev/null` or `NUL`.
fn passes_stdin() -> bool {
    !io::stdin().is_terminal() && pipe_or_file()
}

#[cfg(unix)]
fn pipe_or_file() -> bool {
    use std::os::fd::AsFd;
    use std::os::unix::fs::FileTypeExt;
    let Ok(fd) = io::stdin().as_fd().try_clone_to_owned() else {
        return false;
    };
    std::fs::File::from(fd).metadata().is_ok_and(|meta| {
        let kind = meta.file_type();
        kind.is_fifo() || kind.is_socket() || kind.is_file()
    })
}

#[cfg(windows)]
fn pipe_or_file() -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{FILE_TYPE_DISK, FILE_TYPE_PIPE, GetFileType};
    // SAFETY: the handle is this process's stdin, or null without one, which is of no type.
    let kind = unsafe { GetFileType(HANDLE(io::stdin().as_raw_handle())) };
    kind == FILE_TYPE_PIPE || kind == FILE_TYPE_DISK
}

/// Sends this command's stdin to the app on a connection of its own, which ends where the stdin
/// does. It runs on a thread, so the output keeps coming meanwhile; a command that ends first
/// leaves it behind. Errors end it quietly: the app then gives the command's stdin its end.
fn send_stdin(app: App, token: String) {
    std::thread::spawn(move || {
        let Ok(mut stream) = request(&app, Op::Stdin { token }) else {
            return;
        };
        let _ = io::copy(&mut io::stdin().lock(), &mut stream);
        // A pipe's last bytes are read before it closes.
        #[cfg(windows)]
        let _ = stream.sync_all();
    });
}

/// Writes output as it came. A Windows console takes only UTF-8, so other bytes are shown as
/// U+FFFD there; a pipe or a file gets them as they are.
fn write_out(to: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    match to.write_all(bytes) {
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            to.write_all(String::from_utf8_lossy(bytes).as_bytes())?;
        }
        done => done?,
    }
    to.flush()
}

fn copy(
    app: &App,
    source: Place,
    target: Place,
    prompt: bool,
    sudo: bool,
    mode: Option<u32>,
) -> i32 {
    let op = match (source, target) {
        (Place::Local(local), Place::Remote { host, path }) => match absolute(&local) {
            Ok(local) => Op::Upload {
                host,
                local,
                remote: path,
                prompt,
                sudo,
                mode,
            },
            Err(message) => return fail(&message, FAILED),
        },
        (Place::Remote { host, path }, Place::Local(local)) => match absolute(&local) {
            Ok(local) => Op::Download {
                host,
                remote: path,
                local,
                prompt,
                sudo,
            },
            Err(message) => return fail(&message, FAILED),
        },
        _ => unreachable!("the arguments have one remote side"),
    };
    let mut stream = match request(app, op) {
        Ok(stream) => stream,
        Err(message) => return fail(&message, FAILED),
    };
    let progress = io::stderr().is_terminal();
    let mut shown = false;
    loop {
        match Frame::read(&mut stream) {
            Ok(Some(Frame::Progress(p))) if progress => {
                let done = p
                    .bytes
                    .saturating_mul(100)
                    .checked_div(p.total)
                    .map(|pct| format!(" {pct:>3}%"))
                    .unwrap_or_default();
                eprint!("\r{}{done}  ", size(p.bytes));
                shown = true;
            }
            Ok(Some(Frame::Notice(text))) => {
                if std::mem::take(&mut shown) {
                    eprintln!();
                }
                eprintln!("hatoba: {text}");
            }
            // What the server printed while signing in.
            Ok(Some(Frame::Stderr(bytes))) => {
                if std::mem::take(&mut shown) {
                    eprintln!();
                }
                let _ = write_out(&mut io::stderr(), &bytes);
            }
            Ok(Some(Frame::End(end))) => {
                if shown {
                    eprintln!();
                }
                return ended(end, FAILED);
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return fail(ENDED_EARLY, FAILED),
        }
    }
}

fn absolute(local: &str) -> Result<String, String> {
    #[cfg(windows)]
    let local = &msys_path(
        local,
        std::env::var_os("MSYSTEM").is_some_and(|v| !v.is_empty()),
    )?;
    let path = std::path::absolute(local).map_err(|e| format!("{local}: {e}"))?;
    path.into_os_string()
        .into_string()
        .map_err(|p| format!("{} is not valid text", p.to_string_lossy()))
}

/// Under Git Bash and MSYS2, which set `MSYSTEM`, a local path that starts with `/` is one of
/// theirs: the `hatoba` launcher passes the command's own arguments as typed, so `~/a.txt` comes
/// as `/c/Users/…/a.txt`. Their `cygpath` knows where such a path is on Windows, mounts included.
#[cfg(windows)]
fn msys_path(local: &str, msys: bool) -> Result<String, String> {
    if !msys || !local.starts_with('/') {
        return Ok(local.to_owned());
    }
    cygpath(local).map_err(|reason| {
        format!(
            "can't turn {local} into a Windows path: {reason}. \
             Give it as a Windows path, such as C:/Users/name/file.txt"
        )
    })
}

/// The path goes in on stdin: an MSYS program started from a Windows one expands wildcards and
/// quotes in its arguments.
#[cfg(windows)]
fn cygpath(path: &str) -> Result<String, String> {
    let mut child = Process::new("cygpath")
        .args(["-w", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => "cygpath is not on the PATH".to_owned(),
            _ => format!("cygpath did not start ({e})"),
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("{path}\n").as_bytes());
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("cygpath did not finish ({e})"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let windows = stdout.trim_end_matches(['\r', '\n']);
    if out.status.success() && !windows.is_empty() {
        return Ok(windows.to_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(match stderr.trim() {
        "" => "cygpath failed".to_owned(),
        message => format!("cygpath failed ({message})"),
    })
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// The exit status a request ended with, after printing its error.
fn ended(end: End, failed: i32) -> i32 {
    match end.error {
        Some(message) => fail(&message, failed),
        None => end.status.unwrap_or(0),
    }
}

fn fail(message: &str, status: i32) -> i32 {
    eprintln!("hatoba: {message}");
    status
}

/// Connects to the app and sends `op`.
fn request(app: &App, op: Op) -> Result<Stream, String> {
    let mut stream = connect(app)?;
    let frame = Frame::Request(Request {
        version: VERSION,
        op,
    });
    stream
        .write_all(&frame.encode())
        .map_err(|_| ENDED_EARLY.to_owned())?;
    Ok(stream)
}

fn unreachable_app() -> String {
    format!("can't reach Hatoba. Open Hatoba, and turn on Settings > General > \"{SWITCH}\".")
}

#[cfg(unix)]
type Stream = std::os::unix::net::UnixStream;

#[cfg(unix)]
fn connect(app: &App) -> Result<Stream, String> {
    let path = endpoint::endpoint(app.identifier).map_err(|e| e.to_string())?;
    match Stream::connect(&path) {
        Ok(stream) => Ok(stream),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            Err(unreachable_app())
        }
        Err(e) => Err(format!("can't reach Hatoba at {}: {e}", path.display())),
    }
}

#[cfg(windows)]
type Stream = std::fs::File;

#[cfg(windows)]
fn connect(app: &App) -> Result<Stream, String> {
    /// The app opens the next instance of the pipe right after one connects; until then, a
    /// client is told that every instance is busy.
    const PIPE_BUSY: i32 = 231;
    /// `SecurityIdentification << 16`: a server that took the pipe's name before Hatoba, which
    /// `check_server` turns away after the open, can learn who this user is but not act as them.
    const SECURITY_IDENTIFICATION: u32 = 1 << 16;
    use std::os::windows::fs::OpenOptionsExt;
    let name = endpoint::endpoint(app.identifier).map_err(|e| e.to_string())?;
    let mut tries = 0;
    let pipe = loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(&name)
        {
            Ok(pipe) => break pipe,
            Err(e) if e.raw_os_error() == Some(PIPE_BUSY) && tries < 100 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(unreachable_app()),
            Err(e) => return Err(format!("can't reach Hatoba: {e}")),
        }
    };
    endpoint::check_server(&pipe).map_err(|e| format!("can't reach Hatoba safely: {e}"))?;
    Ok(pipe)
}

/// Opens Hatoba with the arguments, without waiting for it, as the launcher did (HOST-13).
fn open(app: &App, args: Vec<OsString>) -> i32 {
    let Some(exe) = main_exe(app) else {
        return fail(
            &format!("{} is not beside this command", app.main_binary),
            FAILED,
        );
    };
    let mut process = Process::new(exe);
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        process.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        process.process_group(0);
    }
    match process.spawn() {
        Ok(_) => 0,
        Err(e) => fail(&format!("can't start Hatoba: {e}"), FAILED),
    }
}

/// The app's executable: beside this one, or one folder up from the `bin` folder that the
/// `hatoba` command is copied to on Windows.
fn main_exe(app: &App) -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    find_main_exe(&me, app.main_binary)
}

/// [`main_exe`] for the command at `me`. On Windows `bin\hatoba.exe` is also `bin\Hatoba.exe`,
/// since names there ignore case, so the command itself is passed over.
fn find_main_exe(me: &std::path::Path, main_binary: &str) -> Option<PathBuf> {
    let name = format!("{main_binary}{}", std::env::consts::EXE_SUFFIX);
    let me = std::fs::canonicalize(me).ok()?;
    me.ancestors()
        .skip(1)
        .take(2)
        .map(|dir| dir.join(&name))
        .find(|exe| exe.is_file() && std::fs::canonicalize(exe).is_ok_and(|exe| exe != me))
}

/// The hosts as a table: name, target and group, with nothing cut short.
fn table(hosts: &[HostInfo]) -> String {
    let rows: Vec<[String; 3]> =
        std::iter::once(["NAME".to_owned(), "TARGET".to_owned(), "GROUP".to_owned()])
            .chain(hosts.iter().map(|h| {
                let address = if h.address.contains(':') {
                    format!("[{}]", h.address)
                } else {
                    h.address.clone()
                };
                [
                    h.name.clone(),
                    format!("{}@{address}:{}", h.username, h.port),
                    h.group.clone().unwrap_or_default(),
                ]
            }))
            .collect();
    let widths = [0, 1].map(|c| rows.iter().map(|r| width(&r[c])).max().unwrap_or(0));
    let mut out = String::new();
    for row in &rows {
        for (c, cell) in row.iter().enumerate() {
            out.push_str(cell);
            if c < 2 {
                out.push_str(&" ".repeat(widths[c] - width(cell) + 2));
            }
        }
        let trimmed = out.trim_end().len();
        out.truncate(trimmed);
        out.push('\n');
    }
    out
}

/// Columns a terminal gives `text`: two for East Asian wide characters, one for the rest.
fn width(text: &str) -> usize {
    text.chars()
        .map(|c| {
            let wide = matches!(c as u32,
                0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
                | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F
                | 0x20000..=0x3FFFD);
            if wide { 2 } else { 1 }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AuthKind;

    /// The copy in `bin` opens the app one folder up, never itself.
    #[test]
    fn the_command_in_bin_finds_the_app_above_it() {
        let dir = std::env::temp_dir().join(format!("hatoba-cli-find-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let suffix = std::env::consts::EXE_SUFFIX;
        let (app, command) = (
            dir.join(format!("Hatoba{suffix}")),
            bin.join(format!("hatoba{suffix}")),
        );
        std::fs::write(&app, b"app").unwrap();
        std::fs::write(&command, b"command").unwrap();
        let found = find_main_exe(&command, "Hatoba").unwrap();
        assert_eq!(found, std::fs::canonicalize(&app).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn host(name: &str, address: &str, group: Option<&str>) -> HostInfo {
        HostInfo {
            id: format!("id-{name}"),
            name: name.into(),
            address: address.into(),
            port: 22,
            username: "root".into(),
            auth: AuthKind::Key,
            group: group.map(Into::into),
            tags: Vec::new(),
            jump: None,
            ai_notes: String::new(),
        }
    }

    #[test]
    fn the_table_lines_up_and_cuts_nothing_short() {
        let text = table(&[
            host("db", "10.0.0.5", Some("prod")),
            host("东京服务器", "fe80::1", None),
        ]);
        assert_eq!(
            text,
            "NAME        TARGET             GROUP\n\
             db          root@10.0.0.5:22   prod\n\
             东京服务器  root@[fe80::1]:22\n"
        );
    }

    #[test]
    fn sizes_read_like_a_file_manager() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(size(5 * 1024 * 1024), "5.0 MB");
    }

    /// HOST-15: `hatoba cp` from Git Bash takes the local paths its shell gives it.
    #[cfg(windows)]
    #[test]
    fn msys_paths_become_windows_paths() {
        for path in ["/c/x.txt", "C:/x.txt", r"C:\x.txt", "x.txt", "./x.txt"] {
            assert_eq!(msys_path(path, false).unwrap(), path);
        }
        for path in ["C:/x.txt", r"C:\x.txt", "./x.txt", r"\\server\share\x"] {
            assert_eq!(msys_path(path, true).unwrap(), path);
        }
        if Process::new("cygpath").arg("--version").output().is_err() {
            eprintln!("cygpath is not on the PATH: skipping");
            return;
        }
        assert_eq!(
            msys_path("/c/Users/a b/x[1] 'y'.txt", true).unwrap(),
            r"C:\Users\a b\x[1] 'y'.txt"
        );
        assert_eq!(msys_path("/d/东京/", true).unwrap(), r"D:\东京\");
        assert_eq!(
            msys_path("//server/share/x", true).unwrap(),
            r"\\server\share\x"
        );
    }

    #[test]
    fn an_error_wins_over_a_status() {
        assert_eq!(ended(End::status(3), 255), 3);
        assert_eq!(ended(End::ok(), 255), 0);
        assert_eq!(ended(End::error("locked"), 255), 255);
    }
}
