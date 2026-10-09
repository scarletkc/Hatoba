//! SFTP integration tests against a real OpenSSH `sshd` (set `HATOBA_SSH_IT=1`).
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::*;
use hatoba_ssh::{SftpClient, SshErrorKind, SshSession, TransferProgress};
use tokio_util::sync::CancellationToken;

macro_rules! server {
    () => {
        match SshdServer::start() {
            Some(s) => s,
            None => return,
        }
    };
}

async fn sftp_for(server: &SshdServer) -> (SshSession, SftpClient) {
    let session = server.connect_key("ed25519", None).await;
    let sftp = session.sftp().await.expect("sftp subsystem");
    (session, sftp)
}

fn recorder() -> (
    Arc<Mutex<Vec<TransferProgress>>>,
    impl Fn(TransferProgress) + Send + Sync + 'static,
) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    (events, move |p| sink.lock().unwrap().push(p))
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn home_and_listing_order() {
    let server = server!();
    let (session, sftp) = sftp_for(&server).await;

    let (_, echo_home) = session.exec("printf %s \"$HOME\"", None).await.unwrap();
    assert_eq!(
        sftp.home().await.unwrap(),
        String::from_utf8(echo_home).unwrap()
    );

    let area = server.area("list");
    fs::create_dir(area.join("c")).unwrap();
    fs::create_dir(area.join("Zdir")).unwrap();
    fs::write(area.join("b.txt"), b"bb").unwrap();
    fs::write(area.join("A.txt"), b"A").unwrap();
    fs::write(area.join("a.TXT"), b"aaa").unwrap();
    fs::write(area.join(".hidden"), b"").unwrap();
    fs::write(area.join("run.sh"), b"#!/bin/sh\n").unwrap();
    fs::set_permissions(area.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(area.join("c"), fs::Permissions::from_mode(0o700)).unwrap();
    symlink(area.join("c"), area.join("link_to_dir")).unwrap();
    symlink(area.join("b.txt"), area.join("link_to_file")).unwrap();
    symlink(area.join("missing"), area.join("broken_link")).unwrap();

    let entries = sftp.list(s(&area)).await.unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "c",
            "link_to_dir",
            "Zdir", // directories first (the link points to a directory)
            ".hidden",
            "A.txt",
            "a.TXT",
            "b.txt",
            "broken_link",
            "link_to_file",
            "run.sh",
        ]
    );
    assert!(!names.contains(&".") && !names.contains(&".."));

    let by = |n: &str| entries.iter().find(|e| e.name == n).unwrap();
    assert_eq!(by("c").path, format!("{}/c", area.display()));
    assert!(by("c").is_dir && !by("c").is_symlink);
    assert_eq!(by("c").permissions, "drwx------");
    assert!(by("link_to_dir").is_dir && by("link_to_dir").is_symlink);
    assert!(!by("link_to_file").is_dir && by("link_to_file").is_symlink);
    assert!(!by("broken_link").is_dir && by("broken_link").is_symlink);
    assert!(by("link_to_dir").permissions.starts_with('l'));
    assert_eq!(by("run.sh").permissions, "-rwxr-xr-x");
    assert_eq!(by("run.sh").mode & 0o777, 0o755);
    assert_eq!(by("a.TXT").size, 3);
    assert_eq!(by("b.txt").size, 2);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let modified = by("b.txt").modified.expect("mtime");
    assert!(
        (now_ms - modified).abs() < 120_000,
        "mtime {modified} vs now {now_ms}"
    );

    // Trailing slash does not double up.
    let again = sftp.list(&format!("{}/", area.display())).await.unwrap();
    assert_eq!(again[0].path, format!("{}/c", area.display()));

    // Serializes for the frontend.
    let json = serde_json::to_value(by("run.sh")).unwrap();
    assert_eq!(json["is_dir"], false);
    assert_eq!(json["permissions"], "-rwxr-xr-x");
}

