//! The sync engine: one round of pull → push (spec §6.3).
//!
//! ```text
//! round:  pull pages from sync_cursor until has_more = false
//!           · local row clean  → overwrite with the remote row
//!           · local row dirty  → resolve (sync::conflict), log, maybe keep a conflict copy
//!         push every dirty row in chunks of ≤ 100
//!           · ok        → record revision; clear dirty only if the row was not edited meanwhile
//!           · conflict  → resolve; if the local version still wins, retry once on top of the
//!                         server revision
//!         stamp sync_last_at
//! ```
//!
//! **Locking discipline:** the vault lives behind a `std::sync::Mutex`. Every step locks it,
//! reads or writes locally, and unlocks *before* any network `await`, so the UI thread is never
//! blocked on I/O and the guard is never held across a suspension point.
//!
//! **Cursor discipline:** `sync_cursor` only ever advances from *pulled* sequence numbers. The
//! `seq` returned for our own pushes is deliberately ignored: another device may have written
//! items with a lower `seq` between our pull and our push, and jumping past them would lose them.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::model::{Item, new_id};
use crate::store::{ItemRow, NewConflict, StoreOps, StoreTx, meta};
use crate::sync::backend::{Change, MAX_CHANGES_PER_PUSH, PushResult, RemoteItem, SyncBackend};
use crate::sync::conflict::{DEFAULT_CONFLICT_SUFFIX, Decision, Side, conflict_copy, decide};
use crate::sync::{SharedVault, lock_vault};
use crate::vault::{SyncCtx, Vault, decode_envelope, seal_item};

/// Items requested per pull page.
pub const PULL_PAGE_SIZE: u32 = 500;
/// Times one item may be pushed within a single round (the first push plus one retry).
const MAX_PUSH_ATTEMPTS: u8 = 2;

/// Tunables of a sync round.
#[derive(Clone, Debug)]
pub struct SyncOptions {
    /// Appended to the name of a preserved key conflict copy. The desktop app passes the
    /// localized "（冲突副本）".
    pub conflict_suffix: String,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            conflict_suffix: DEFAULT_CONFLICT_SUFFIX.to_owned(),
        }
    }
}

/// What a round did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SyncReport {
    /// Remote items applied to the local store.
    pub pulled: u32,
    /// Local changes accepted by the server.
    pub pushed: u32,
    /// Conflicts resolved automatically (and logged).
    pub conflicts_resolved: u32,
    /// Remote items that could not be decrypted and were left out of the vault.
    pub skipped: u32,
    /// Local changes the server rejected for this round (for example `too_large`).
    pub failed: u32,
    /// Items still waiting to be pushed after the round.
    pub pending_after: u64,
}

/// Drives sync rounds for one vault against one backend.
pub struct SyncEngine {
    vault: SharedVault,
    backend: Arc<dyn SyncBackend>,
    options: SyncOptions,
}

impl SyncEngine {
    /// An engine with default options.
    #[must_use]
    pub fn new(vault: SharedVault, backend: Arc<dyn SyncBackend>) -> Self {
        Self {
            vault,
            backend,
            options: SyncOptions::default(),
        }
    }

    /// Sets the suffix used for key conflict copies.
    #[must_use]
    pub fn with_conflict_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.options.conflict_suffix = suffix.into();
        self
    }

    /// The vault this engine syncs.
    #[must_use]
    pub fn vault(&self) -> &SharedVault {
        &self.vault
    }

    /// The backend this engine talks to.
    #[must_use]
    pub fn backend(&self) -> &Arc<dyn SyncBackend> {
        &self.backend
    }

    /// Runs one sync round.
    ///
    /// # Errors
    /// [`Error::Locked`] if the vault is locked; transport errors ([`Error::Offline`],
    /// [`Error::Unauthorized`], …). Local state is consistent after any failure and the next round
    /// resumes where this one stopped.
    pub async fn sync(&self) -> Result<SyncReport> {
        sync_round(&self.vault, self.backend.as_ref(), &self.options).await
    }
}

/// Runs `f` with the vault locked. The guard never escapes, so it can never cross an `await`.
fn with_vault<T>(vault: &SharedVault, f: impl FnOnce(&mut Vault) -> Result<T>) -> Result<T> {
    let mut guard = lock_vault(vault)?;
    f(&mut guard)
}

