//! Compile-time checks that the public handles can be shared across tasks and threads
//! (the Tauri shell keeps them in managed state).

use hatoba_ssh::{
    ConnectConfig, FileEntry, ForwardHandle, HostKeyInfo, ParsedKey, SftpClient, ShellEvent,
    ShellHandle, SshError, SshSession, StatsEvent, StatsHandle,
};

fn send_sync<T: Send + Sync>() {}
fn send<T: Send>() {}
fn clone<T: Clone>() {}

#[test]
fn handles_are_send_sync_clone() {
    send_sync::<SshSession>();
    send_sync::<ShellHandle>();
    send_sync::<SftpClient>();
    send_sync::<ForwardHandle>();
    send_sync::<SshError>();
    send_sync::<ConnectConfig>();
    send_sync::<HostKeyInfo>();
    send_sync::<FileEntry>();
    send_sync::<ParsedKey>();
    send::<ShellEvent>();
    send_sync::<StatsHandle>();
    send::<StatsEvent>();
    clone::<SshSession>();
    clone::<ShellHandle>();
    clone::<SftpClient>();
    clone::<ForwardHandle>();
    clone::<ConnectConfig>();
    clone::<ShellEvent>();
}

/// The futures returned by the public async API must be `Send` so they can be spawned.
#[test]
fn futures_are_send() {
    fn assert_send<T: Send>(_: &T) {}
    async fn exercise(s: SshSession, sftp: SftpClient, shell: ShellHandle) {
        assert_send(&s.exec("true", None));
        assert_send(&s.open_shell(Default::default()));
        assert_send(&s.sftp());
        assert_send(&s.open_stats(std::time::Duration::from_secs(2)));
        assert_send(&s.disconnect());
        assert_send(&s.closed());
        assert_send(&sftp.list("/"));
        assert_send(&sftp.home());
        assert_send(&sftp.remove_recursive("/x"));
        assert_send(&sftp.download("/a", std::path::Path::new("b"), |_| {}, Default::default()));
        assert_send(&sftp.upload(std::path::Path::new("b"), "/a", |_| {}, Default::default()));
        assert_send(&shell.write(vec![]));
        assert_send(&shell.resize(1, 1));
        assert_send(&shell.close());
    }
    let _ = exercise;
}