#[tokio::test]
async fn listing_errors_are_sftp_errors() {
    let server = server!();
    let (_s, sftp) = sftp_for(&server).await;
    let err = sftp.list("/definitely/not/here").await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Sftp, "{err}");
    assert!(err.message.contains("/definitely/not/here"), "{err}");
    assert!(err.message.to_lowercase().contains("no such file"), "{err}");

    let area = server.area("errs");
    fs::write(area.join("file"), b"x").unwrap();
    assert_eq!(
        sftp.list(s(&area.join("file"))).await.unwrap_err().kind,
        SshErrorKind::Sftp
    );
    assert_eq!(
        sftp.mkdir(s(&area)).await.unwrap_err().kind,
        SshErrorKind::Sftp
    );
    fs::create_dir(area.join("full")).unwrap();
    fs::write(area.join("full/x"), b"x").unwrap();
    assert_eq!(
        sftp.remove_dir(s(&area.join("full")))
            .await
            .unwrap_err()
            .kind,
        SshErrorKind::Sftp
    );
    assert_eq!(
        sftp.remove_file(s(&area.join("nope")))
            .await
            .unwrap_err()
            .kind,
        SshErrorKind::Sftp
    );
}

#[tokio::test]
async fn mkdir_rename_remove() {
    let server = server!();
    let (_s, sftp) = sftp_for(&server).await;
    let area = server.area("ops");

    let dir = area.join("new dir");
    sftp.mkdir(s(&dir)).await.unwrap();
    assert!(dir.is_dir());

    fs::write(dir.join("f.txt"), b"data").unwrap();
    sftp.rename(s(&dir.join("f.txt")), s(&dir.join("g.txt")))
        .await
        .unwrap();
    assert!(!dir.join("f.txt").exists() && dir.join("g.txt").exists());

    // Rename onto an existing file fails (SFTP v3) and leaves both in place.
    fs::write(dir.join("h.txt"), b"other").unwrap();
    assert!(
        sftp.rename(s(&dir.join("g.txt")), s(&dir.join("h.txt")))
            .await
            .is_err()
    );
    assert!(dir.join("g.txt").exists() && dir.join("h.txt").exists());

    sftp.rename(s(&dir), s(&area.join("renamed")))
        .await
        .unwrap();
    assert!(area.join("renamed/g.txt").exists());

    sftp.remove_file(s(&area.join("renamed/g.txt")))
        .await
        .unwrap();
    sftp.remove_file(s(&area.join("renamed/h.txt")))
        .await
        .unwrap();
    sftp.remove_dir(s(&area.join("renamed"))).await.unwrap();
    assert!(!area.join("renamed").exists());
}

#[tokio::test]
async fn remove_recursive_deletes_trees_but_never_follows_links() {
    let server = server!();
    let (_s, sftp) = sftp_for(&server).await;
    let area = server.area("rm");

    let outside = area.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep.txt"), b"keep").unwrap();

    let tree = area.join("tree");
    fs::create_dir_all(tree.join("a/b/c")).unwrap();
    fs::write(tree.join("a/b/c/deep.txt"), b"1").unwrap();
    fs::write(tree.join("a/b/f.txt"), b"2").unwrap();
    fs::write(tree.join("a/g.txt"), b"3").unwrap();
    fs::write(tree.join("top.txt"), b"4").unwrap();
    fs::create_dir(tree.join("empty")).unwrap();
    symlink(&outside, tree.join("a/link_to_outside")).unwrap();
    symlink(outside.join("keep.txt"), tree.join("file_link")).unwrap();

    sftp.remove_recursive(s(&tree)).await.unwrap();
    assert!(!tree.exists());
    assert!(
        outside.join("keep.txt").exists(),
        "link target must survive"
    );

    // A plain file works too, and so does a trailing slash.
    fs::write(area.join("single"), b"x").unwrap();
    sftp.remove_recursive(s(&area.join("single")))
        .await
        .unwrap();
    assert!(!area.join("single").exists());
    fs::create_dir_all(area.join("slash/inner")).unwrap();
    sftp.remove_recursive(&format!("{}/slash/", area.display()))
        .await
        .unwrap();
    assert!(!area.join("slash").exists());

    // Refuse the root and report missing paths.
    assert_eq!(
        sftp.remove_recursive("/").await.unwrap_err().kind,
        SshErrorKind::Sftp
    );
    assert_eq!(
        sftp.remove_recursive("").await.unwrap_err().kind,
        SshErrorKind::Sftp
    );
    assert_eq!(
        sftp.remove_recursive(s(&area.join("gone")))
            .await
            .unwrap_err()
            .kind,
        SshErrorKind::Sftp
    );
}