/// One full round: pull, then push.
///
/// # Errors
/// See [`SyncEngine::sync`].
pub async fn sync_round(
    vault: &SharedVault,
    backend: &dyn SyncBackend,
    options: &SyncOptions,
) -> Result<SyncReport> {
    with_vault(vault, |v| {
        if v.is_unlocked() {
            Ok(())
        } else {
            Err(Error::Locked)
        }
    })?;
    let mut report = SyncReport::default();
    pull_all(vault, backend, options, &mut report).await?;
    // Message parts of conversations deleted on either side, so their tombstones go out with
    // this round's push (§13.7). A failure here leaves the parts for the next round.
    with_vault(vault, |v| {
        match v.ai_sweep_orphaned_parts() {
            Ok(0) => {}
            Ok(parts) => tracing::info!(parts, "deleted message parts of deleted conversations"),
            Err(e) => tracing::warn!("could not delete orphaned message parts: {e}"),
        }
        Ok(())
    })?;
    push_all(vault, backend, options, &mut report).await?;
    with_vault(vault, |v| {
        let now = v.clock.now_ms();
        v.store.set_meta(meta::SYNC_LAST_AT, &now.to_string())?;
        report.pending_after = v.pending_count();
        Ok(())
    })?;
    tracing::debug!(
        pulled = report.pulled,
        pushed = report.pushed,
        conflicts = report.conflicts_resolved,
        skipped = report.skipped,
        pending = report.pending_after,
        "sync round finished"
    );
    Ok(report)
}

// ---- pull ---------------------------------------------------------------------------------

async fn pull_all(
    vault: &SharedVault,
    backend: &dyn SyncBackend,
    options: &SyncOptions,
    report: &mut SyncReport,
) -> Result<()> {
    loop {
        let since = with_vault(vault, |v| Ok(v.sync_cursor()))?;
        let page = backend.pull(since, PULL_PAGE_SIZE).await?;
        let has_more = page.has_more;
        let next = page.next_since;
        with_vault(vault, |v| {
            let mut ctx = v.sync_ctx()?;
            apply_pull_page(&mut ctx, &page.items, next, options, report)
        })?;
        if !has_more {
            return Ok(());
        }
        if next <= since {
            // A server that says "more" but does not move the cursor would loop us forever.
            return Err(Error::Protocol("pull cursor did not advance".into()));
        }
    }
}

/// Accumulates what one transaction changed, so the cache can be refreshed after commit.
#[derive(Default)]
struct Batch {
    touched: Vec<String>,
    pulled: u32,
    conflicts: u32,
    max_seq: u64,
}

fn apply_pull_page(
    ctx: &mut SyncCtx<'_>,
    items: &[RemoteItem],
    next_since: u64,
    options: &SyncOptions,
    report: &mut SyncReport,
) -> Result<()> {
    let (key, now) = (ctx.key, ctx.now);
    let batch = ctx.store.transaction(|tx| {
        let mut batch = Batch::default();
        for remote in items {
            batch.max_seq = batch.max_seq.max(remote.seq);
            apply_remote(
                tx,
                key,
                now,
                &options.conflict_suffix,
                remote,
                false,
                &mut batch,
            )?;
        }
        let current = tx
            .get_meta_i64(meta::SYNC_CURSOR)?
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or(0);
        let cursor = current.max(next_since).max(batch.max_seq);
        tx.set_meta(meta::SYNC_CURSOR, &cursor.to_string())?;
        Ok(batch)
    })?;
    finish_batch(ctx, batch, report)
}

/// Refreshes the decrypted cache for everything a committed transaction touched and folds the
/// counters into the report.
fn finish_batch(ctx: &mut SyncCtx<'_>, batch: Batch, report: &mut SyncReport) -> Result<()> {
    for id in &batch.touched {
        if !ctx.refresh(id)? {
            report.skipped += 1;
        }
    }
    report.pulled += batch.pulled;
    report.conflicts_resolved += batch.conflicts;
    Ok(())
}

/// What happened to a remote row relative to the local one.
#[derive(Debug, PartialEq, Eq)]
enum Disposition {
    /// The local row already reflects (or is newer than) this revision.
    UpToDate,
    /// The row now mirrors the remote state and is clean.
    Adopted,
    /// The local edit won; the row is still dirty, now based on the remote revision.
    LocalKept,
}

fn remote_row(remote: &RemoteItem) -> ItemRow {
    ItemRow {
        id: remote.id.clone(),
        envelope: remote.envelope.clone().filter(|_| !remote.deleted),
        revision: remote.revision,
        deleted: remote.deleted,
        dirty: false,
        updated_at: remote.updated_at,
    }
}

