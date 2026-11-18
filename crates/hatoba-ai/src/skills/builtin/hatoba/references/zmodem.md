# ZMODEM file transfers (sz/rz)

The desktop terminal detects ZMODEM on its existing byte stream. Run `rz` on the
remote host, then choose local files in the transfer bar to upload. Run
`sz filename` to download and choose **Receive** for each offered file. The bar
shows progress and **Cancel transfer**; Ctrl+C also cancels. A transfer reserves
terminal input, including AI typing, until it ends. It does not use SFTP or open
another SSH connection, so it follows an interactive SSH hop or elevated shell.

Settings → Terminal → ZMODEM sets a device-local download directory. The default
is the current user's Downloads directory. Files stream to temporary files and
are published only after a successful transfer; existing names receive a numeric
suffix. Remote paths, control characters and Windows special names are rejected.
Publishing requires hard-link support in the destination filesystem (for example
NTFS or ext4); an unsupported filesystem produces an error without overwriting
existing data. Cancellation, disconnection and locking discard partial downloads.
An idle transfer times out after two minutes. Uploads are read in 64 KiB chunks;
the pending disk-write queue is capped at 8 MiB and aborts if the disk cannot keep
up. Binary payloads are excluded from terminal display and AI output capture.

Only ZMODEM over a reliable byte stream is supported, without resume or
XMODEM/YMODEM fallback. The remote host needs `lrzsz` or a compatible program.