#[tokio::test]
async fn upload_and_download_round_trip_with_progress() {
    let server = server!();
    let (_s, sftp) = sftp_for(&server).await;
    let area = server.area("xfer");
    let local = tempfile::tempdir().unwrap();

    let data = pseudo_random(6 * 1024 * 1024 + 123, 42);
    let src = local.path().join("src.bin");
    fs::write(&src, &data).unwrap();
    let remote = area.join("remote.bin");

    // Upload.
    let (events, cb) = recorder();
    let started = Instant::now();
    sftp.upload(&src, s(&remote), cb, CancellationToken::new())
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(sha256_hex(&fs::read(&remote).unwrap()), sha256_hex(&data));
    assert_eq!(names_in(&area), ["remote.bin"], "no leftovers");
    let ev = events.lock().unwrap().clone();
    let last = ev.last().expect("final progress event");
    assert_eq!(
        (last.bytes, last.total),
        (data.len() as u64, data.len() as u64)
    );
    assert!(ev.windows(2).all(|w| w[0].bytes <= w[1].bytes), "monotonic");
    assert!(ev.iter().all(|e| e.total == data.len() as u64));
    assert!(
        ev.len() as u128 <= elapsed.as_millis() / 100 + 2,
        "{} events in {elapsed:?}",
        ev.len()
    );
    assert!(last.bytes_per_sec > 0);

    // Download to a new place.
    let dst = local.path().join("dst.bin");
    let (events, cb) = recorder();
    sftp.download(s(&remote), &dst, cb, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(sha256_hex(&fs::read(&dst).unwrap()), sha256_hex(&data));
    assert_eq!(
        names_in(local.path()),
        ["dst.bin", "src.bin"],
        "no leftovers"
    );
    let last = *events.lock().unwrap().last().unwrap();
    assert_eq!(
        (last.bytes, last.total),
        (data.len() as u64, data.len() as u64)
    );

    // Empty files work and still report a final event.
    let empty = local.path().join("empty");
    fs::write(&empty, b"").unwrap();
    let (events, cb) = recorder();
    sftp.upload(&empty, s(&area.join("empty")), cb, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(fs::metadata(area.join("empty")).unwrap().len(), 0);
    assert_eq!(events.lock().unwrap().len(), 1);
    let back = local.path().join("empty_back");
    sftp.download(
        s(&area.join("empty")),
        &back,
        |_| {},
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(fs::metadata(&back).unwrap().len(), 0);
}

#[tokio::test]
async fn upload_replaces_existing_files_atomically() {
    let server = server!();
    let (_s, sftp) = sftp_for(&server).await;
    let area = server.area("replace");
    let local = tempfile::tempdir().unwrap();
    let remote = area.join("doc.txt");

    for (i, body) in ["first version", "second, longer version"]
        .iter()
        .enumerate()
    {
        let src = local.path().join(format!("v{i}"));
        fs::write(&src, body).unwrap();
        sftp.upload(&src, s(&remote), |_| {}, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&remote).unwrap(), *body);
    }
    let mut names: Vec<_> = fs::read_dir(&area)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, ["doc.txt"], "no .part or backup files remain");

    // Download over an existing local file replaces it.
    let dst = local.path().join("existing");
    fs::write(&dst, "old local content that is longer than the remote one").unwrap();
    sftp.download(s(&remote), &dst, |_| {}, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(&dst).unwrap(), "second, longer version");
}

#[tokio::test]
async fn upload_cancel_removes_partial_remote_file() {
    let server = server!();
    let (_s, sftp) = sftp_for(&server).await;
    let area = server.area("cancel_up");
    let local = tempfile::tempdir().unwrap();

    // 512 MiB sparse file: far too big to finish before the first progress event.
    let src = local.path().join("huge");
    fs::File::create(&src)
        .unwrap()
        .set_len(512 * 1024 * 1024)
        .unwrap();
    let remote = area.join("huge.bin");
    fs::write(&remote, b"original content").unwrap();

    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let started = Instant::now();
    let err = sftp
        .upload(
            &src,
            s(&remote),
            move |p| {
                if p.bytes >= 1024 * 1024 {
                    trigger.cancel();
                }
            },
            cancel,
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Cancelled, "{err}");
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(
        fs::read_to_string(&remote).unwrap(),
        "original content",
        "original untouched"
    );
    let mut names: Vec<_> = fs::read_dir(&area)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, ["huge.bin"], "partial upload removed");

    // The connection is still usable afterwards.
    assert!(sftp.list(s(&area)).await.is_ok());
}