/// Applies one remote row (from a pull page or a push conflict) to the local store.
///
/// `from_push` marks a row returned as the *server side of a push conflict*: there the server
/// revision may legitimately be lower than ours (a rolled-back server), and the right move is to
/// push our newer data on top of it.
fn apply_remote(
    tx: &StoreTx<'_>,
    key: &[u8; 32],
    now: i64,
    suffix: &str,
    remote: &RemoteItem,
    from_push: bool,
    batch: &mut Batch,
) -> Result<Disposition> {
    let Some(local) = tx.item_row(&remote.id)? else {
        tx.upsert_item_row(&remote_row(remote))?;
        batch.touched.push(remote.id.clone());
        batch.pulled += 1;
        return Ok(Disposition::Adopted);
    };

    if !local.dirty {
        if remote.revision <= local.revision {
            return Ok(Disposition::UpToDate);
        }
        tx.upsert_item_row(&remote_row(remote))?;
        batch.touched.push(remote.id.clone());
        batch.pulled += 1;
        return Ok(Disposition::Adopted);
    }

    // Local has unpushed changes.
    if remote.revision <= local.revision {
        if from_push && remote.revision != local.revision {
            tx.set_row_revision(&remote.id, remote.revision)?;
            return Ok(Disposition::LocalKept);
        }
        return Ok(Disposition::UpToDate);
    }
    resolve_conflict(tx, key, now, suffix, &local, remote, batch)
}

fn resolve_conflict(
    tx: &StoreTx<'_>,
    key: &[u8; 32],
    now: i64,
    suffix: &str,
    local: &ItemRow,
    remote: &RemoteItem,
    batch: &mut Batch,
) -> Result<Disposition> {
    let id = remote.id.as_str();

    // Decrypt both sides. A remote we cannot read cannot win a comparison: keep our version and
    // let the push overwrite it. A local row we cannot read loses to the remote.
    let remote_item = match remote.envelope.as_deref().filter(|_| !remote.deleted) {
        Some(env) => match decode_envelope(key, id, env) {
            Ok(item) => Some(item),
            Err(_) => {
                tracing::warn!(
                    item_id = id,
                    "remote item could not be decrypted; keeping local version"
                );
                tx.set_row_revision(id, remote.revision)?;
                return Ok(Disposition::LocalKept);
            }
        },
        None => None,
    };
    let local_item = match local.envelope.as_deref().filter(|_| !local.deleted) {
        Some(env) => match decode_envelope(key, id, env) {
            Ok(item) => Some(item),
            Err(_) => {
                tracing::warn!(
                    item_id = id,
                    "local item could not be decrypted; taking remote version"
                );
                tx.upsert_item_row(&remote_row(remote))?;
                batch.touched.push(id.to_owned());
                batch.pulled += 1;
                return Ok(Disposition::Adopted);
            }
        },
        None => None,
    };

    let local_side = local_item
        .as_ref()
        .map_or_else(|| Side::tombstone(local.updated_at), Side::live);
    let remote_side = remote_item
        .as_ref()
        .map_or_else(|| Side::tombstone(remote.updated_at), Side::live);
    let decision = decide(&local_side, &remote_side);

    if let Some(resolution) = decision.resolution() {
        tx.insert_conflict(&NewConflict {
            item_id: id.to_owned(),
            resolution: resolution.as_str().to_owned(),
            local_envelope: local.envelope.clone(),
            remote_envelope: remote.envelope.clone(),
            local_deleted: local.deleted,
            remote_deleted: remote.deleted,
            local_updated_at: Some(local_side.updated_at),
            remote_updated_at: Some(remote_side.updated_at),
            created_at: now,
        })?;
        batch.conflicts += 1;
    }

    match decision {
        Decision::Converged => {
            tx.upsert_item_row(&remote_row(remote))?;
            batch.touched.push(id.to_owned());
            Ok(Disposition::Adopted)
        }
        Decision::KeepLocal { copy_remote } => {
            if copy_remote && let Some(loser) = &remote_item {
                save_conflict_copy(tx, key, now, suffix, loser, batch)?;
            }
            tx.set_row_revision(id, remote.revision)?;
            Ok(Disposition::LocalKept)
        }
        Decision::KeepRemote { copy_local } => {
            if copy_local && let Some(loser) = &local_item {
                save_conflict_copy(tx, key, now, suffix, loser, batch)?;
            }
            tx.upsert_item_row(&remote_row(remote))?;
            batch.touched.push(id.to_owned());
            batch.pulled += 1;
            Ok(Disposition::Adopted)
        }
    }
}

