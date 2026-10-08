# Run the end-to-end smoke test

`smoke.mjs` drives the real app (Rust backend + WebView) through WebDriver and walks the main
path against a real OpenSSH server:

1. first launch → create a vault, read the recovery code, confirm its last group (VAULT-01/02);
2. create a password-auth host (HOST-01);
3. connect → first-connection host-key dialog (SSH-04, the fingerprint must match the server's)
   → run a command and print UTF-8 text over the binary terminal channel
   ([spec §10.3](../../../docs/hatoba-spec.md#103-terminal-data-flow), TERM-02);
4. open the SFTP panel (SFTP-01);
5. lock with Ctrl+Shift+L, try a wrong password, unlock (SEC-02, VAULT-03, SEC-03).

Requirement IDs refer to the [architecture and requirements document](../../../docs/hatoba-spec.md).

## Requirements

- Linux. tauri-driver uses WebKitWebDriver; there is no WebDriver for WKWebView on macOS, and
  WebView2 needs `msedgedriver` on Windows.
- `openssh-server`, `xvfb`, `dbus`, `webkit2gtk-driver`, and tauri-driver (`cargo install tauri-driver`).
- Root access: the script creates a local test user for password authentication.

## Run

Debug builds load the frontend from the dev server on `http://localhost:1420`, so start it first:

```sh
pnpm dev &
sudo apps/desktop/e2e/run-smoke.sh ./e2e-shots
```

The script starts a throwaway `sshd`, Xvfb, D-Bus and tauri-driver, builds the app in debug mode,
and writes a screenshot of every step to the output directory (`./e2e-shots` above).

## Check the vault for plaintext

After a run, confirm that the vault database holds no plaintext:
`grep -a <host name> ~/.local/share/app.hatoba.desktop/vault.db*` must find nothing.