#[tokio::test]
async fn download_cancel_and_failure_remove_partial_local_file() {
    let server = server!();
    let (_s, sftp) = sftp_for(&server).await;
    let area = server.area("cancel_down");
    let local = tempfile::tempdir().unwrap();

    let remote = area.join("huge");
    fs::File::create(&remote)
        .unwrap()
        .set_len(512 * 1024 * 1024)
        .unwrap();
    let dst = local.path().join("huge.local");

    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let err = sftp
        .download(
            s(&remote),
            &dst,
            move |p| {
                if p.bytes >= 1024 * 1024 {
                    trigger.cancel();
                }
            },
            cancel,
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Cancelled, "{err}");
    assert!(!dst.exists());
    assert!(fs::read_dir(local.path()).unwrap().next().is_none());

    // Pre-cancelled token: nothing is created.
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let err = sftp
        .download(s(&remote), &dst, |_| {}, cancelled)
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Cancelled);
    assert!(fs::read_dir(local.path()).unwrap().next().is_none());

    // Missing remote file and directories fail cleanly.
    let err = sftp
        .download(
            s(&area.join("nope")),
            &dst,
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Sftp, "{err}");
    let err = sftp
        .download(s(&area), &dst, |_| {}, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Sftp, "{err}");
    assert!(err.message.contains("directory"), "{err}");
    assert!(fs::read_dir(local.path()).unwrap().next().is_none());

    // Unwritable local target.
    let err = sftp
        .download(
            s(&remote),
            &local.path().join("no/such/dir/file"),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Io, "{err}");
}

#[tokio::test]
async fn upload_errors() {
    let server = server!();
    let (_s, sftp) = sftp_for(&server).await;
    let area = server.area("up_err");
    let local = tempfile::tempdir().unwrap();

    let err = sftp
        .upload(
            &local.path().join("missing"),
            s(&area.join("x")),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Io, "{err}");
    let err = sftp
        .upload(
            local.path(),
            s(&area.join("x")),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Io, "{err}");

    let src = local.path().join("f");
    fs::write(&src, b"x").unwrap();
    let err = sftp
        .upload(
            &src,
            s(&area.join("no/such/dir/f")),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Sftp, "{err}");
}

#[tokio::test]
async fn sftp_after_disconnect_reports_the_connection_error() {
    let server = server!();
    let (session, sftp) = sftp_for(&server).await;
    let area = server.area("gone");
    assert!(sftp.list(s(&area)).await.is_ok());
    session.disconnect().await;
    let err = sftp.list(s(&area)).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Disconnected, "{err}");
}