/// Stores `loser` as a brand-new dirty item so it propagates and is never lost.
fn save_conflict_copy(
    tx: &StoreTx<'_>,
    key: &[u8; 32],
    now: i64,
    suffix: &str,
    loser: &Item,
    batch: &mut Batch,
) -> Result<()> {
    let copy = conflict_copy(loser, suffix, now);
    let copy_id = new_id();
    let envelope = seal_item(key, &copy_id, &copy)?;
    tx.upsert_item_row(&ItemRow {
        id: copy_id.clone(),
        envelope: Some(envelope.to_json()),
        revision: 0,
        deleted: false,
        dirty: true,
        updated_at: copy.updated_at(),
    })?;
    batch.touched.push(copy_id);
    Ok(())
}

// ---- push ---------------------------------------------------------------------------------

fn to_change(row: &ItemRow) -> Change {
    Change {
        id: row.id.clone(),
        base_revision: row.revision,
        deleted: row.deleted,
        envelope: row.envelope.clone().filter(|_| !row.deleted),
        updated_at: row.updated_at,
    }
}

async fn push_all(
    vault: &SharedVault,
    backend: &dyn SyncBackend,
    options: &SyncOptions,
    report: &mut SyncReport,
) -> Result<()> {
    // How often each item has been pushed this round; bounds retries and conflict ping-pong.
    let mut attempts: HashMap<String, u8> = HashMap::new();
    loop {
        let batch: Vec<ItemRow> = with_vault(vault, |v| {
            Ok(v.store
                .dirty_rows()?
                .into_iter()
                .filter(|r| attempts.get(&r.id).copied().unwrap_or(0) < MAX_PUSH_ATTEMPTS)
                .collect())
        })?;
        if batch.is_empty() {
            return Ok(());
        }
        for chunk in batch.chunks(MAX_CHANGES_PER_PUSH) {
            for row in chunk {
                *attempts.entry(row.id.clone()).or_default() += 1;
            }
            let results = backend.push(chunk.iter().map(to_change).collect()).await?;
            let given_up = with_vault(vault, |v| {
                let mut ctx = v.sync_ctx()?;
                apply_push_results(&mut ctx, chunk, &results, options, report)
            })?;
            for id in given_up {
                attempts.insert(id, MAX_PUSH_ATTEMPTS);
            }
        }
    }
}

/// Records the outcome of one pushed chunk. Returns ids that should not be retried this round.
fn apply_push_results(
    ctx: &mut SyncCtx<'_>,
    pushed: &[ItemRow],
    results: &[PushResult],
    options: &SyncOptions,
    report: &mut SyncReport,
) -> Result<Vec<String>> {
    let (key, now) = (ctx.key, ctx.now);
    let by_id: HashMap<&str, &ItemRow> = pushed.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut pushed_ok = 0u32;
    let mut failed = 0u32;
    let mut given_up = Vec::new();

    let batch = ctx.store.transaction(|tx| {
        let mut batch = Batch::default();
        for result in results {
            let Some(sent) = by_id.get(result.id()) else {
                // The server answered about something we did not send; ignore it.
                continue;
            };
            match result {
                PushResult::Ok { id, revision, .. } => {
                    // Clears `dirty` only if the row still holds exactly what was pushed.
                    tx.mark_pushed(
                        id,
                        *revision,
                        sent.envelope.as_deref().filter(|_| !sent.deleted),
                    )?;
                    pushed_ok += 1;
                }
                PushResult::Conflict { server, .. } => {
                    apply_remote(
                        tx,
                        key,
                        now,
                        &options.conflict_suffix,
                        server,
                        true,
                        &mut batch,
                    )?;
                }
                PushResult::Error { id, error } if error == "not_found" => {
                    // The server has no such row (for example its database was reset): resend as new.
                    tx.set_row_revision(id, 0)?;
                }
                PushResult::Error { id, error } => {
                    tracing::warn!(item_id = %id, error = %error, "server rejected a change");
                    failed += 1;
                    given_up.push(id.clone());
                }
            }
        }
        Ok(batch)
    })?;
    report.pushed += pushed_ok;
    report.failed += failed;
    finish_batch(ctx, batch, report)?;
    Ok(given_up)
}

#[cfg(test)]
mod tests {
    // The engine is exercised end-to-end against `FakeBackend` in `sync::tests`.
    use super::*;

    #[test]
    fn default_options_use_the_english_suffix() {
        assert_eq!(SyncOptions::default().conflict_suffix, " (conflict copy)");
    }

    #[test]
    fn report_serialises_for_the_shell() {
        let json = serde_json::to_value(SyncReport {
            pulled: 1,
            pushed: 2,
            ..SyncReport::default()
        })
        .unwrap();
        assert_eq!(json["pulled"], 1);
        assert_eq!(json["pushed"], 2);
        assert_eq!(json["pending_after"], 0);
    }
}