#[tokio::test]
async fn sftp_keeps_the_connection_alive_after_other_handles_drop() {
    let server = server!();
    let area = server.area("alive");
    fs::write(area.join("f"), b"x").unwrap();
    let sftp = {
        let session = server.connect_key("ed25519", None).await;
        session.sftp().await.unwrap()
        // `session` handle dropped here; the SftpClient holds the connection open.
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(sftp.list(s(&area)).await.unwrap().len(), 1);
}

#[tokio::test]
async fn transfers_leave_existing_part_files_alone() {
    let server = server!();
    let (_s, sftp) = sftp_for(&server).await;
    let area = server.area("part_sibling");
    let local = tempfile::tempdir().unwrap();
    let keep = "unrelated irreplaceable data";

    // Upload next to a remote `report.txt.part` that belongs to someone else.
    fs::write(area.join("report.txt.part"), keep).unwrap();
    let src = local.path().join("src.txt");
    fs::write(&src, "uploaded").unwrap();
    sftp.upload(
        &src,
        s(&area.join("report.txt")),
        |_| {},
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        fs::read_to_string(area.join("report.txt")).unwrap(),
        "uploaded"
    );
    assert_eq!(
        fs::read_to_string(area.join("report.txt.part")).unwrap(),
        keep
    );
    assert_eq!(names_in(&area), ["report.txt", "report.txt.part"]);

    // Download next to a local `report.txt.part`.
    let dst = local.path().join("report.txt");
    fs::write(local.path().join("report.txt.part"), keep).unwrap();
    sftp.download(
        s(&area.join("report.txt")),
        &dst,
        |_| {},
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(fs::read_to_string(&dst).unwrap(), "uploaded");
    assert_eq!(
        fs::read_to_string(local.path().join("report.txt.part")).unwrap(),
        keep
    );
    assert_eq!(
        names_in(local.path()),
        ["report.txt", "report.txt.part", "src.txt"]
    );

    // A cancelled upload removes only its own temporary file.
    let huge = local.path().join("huge");
    fs::File::create(&huge)
        .unwrap()
        .set_len(512 * 1024 * 1024)
        .unwrap();
    fs::write(area.join("huge.bin.part"), keep).unwrap();
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let err = sftp
        .upload(
            &huge,
            s(&area.join("huge.bin")),
            move |p| {
                if p.bytes >= 1024 * 1024 {
                    trigger.cancel();
                }
            },
            cancel,
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Cancelled, "{err}");
    assert_eq!(
        fs::read_to_string(area.join("huge.bin.part")).unwrap(),
        keep
    );
    assert_eq!(
        names_in(&area),
        ["huge.bin.part", "report.txt", "report.txt.part"]
    );
}

#[tokio::test]
async fn concurrent_transfers_to_the_same_destination_do_not_collide() {
    let server = server!();
    let (_s, sftp) = sftp_for(&server).await;
    let area = server.area("same_dest");
    let local = tempfile::tempdir().unwrap();

    let a = pseudo_random(3 * 1024 * 1024, 1);
    let b = pseudo_random(3 * 1024 * 1024 + 7, 2);
    let (src_a, src_b) = (local.path().join("a"), local.path().join("b"));
    fs::write(&src_a, &a).unwrap();
    fs::write(&src_b, &b).unwrap();
    let remote = area.join("shared.bin");
    let (ra, rb) = tokio::join!(
        sftp.upload(&src_a, s(&remote), |_| {}, CancellationToken::new()),
        sftp.upload(&src_b, s(&remote), |_| {}, CancellationToken::new()),
    );
    ra.unwrap();
    rb.unwrap();
    let got = sha256_hex(&fs::read(&remote).unwrap());
    assert!(
        got == sha256_hex(&a) || got == sha256_hex(&b),
        "one whole upload wins"
    );
    assert_eq!(names_in(&area), ["shared.bin"]);

    // Concurrent uploads over an existing file: at least one wins, no backup
    // or temporary file is left behind, and later uploads still work.
    for round in 0..5 {
        let (ra, rb) = tokio::join!(
            sftp.upload(&src_a, s(&remote), |_| {}, CancellationToken::new()),
            sftp.upload(&src_b, s(&remote), |_| {}, CancellationToken::new()),
        );
        assert!(ra.is_ok() || rb.is_ok(), "round {round}: {ra:?} {rb:?}");
        let got = sha256_hex(&fs::read(&remote).unwrap());
        assert!(
            got == sha256_hex(&a) || got == sha256_hex(&b),
            "round {round}"
        );
        assert_eq!(names_in(&area), ["shared.bin"], "round {round}");
    }
    sftp.upload(&src_a, s(&remote), |_| {}, CancellationToken::new())
        .await
        .unwrap();
    let got = sha256_hex(&fs::read(&remote).unwrap());
    assert_eq!(got, sha256_hex(&a));

    let dst = local.path().join("dst.bin");
    let (da, db) = tokio::join!(
        sftp.download(s(&remote), &dst, |_| {}, CancellationToken::new()),
        sftp.download(s(&remote), &dst, |_| {}, CancellationToken::new()),
    );
    da.unwrap();
    db.unwrap();
    assert_eq!(sha256_hex(&fs::read(&dst).unwrap()), got);
    assert_eq!(names_in(local.path()), ["a", "b", "dst.bin"]);
}
