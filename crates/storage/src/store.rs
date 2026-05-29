//! SQLite-backed event store for the storage node.
//!
//! Stores all events indefinitely and serves paginated history queries.
//! Events are keyed by their content hash and indexed by author/seq for
//! DAG-aware pagination.
//!
//! ## Durability
//!
//! On open, the store enables WAL journaling, `synchronous = FULL`, and
//! foreign-key enforcement to provide durability across crashes. WAL is
//! incompatible with in-memory databases, so it is skipped automatically
//! when the path is `:memory:`.
//!
//! ## Schema versioning
//!
//! All `CREATE TABLE` / `CREATE INDEX` statements live in the [`MIGRATIONS`]
//! slice. On open, the store records applied migration versions in a
//! `schema_version` table and applies any pending migrations inside a
//! transaction. New migrations should be appended to [`MIGRATIONS`] and
//! never reordered or rewritten so existing databases stay consistent.

use rusqlite::{params, Connection};
use willow_common::{MAX_AUTHORS_PER_SYNC, SYNC_BATCH_LIMIT};
use willow_identity::EndpointId;
use willow_state::{Event, EventKind, HeadsSummary};

/// Ordered list of schema migrations. Each entry is run inside its own
/// transaction the first time the database is opened. Once a migration is
/// shipped, never edit or reorder it — only append new entries.
const MIGRATIONS: &[&str] = &[
    // Migration 1: initial schema (the events table and its indexes).
    "CREATE TABLE IF NOT EXISTS events (
        hash BLOB PRIMARY KEY,
        server_id TEXT NOT NULL,
        channel_id TEXT NOT NULL DEFAULT '',
        author BLOB NOT NULL,
        seq INTEGER NOT NULL,
        timestamp_hint_ms INTEGER NOT NULL,
        event_data BLOB NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_events_server ON events(server_id);
    CREATE INDEX IF NOT EXISTS idx_events_channel ON events(server_id, channel_id);
    CREATE INDEX IF NOT EXISTS idx_events_author_seq ON events(author, seq);",
    // Migration 2: server-prefixed (server_id, author, seq) index so the
    // restructured `sync_since` resolves every `(author = ? AND seq > ?)`
    // disjunct as a per-(server, author) range scan instead of a server scan.
    // The legacy `idx_events_author_seq` is intentionally NOT dropped here:
    // dropping it is a separate, later migration so a rollout stays reversible
    // (see docs/specs/2026-04-24-negentropy-sync.md § Storage requirements).
    "CREATE INDEX IF NOT EXISTS idx_events_server_author_seq
        ON events(server_id, author, seq);",
];

/// SQLite-backed event store.
pub struct StorageEventStore {
    conn: Connection,
}

impl StorageEventStore {
    /// Open or create the database at `path`.
    ///
    /// Enables WAL journaling (skipped for `:memory:` databases),
    /// `synchronous = FULL`, and foreign-key enforcement, then runs any
    /// pending schema migrations.
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let is_memory = path == ":memory:";
        let conn = if is_memory {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };

        // Durability pragmas. WAL is a no-op for in-memory databases and
        // SQLite ignores the request — skip it explicitly so we can assert
        // on the resulting journal_mode in tests without surprises.
        if !is_memory {
            conn.pragma_update(None, "journal_mode", "WAL")?;
        }
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let store = Self { conn };
        store.run_migrations()?;
        Ok(store)
    }

    /// Apply any migrations from [`MIGRATIONS`] that haven't been recorded
    /// yet. Each pending migration runs inside its own transaction along
    /// with the `schema_version` insert, so a crash mid-migration leaves the
    /// database either fully migrated or untouched.
    fn run_migrations(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at_ms INTEGER NOT NULL
            );",
        )?;

        let current: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        for (idx, sql) in MIGRATIONS.iter().enumerate() {
            let version = (idx + 1) as i64;
            if version <= current {
                continue;
            }
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            // Propagate clock errors instead of recording `applied_at_ms = 0`
            // when the system clock is somehow set before 1970. The rest of
            // this crate returns `anyhow::Result`, so surface the error too.
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(anyhow::Error::from)?
                .as_millis() as i64;
            tx.execute(
                "INSERT INTO schema_version (version, applied_at_ms) VALUES (?1, ?2)",
                params![version, now_ms],
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    /// Store an event. Deduplicates by event hash. Returns true if inserted.
    ///
    /// Cross-call duplicates (same hash already in the DB) are silently
    /// ignored and reported as `Ok(false)`. For higher throughput, batch
    /// many events into a single [`Self::store_events`] call.
    pub fn store_event(&self, server_id: &str, event: &Event) -> anyhow::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let inserted = insert_event_or_ignore(&tx, server_id, event)?;
        tx.commit()?;
        Ok(inserted)
    }

    /// Store a batch of events atomically inside a single transaction.
    ///
    /// Returns the number of rows actually inserted; duplicates already
    /// present in the database are silently ignored. The whole batch is
    /// committed at once, so a single fsync flushes every event in the
    /// batch — this is much faster than calling [`Self::store_event`] in a
    /// loop.
    ///
    /// If any insert fails (for example, two events in the *same* batch
    /// share a primary key, or another constraint is violated), the entire
    /// transaction is rolled back and no events from the batch are
    /// persisted. To deduplicate across calls only — and roll back the
    /// batch on intra-batch duplicates — pass each event at most once.
    pub fn store_events(&self, events: &[(String, Event)]) -> anyhow::Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.unchecked_transaction()?;
        let mut inserted = 0usize;
        {
            // Plain INSERT (no OR IGNORE): a duplicate hash inside the
            // batch raises a UNIQUE constraint error and rolls back the
            // whole transaction. Cross-call duplicates are still tolerated
            // via the per-row helper used by `store_event`, which uses
            // INSERT OR IGNORE for that case.
            let mut stmt = tx.prepare(
                "INSERT INTO events (hash, server_id, channel_id, author, seq, timestamp_hint_ms, event_data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (server_id, event) in events {
                let channel_id = extract_channel_id(event);
                let event_data = bincode::serialize(event)?;
                let rows = stmt.execute(params![
                    event.hash.0.as_slice(),
                    server_id,
                    channel_id,
                    event.author.as_bytes(),
                    event.seq as i64,
                    event.timestamp_hint_ms as i64,
                    event_data,
                ])?;
                inserted += rows;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Query events for a server, optionally filtered by channel, paginated
    /// by HeadsSummary (events before the given heads).
    ///
    /// Returns events whose (author, seq) pairs are before the given heads,
    /// limited to `limit`. The second return value indicates whether more
    /// pages exist.
    pub fn history(
        &self,
        server_id: &str,
        channel: Option<&str>,
        before: Option<&HeadsSummary>,
        limit: u32,
    ) -> anyhow::Result<(Vec<Event>, bool)> {
        // Reject peer-supplied cursors that would balloon SQL construction
        // (see `MAX_AUTHORS_PER_SYNC`). Done before any allocation or SQL
        // building so a hostile request fails fast.
        if let Some(heads) = before {
            if heads.heads.len() > MAX_AUTHORS_PER_SYNC {
                anyhow::bail!(
                    "too many heads in history cursor: {} > {}",
                    heads.heads.len(),
                    MAX_AUTHORS_PER_SYNC
                );
            }
        }

        let capped = (limit as usize).min(SYNC_BATCH_LIMIT);
        let fetch_limit = capped + 1;

        // Build the query dynamically based on filters.
        let mut sql = String::from("SELECT event_data FROM events WHERE server_id = ?1");
        let mut param_idx = 2;

        // Channel filter
        let channel_param: String;
        if let Some(ch) = channel {
            sql.push_str(&format!(" AND channel_id = ?{param_idx}"));
            channel_param = ch.to_string();
            param_idx += 1;
        } else {
            channel_param = String::new();
        }

        // HeadsSummary cursor: exclude events at or after the given heads.
        // For each author in the cursor, only include events with seq < their_seq.
        // For authors NOT in the cursor, include all events.
        // All values are parameterized to prevent SQL injection.
        let mut extra_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(heads) = before {
            if !heads.heads.is_empty() {
                let mut conditions = Vec::new();
                let mut author_placeholders = Vec::new();
                for (author, head) in &heads.heads {
                    conditions.push(format!(
                        "(author = ?{} AND seq < ?{})",
                        param_idx,
                        param_idx + 1
                    ));
                    extra_params.push(Box::new(author.as_bytes().to_vec()));
                    extra_params.push(Box::new(head.seq as i64));
                    author_placeholders.push(format!("?{param_idx}"));
                    param_idx += 2;
                }
                let known_authors = author_placeholders.join(", ");
                let cond = format!(
                    " AND (({}) OR author NOT IN ({}))",
                    conditions.join(" OR "),
                    known_authors,
                );
                sql.push_str(&cond);
            }
        }

        sql.push_str(&format!(" ORDER BY seq DESC, hash ASC LIMIT ?{param_idx}"));

        let mut stmt = self.conn.prepare(&sql)?;

        // Build the full parameter list dynamically.
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.push(Box::new(server_id.to_string()));
        if channel.is_some() {
            all_params.push(Box::new(channel_param.clone()));
        }
        all_params.extend(extra_params);
        all_params.push(Box::new(fetch_limit as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| &**p).collect();
        let raw_rows: Vec<Vec<u8>> = stmt
            .query_map(param_refs.as_slice(), |row| row.get(0))?
            .filter_map(|r| match r {
                Ok(data) => Some(data),
                Err(e) => {
                    tracing::warn!("failed to read event row: {e}");
                    None
                }
            })
            .collect();

        let events: Vec<Event> = raw_rows
            .iter()
            .filter_map(|data| match bincode::deserialize(data) {
                Ok(e) => Some(e),
                Err(e) => {
                    tracing::warn!("corrupt event data, skipping: {e}");
                    None
                }
            })
            .collect();

        let has_more = events.len() > capped;
        let events: Vec<Event> = events.into_iter().take(capped).collect();

        Ok((events, has_more))
    }

    /// Return events the requester is missing based on their HeadsSummary.
    ///
    /// For each author in our store: if the requester has a lower seq (or
    /// doesn't know the author at all), return events with seq > their_seq.
    /// If heads is empty, returns all events for the server.
    ///
    /// The SQL `LIMIT willow_common::SYNC_BATCH_LIMIT` is an **OOM guard** on
    /// the materialized row set, not the wire bound: the caller
    /// ([`StorageRole::handle_request`](crate::role)) byte-budgets this delta
    /// with `pack_sync_batches` / `SYNC_ENVELOPE_BUDGET` and serves only the
    /// first budget-fitting batch, so the authoritative per-envelope bound is
    /// the 64 KiB gossip ceiling (see
    /// `docs/specs/2026-04-24-negentropy-sync.md` § Wire protocol).
    pub fn sync_since(&self, server_id: &str, heads: &HeadsSummary) -> anyhow::Result<Vec<Event>> {
        // Reject peer-supplied summaries that would balloon SQL construction
        // (see `MAX_AUTHORS_PER_SYNC`). Done before any allocation or SQL
        // building so a hostile request fails fast.
        if heads.heads.len() > MAX_AUTHORS_PER_SYNC {
            anyhow::bail!(
                "too many heads in sync request: {} > {}",
                heads.heads.len(),
                MAX_AUTHORS_PER_SYNC
            );
        }

        if heads.heads.is_empty() {
            // New peer — send up to SYNC_BATCH_LIMIT events for this server.
            let mut stmt = self.conn.prepare(
                "SELECT event_data FROM events WHERE server_id = ?1 ORDER BY seq ASC LIMIT ?2",
            )?;
            let rows: Vec<Vec<u8>> = stmt
                .query_map(
                    rusqlite::params![server_id, SYNC_BATCH_LIMIT as i64],
                    |row| row.get(0),
                )?
                .filter_map(|r| match r {
                    Ok(data) => Some(data),
                    Err(e) => {
                        tracing::warn!("failed to read event row in sync_since: {e}");
                        None
                    }
                })
                .collect();
            let events: Vec<Event> = rows
                .iter()
                .filter_map(|data| match bincode::deserialize(data) {
                    Ok(e) => Some(e),
                    Err(e) => {
                        tracing::warn!("corrupt event data in sync_since, skipping: {e}");
                        None
                    }
                })
                .collect();
            return Ok(events);
        }

        // Build a parameterized query for events after the requester's known
        // heads. Every disjunct is a per-author range predicate
        // `(author = ? AND seq > ?)`:
        //   - authors the requester mentioned use their advertised seq;
        //   - authors we hold locally but the requester never mentioned default
        //     to seq 0 (the requester has nothing for them).
        // Enumerating the unmentioned authors explicitly — rather than the old
        // `author NOT IN (...)` fanout — lets the planner seek each disjunct on
        // the `(server_id, author, seq)` index (`idx_events_server_author_seq`,
        // migration 2) instead of falling back to a server scan with an in-list
        // negation. See docs/specs/2026-04-24-negentropy-sync.md
        // § Storage requirements.
        let mut sql = String::from("SELECT event_data FROM events WHERE server_id = ?1 AND (");
        let mut param_idx = 2u32;
        let mut extra_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut conditions = Vec::new();

        // Mentioned authors: seq > their advertised seq.
        for (author, head) in &heads.heads {
            conditions.push(format!(
                "(author = ?{} AND seq > ?{})",
                param_idx,
                param_idx + 1
            ));
            extra_params.push(Box::new(author.as_bytes().to_vec()));
            extra_params.push(Box::new(head.seq as i64));
            param_idx += 2;
        }

        // Locally-known-but-unmentioned authors: seq > 0 (i.e. everything).
        // Resolved up front via `known_authors` so each becomes its own
        // per-(server, author) range scan rather than an in-list negation.
        for author in self.known_authors(server_id)? {
            if heads.heads.contains_key(&author) {
                continue;
            }
            conditions.push(format!("(author = ?{param_idx} AND seq > 0)"));
            extra_params.push(Box::new(author.as_bytes().to_vec()));
            param_idx += 1;
        }

        // No authors to serve at all → empty delta (the requester is up to date
        // on everything we hold). Bail before issuing an `AND ()` query.
        if conditions.is_empty() {
            return Ok(Vec::new());
        }

        sql.push_str(&conditions.join(" OR "));
        sql.push_str(&format!(") ORDER BY seq ASC LIMIT {SYNC_BATCH_LIMIT}"));

        let mut stmt = self.conn.prepare(&sql)?;

        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.push(Box::new(server_id.to_string()));
        all_params.extend(extra_params);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| &**p).collect();

        let rows: Vec<Vec<u8>> = stmt
            .query_map(param_refs.as_slice(), |row| row.get(0))?
            .filter_map(|r| match r {
                Ok(data) => Some(data),
                Err(e) => {
                    tracing::warn!("failed to read event row in sync_since: {e}");
                    None
                }
            })
            .collect();
        let events: Vec<Event> = rows
            .iter()
            .filter_map(|data| match bincode::deserialize(data) {
                Ok(e) => Some(e),
                Err(e) => {
                    tracing::warn!("corrupt event data in sync_since, skipping: {e}");
                    None
                }
            })
            .collect();

        Ok(events)
    }

    /// Distinct authors with at least one stored event for `server_id`.
    ///
    /// Used by [`Self::sync_since`] to enumerate locally-known-but-unmentioned
    /// authors up front (so each becomes a per-author range scan), and by a
    /// sync responder to serve authors the requester never advertised. Backed
    /// by `idx_events_server_author_seq` (migration 2). Author byte-blobs that
    /// are not a valid 32-byte endpoint id are skipped with a warning rather
    /// than failing the whole call.
    pub fn known_authors(&self, server_id: &str) -> anyhow::Result<Vec<EndpointId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT author FROM events WHERE server_id = ?1")?;
        let rows: Vec<Vec<u8>> = stmt
            .query_map(params![server_id], |row| row.get(0))?
            .filter_map(|r| match r {
                Ok(bytes) => Some(bytes),
                Err(e) => {
                    tracing::warn!("failed to read author row in known_authors: {e}");
                    None
                }
            })
            .collect();

        let authors = rows
            .into_iter()
            .filter_map(|bytes| match <[u8; 32]>::try_from(bytes.as_slice()) {
                Ok(arr) => match EndpointId::from_bytes(&arr) {
                    Ok(id) => Some(id),
                    Err(e) => {
                        tracing::warn!("invalid author key in known_authors: {e}");
                        None
                    }
                },
                Err(_) => {
                    tracing::warn!(
                        len = bytes.len(),
                        "author blob is not 32 bytes; skipping in known_authors"
                    );
                    None
                }
            })
            .collect();
        Ok(authors)
    }

    /// Total number of stored events.
    pub fn count(&self) -> anyhow::Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    /// Total disk usage estimate in bytes.
    pub fn disk_usage_bytes(&self) -> anyhow::Result<u64> {
        let pages: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let page_size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))?;
        let pages = u64::try_from(pages).unwrap_or(0);
        let page_size = u64::try_from(page_size).unwrap_or(0);
        Ok(pages.saturating_mul(page_size))
    }

    /// Number of distinct servers tracked.
    ///
    /// Return type is capped at `u32` to match `WorkerRoleInfo::Storage.servers_tracked`.
    /// A distinct-server count exceeding `u32::MAX` is astronomically unlikely and
    /// is reported as `u32::MAX` with a warning.
    pub fn server_count(&self) -> anyhow::Result<u32> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(DISTINCT server_id) FROM events", [], |row| {
                    row.get(0)
                })?;
        Ok(u32::try_from(count).unwrap_or_else(|_| {
            tracing::warn!(
                count,
                "server_count exceeds u32::MAX; clamping (widen servers_tracked upstream)"
            );
            u32::MAX
        }))
    }
}

/// Insert one event into the open transaction, ignoring rows whose hash
/// already exists. Returns `true` if a row was actually inserted.
fn insert_event_or_ignore(
    tx: &rusqlite::Transaction<'_>,
    server_id: &str,
    event: &Event,
) -> anyhow::Result<bool> {
    let channel_id = extract_channel_id(event);
    let event_data = bincode::serialize(event)?;
    let rows = tx.execute(
        "INSERT OR IGNORE INTO events (hash, server_id, channel_id, author, seq, timestamp_hint_ms, event_data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event.hash.0.as_slice(),
            server_id,
            channel_id,
            event.author.as_bytes(),
            event.seq as i64,
            event.timestamp_hint_ms as i64,
            event_data,
        ],
    )?;
    Ok(rows > 0)
}

/// Extract channel_id from an event, if applicable.
fn extract_channel_id(event: &Event) -> String {
    match &event.kind {
        EventKind::Message { channel_id, .. }
        | EventKind::PinMessage { channel_id, .. }
        | EventKind::UnpinMessage { channel_id, .. }
        | EventKind::CreateChannel { channel_id, .. }
        | EventKind::DeleteChannel { channel_id, .. }
        | EventKind::RenameChannel { channel_id, .. }
        | EventKind::RotateChannelKey { channel_id, .. } => channel_id.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;
    use willow_state::{EventHash, EventKind};

    fn make_message(id: &Identity, seq: u64, prev: EventHash, channel: &str) -> Event {
        Event::new(
            id,
            seq,
            prev,
            vec![],
            EventKind::Message {
                channel_id: channel.to_string(),
                body: format!("msg seq={seq}"),
                reply_to: None,
            },
            seq * 1000,
        )
    }

    fn setup_identity_and_genesis() -> (Identity, Event) {
        let id = Identity::generate();
        let genesis = Event::new(
            &id,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::CreateServer {
                name: "test".to_string(),
            },
            0,
        );
        (id, genesis)
    }

    #[test]
    fn store_and_count() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();
        let event = make_message(&id, 2, genesis.hash, "general");
        assert!(store.store_event("srv-1", &event).unwrap());
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn deduplicates_by_event_hash() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();
        let event = make_message(&id, 2, genesis.hash, "general");
        assert!(store.store_event("srv-1", &event).unwrap());
        assert!(!store.store_event("srv-1", &event).unwrap());
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn history_returns_newest_first() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        let mut prev = genesis.hash;
        for seq in 2..=6 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            store.store_event("srv-1", &e).unwrap();
        }

        let (events, has_more) = store.history("srv-1", Some("general"), None, 3).unwrap();
        assert_eq!(events.len(), 3);
        assert!(has_more);
        // Ordered by seq DESC
        assert_eq!(events[0].seq, 6);
        assert_eq!(events[1].seq, 5);
        assert_eq!(events[2].seq, 4);
    }

    #[test]
    fn history_caps_caller_limit_to_sync_batch_limit() {
        // Confirms history() silently caps `limit` at SYNC_BATCH_LIMIT so a
        // malicious client cannot request up to u32::MAX events. With fewer
        // rows stored than the cap, the return length matches the stored
        // rows and `has_more` is false.
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        let mut prev = genesis.hash;
        for seq in 2..=6 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            store.store_event("srv-1", &e).unwrap();
        }

        let (events, has_more) = store
            .history("srv-1", Some("general"), None, u32::MAX)
            .unwrap();
        assert_eq!(events.len(), 5);
        assert!(!has_more);
        assert!(events.len() <= SYNC_BATCH_LIMIT);
    }

    #[test]
    fn history_pagination_with_heads_cursor() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        let mut prev = genesis.hash;
        let mut event_hashes = vec![];
        for seq in 2..=11 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            event_hashes.push((seq, e.hash));
            store.store_event("srv-1", &e).unwrap();
        }

        // First page: no cursor.
        let (page1, has_more1) = store.history("srv-1", Some("general"), None, 3).unwrap();
        assert_eq!(page1.len(), 3);
        assert!(has_more1);
        assert_eq!(page1[0].seq, 11);

        // Second page: cursor at the last event of page 1.
        let last_seq = page1.last().unwrap().seq;
        let last_hash = page1.last().unwrap().hash;
        let mut cursor_heads = std::collections::BTreeMap::new();
        cursor_heads.insert(
            id.endpoint_id(),
            willow_state::AuthorHead {
                seq: last_seq,
                hash: last_hash,
            },
        );
        let cursor = HeadsSummary {
            heads: cursor_heads,
        };
        let (page2, _) = store
            .history("srv-1", Some("general"), Some(&cursor), 3)
            .unwrap();
        assert_eq!(page2.len(), 3);
        // All events in page2 should have seq < last_seq
        for e in &page2 {
            assert!(e.seq < last_seq);
        }
    }

    #[test]
    fn history_filters_by_channel() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        let e1 = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e1).unwrap();

        let e2 = make_message(&id, 3, e1.hash, "random");
        store.store_event("srv-1", &e2).unwrap();

        let e3 = make_message(&id, 4, e2.hash, "general");
        store.store_event("srv-1", &e3).unwrap();

        let (events, _) = store.history("srv-1", Some("general"), None, 10).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn history_all_channels() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        let e1 = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e1).unwrap();

        let e2 = make_message(&id, 3, e1.hash, "random");
        store.store_event("srv-1", &e2).unwrap();

        let (events, _) = store.history("srv-1", None, None, 10).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn history_filters_by_server() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        let e1 = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e1).unwrap();

        let e2 = make_message(&id, 3, e1.hash, "general");
        store.store_event("srv-2", &e2).unwrap();

        let (events, _) = store.history("srv-1", Some("general"), None, 10).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn server_count_tracks_distinct_servers() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        let e1 = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e1).unwrap();

        let e2 = make_message(&id, 3, e1.hash, "general");
        store.store_event("srv-2", &e2).unwrap();

        let e3 = make_message(&id, 4, e2.hash, "general");
        store.store_event("srv-1", &e3).unwrap();

        assert_eq!(store.server_count().unwrap(), 2);
    }

    #[test]
    fn disk_usage_returns_nonzero_after_insert() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        let e = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e).unwrap();
        assert!(store.disk_usage_bytes().unwrap() > 0);
    }

    #[test]
    fn empty_history_returns_no_events() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (events, has_more) = store.history("srv-1", Some("general"), None, 10).unwrap();
        assert!(events.is_empty());
        assert!(!has_more);
    }

    #[test]
    fn history_ordering_is_deterministic_regardless_of_insertion_order() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();

        // Two events with different timestamps but same seq.
        let e_a = Event::new(
            &id_a,
            2,
            EventHash::ZERO,
            vec![],
            EventKind::Message {
                channel_id: "general".into(),
                body: "from A".into(),
                reply_to: None,
            },
            5000, // higher timestamp
        );
        let e_b = Event::new(
            &id_b,
            2,
            EventHash::ZERO,
            vec![],
            EventKind::Message {
                channel_id: "general".into(),
                body: "from B".into(),
                reply_to: None,
            },
            3000, // lower timestamp
        );

        // Store 1: insert A then B.
        let store1 = StorageEventStore::open(":memory:").unwrap();
        store1.store_event("srv-1", &e_a).unwrap();
        store1.store_event("srv-1", &e_b).unwrap();
        let (events1, _) = store1.history("srv-1", None, None, 10).unwrap();

        // Store 2: insert B then A.
        let store2 = StorageEventStore::open(":memory:").unwrap();
        store2.store_event("srv-1", &e_b).unwrap();
        store2.store_event("srv-1", &e_a).unwrap();
        let (events2, _) = store2.history("srv-1", None, None, 10).unwrap();

        assert_eq!(events1.len(), 2);
        assert_eq!(events2.len(), 2);
        // Both stores must return events in the same order.
        assert_eq!(
            events1[0].hash, events2[0].hash,
            "ordering should be deterministic regardless of insertion order"
        );
        assert_eq!(events1[1].hash, events2[1].hash);
    }

    #[test]
    fn corrupt_event_data_does_not_panic() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        // Insert a valid event.
        let e = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e).unwrap();

        // Manually insert corrupt binary data.
        store
            .conn
            .execute(
                "INSERT INTO events (hash, server_id, channel_id, author, seq, timestamp_hint_ms, event_data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    &[0xFFu8; 32] as &[u8],
                    "srv-1",
                    "general",
                    id.endpoint_id().as_bytes(),
                    99_i64,
                    0_i64,
                    &[0xDEu8, 0xAD] as &[u8],
                ],
            )
            .unwrap();

        // history() should not panic and should return only the valid event.
        let (events, _) = store.history("srv-1", None, None, 100).unwrap();
        assert_eq!(
            events.len(),
            1,
            "should skip corrupt event and return only valid one"
        );
        assert_eq!(events[0].hash, e.hash);

        // sync_since() should also not panic.
        let synced = store.sync_since("srv-1", &HeadsSummary::default()).unwrap();
        assert_eq!(
            synced.len(),
            1,
            "sync_since should also skip corrupt events"
        );
    }

    // ---------------------------------------------------------------------
    // Durability + schema versioning tests
    // ---------------------------------------------------------------------

    /// Helper: open a fresh file-backed store inside a tempdir.
    fn open_file_store() -> (tempfile::TempDir, StorageEventStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.db");
        let store = StorageEventStore::open(path.to_str().unwrap()).unwrap();
        (dir, store)
    }

    #[test]
    fn wal_mode_is_enabled() {
        let (_dir, store) = open_file_store();
        let mode: String = store
            .conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[test]
    fn synchronous_full_is_enabled() {
        let (_dir, store) = open_file_store();
        let sync: i64 = store
            .conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        // SQLite reports synchronous as an integer: 0=OFF, 1=NORMAL, 2=FULL,
        // 3=EXTRA. FULL is what we asked for.
        assert_eq!(sync, 2);
    }

    #[test]
    fn foreign_keys_enabled() {
        let (_dir, store) = open_file_store();
        let fk: i64 = store
            .conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn schema_version_table_exists_after_open() {
        let (_dir, store) = open_file_store();
        let rows: Vec<(i64, i64)> = store
            .conn
            .prepare("SELECT version, applied_at_ms FROM schema_version ORDER BY version ASC")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(
            !rows.is_empty(),
            "schema_version table should be populated after open"
        );
        let versions: Vec<i64> = rows.iter().map(|(v, _)| *v).collect();
        assert!(
            versions.contains(&1),
            "migration 1 (initial schema) must be recorded"
        );
        // applied_at_ms must reflect the real clock, not the legacy `0`
        // fallback. Under a normal system clock (post-1970) it is strictly
        // positive. The pre-1970 branch now returns an error instead of
        // silently writing 0; testing that path would require time-mocking
        // infrastructure this crate doesn't have, which is out of scope.
        for (version, applied_at_ms) in &rows {
            assert!(
                *applied_at_ms > 0,
                "migration {version} recorded applied_at_ms={applied_at_ms}, \
                 expected a positive Unix-ms timestamp",
            );
        }
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.db");
        let path_str = path.to_str().unwrap();

        // Open once.
        {
            let _store = StorageEventStore::open(path_str).unwrap();
        }
        // Open again — must not duplicate version rows or error.
        let store = StorageEventStore::open(path_str).unwrap();

        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM schema_version WHERE version = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "reopening the database must not duplicate the schema_version row"
        );

        let total: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            total,
            MIGRATIONS.len() as i64,
            "schema_version row count must match the number of migrations"
        );
    }

    #[test]
    fn batched_store_inserts_all_rows() {
        let (_dir, store) = open_file_store();
        let (id, genesis) = setup_identity_and_genesis();

        let mut prev = genesis.hash;
        let mut batch = Vec::new();
        for seq in 2..=6 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            batch.push(("srv-1".to_string(), e));
        }

        let inserted = store.store_events(&batch).unwrap();
        assert_eq!(inserted, 5);
        assert_eq!(store.count().unwrap(), 5);
    }

    #[test]
    fn batched_store_is_atomic() {
        let (_dir, store) = open_file_store();
        let (id, genesis) = setup_identity_and_genesis();

        // Three valid events; the third one shares its primary key with the
        // first, which forces a UNIQUE-constraint violation mid-batch.
        let e1 = make_message(&id, 2, genesis.hash, "general");
        let e2 = make_message(&id, 3, e1.hash, "general");
        let dup = e1.clone();

        let batch = vec![
            ("srv-1".to_string(), e1),
            ("srv-1".to_string(), e2),
            ("srv-1".to_string(), dup),
        ];
        let result = store.store_events(&batch);
        assert!(
            result.is_err(),
            "duplicate primary key in the batch must surface an error"
        );

        // Critical: every row in the batch must be rolled back.
        assert_eq!(
            store.count().unwrap(),
            0,
            "failed batch must leave the database untouched"
        );
    }

    #[test]
    fn store_event_after_failed_batch_still_works() {
        // Sanity check that a failed batch doesn't poison the connection.
        let (_dir, store) = open_file_store();
        let (id, genesis) = setup_identity_and_genesis();

        let e1 = make_message(&id, 2, genesis.hash, "general");
        let dup = e1.clone();
        let _ = store.store_events(&[
            ("srv-1".to_string(), e1.clone()),
            ("srv-1".to_string(), dup),
        ]);
        assert_eq!(store.count().unwrap(), 0);

        // Subsequent single-event store should succeed.
        assert!(store.store_event("srv-1", &e1).unwrap());
        assert_eq!(store.count().unwrap(), 1);
    }

    // -------------------------------------------------------------------------
    // Round-trip equality
    // -------------------------------------------------------------------------

    /// Retrieving a stored event via `history` must return an event whose
    /// serialized bytes are identical to the original (full round-trip).
    #[test]
    fn stored_event_round_trips_with_full_equality() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();
        let original = make_message(&id, 2, genesis.hash, "general");

        store.store_event("srv-1", &original).unwrap();

        let (events, _) = store.history("srv-1", None, None, 10).unwrap();
        assert_eq!(events.len(), 1);

        let retrieved = &events[0];
        // Event does not implement PartialEq, so compare all public fields.
        assert_eq!(retrieved.hash, original.hash, "hash mismatch");
        assert_eq!(retrieved.author, original.author, "author mismatch");
        assert_eq!(retrieved.seq, original.seq, "seq mismatch");
        assert_eq!(retrieved.prev, original.prev, "prev mismatch");
        assert_eq!(
            retrieved.timestamp_hint_ms, original.timestamp_hint_ms,
            "timestamp_hint_ms mismatch"
        );
        // Verify bincode round-trip produces identical bytes.
        let orig_bytes = bincode::serialize(&original).unwrap();
        let ret_bytes = bincode::serialize(retrieved).unwrap();
        assert_eq!(
            orig_bytes, ret_bytes,
            "serialized bytes of retrieved event must match original"
        );
    }

    // -------------------------------------------------------------------------
    // sync_since tests
    // -------------------------------------------------------------------------

    /// `sync_since` with a non-empty HeadsSummary should return only events
    /// the remote peer is missing — i.e., events after the peer's known seq.
    #[test]
    fn sync_since_returns_delta_for_known_heads() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        // Store events e1..e5 (seq 2..=6 — seq 1 is genesis, not stored).
        let mut prev = genesis.hash;
        let mut events = Vec::new();
        for seq in 2..=6 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            store.store_event("srv-1", &e).unwrap();
            events.push(e);
        }

        // The remote peer knows events up to seq=4 (the third stored event,
        // i.e., e1=seq2, e2=seq3, e3=seq4).
        let e3 = &events[2]; // seq=4
        let mut heads_map = std::collections::BTreeMap::new();
        heads_map.insert(
            id.endpoint_id(),
            willow_state::AuthorHead {
                seq: e3.seq,
                hash: e3.hash,
            },
        );
        let remote_heads = HeadsSummary { heads: heads_map };

        let delta = store.sync_since("srv-1", &remote_heads).unwrap();

        // Should return only seq=5 and seq=6.
        assert_eq!(
            delta.len(),
            2,
            "expected exactly 2 delta events (seq 5 and 6), got {}",
            delta.len()
        );
        let seqs: Vec<u64> = delta.iter().map(|e| e.seq).collect();
        assert!(
            seqs.contains(&5),
            "delta must contain seq=5; got seqs: {seqs:?}"
        );
        assert!(
            seqs.contains(&6),
            "delta must contain seq=6; got seqs: {seqs:?}"
        );
        // Confirm the early events are NOT in the delta.
        for e in &delta {
            assert!(
                e.seq > 4,
                "delta must not include events the peer already knows (seq={})",
                e.seq
            );
        }
    }

    /// `sync_since` for a server ID with no stored events must return an
    /// empty Vec, not an error.
    #[test]
    fn sync_since_unknown_server_returns_empty() {
        let store = StorageEventStore::open(":memory:").unwrap();

        // Don't store anything — query a completely unknown server.
        let delta = store
            .sync_since("nonexistent-server", &HeadsSummary::default())
            .unwrap();
        assert!(
            delta.is_empty(),
            "sync_since for an unknown server must return empty, not error"
        );
    }

    /// `sync_since` with an empty HeadsSummary (new peer) returns all events.
    #[test]
    fn sync_since_empty_heads_returns_all_events() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis();

        let mut prev = genesis.hash;
        for seq in 2..=5 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            store.store_event("srv-1", &e).unwrap();
        }

        let all = store.sync_since("srv-1", &HeadsSummary::default()).unwrap();
        assert_eq!(
            all.len(),
            4,
            "empty heads should return all 4 stored events"
        );
    }

    // -------------------------------------------------------------------------
    // Durability: persist → drop → reopen
    // -------------------------------------------------------------------------

    /// Events written to a file-backed store must survive a close/reopen cycle.
    #[test]
    fn events_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("durability.db");
        let path_str = path.to_str().unwrap();

        let (id, genesis) = setup_identity_and_genesis();

        // Create three events and remember the first one for full-equality check.
        let e1 = make_message(&id, 2, genesis.hash, "general");
        let e2 = make_message(&id, 3, e1.hash, "general");
        let e3 = make_message(&id, 4, e2.hash, "general");
        let original_e1 = e1.clone();

        // Store in first store instance, then drop it (closes the connection).
        {
            let store = StorageEventStore::open(path_str).unwrap();
            store.store_event("srv-1", &e1).unwrap();
            store.store_event("srv-1", &e2).unwrap();
            store.store_event("srv-1", &e3).unwrap();
        }

        // Reopen from the same file path.
        let store2 = StorageEventStore::open(path_str).unwrap();
        let (events, _) = store2.history("srv-1", None, None, 10).unwrap();

        assert_eq!(events.len(), 3, "all 3 events must survive the reopen");

        // history() returns newest-first (ORDER BY seq DESC), so the last
        // element is the oldest (e1 = seq 2).
        let recovered_e1 = events.iter().find(|e| e.seq == 2).unwrap();
        // Event does not implement PartialEq — compare all public fields.
        assert_eq!(
            recovered_e1.hash, original_e1.hash,
            "hash mismatch after reopen"
        );
        assert_eq!(
            recovered_e1.author, original_e1.author,
            "author mismatch after reopen"
        );
        assert_eq!(
            recovered_e1.seq, original_e1.seq,
            "seq mismatch after reopen"
        );
        assert_eq!(
            recovered_e1.prev, original_e1.prev,
            "prev mismatch after reopen"
        );
        assert_eq!(
            recovered_e1.timestamp_hint_ms, original_e1.timestamp_hint_ms,
            "timestamp_hint_ms mismatch after reopen"
        );
        // Full byte-level equality via bincode.
        let orig_bytes = bincode::serialize(&original_e1).unwrap();
        let rec_bytes = bincode::serialize(recovered_e1).unwrap();
        assert_eq!(
            orig_bytes, rec_bytes,
            "serialized bytes of recovered event must match original after reopen"
        );
    }

    /// Build a `HeadsSummary` with `n` distinct random authors for cap tests.
    fn heads_summary_with_authors(n: usize) -> HeadsSummary {
        use willow_state::AuthorHead;
        let mut heads = std::collections::BTreeMap::new();
        for _ in 0..n {
            let id = Identity::generate();
            heads.insert(
                id.endpoint_id(),
                AuthorHead {
                    seq: 1,
                    hash: EventHash::ZERO,
                },
            );
        }
        HeadsSummary { heads }
    }

    /// A peer-supplied `HeadsSummary` with more than `MAX_AUTHORS_PER_SYNC`
    /// entries must be rejected by `sync_since` before any SQL is built.
    /// Without the cap, the rusqlite bind-parameter limit (default 32766) or
    /// CPU spent compiling the giant prepared statement is the failure mode.
    #[test]
    fn sync_since_rejects_oversize_heads() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let oversize = heads_summary_with_authors(MAX_AUTHORS_PER_SYNC + 1);

        let err = store
            .sync_since("srv-1", &oversize)
            .expect_err("oversize heads must be rejected, not processed");
        let msg = format!("{err}");
        assert!(
            msg.contains("too many heads"),
            "error message should mention the cap; got: {msg}"
        );
    }

    /// `sync_since` must accept exactly `MAX_AUTHORS_PER_SYNC` entries — the
    /// cap is inclusive on the legal side.
    #[test]
    fn sync_since_accepts_exact_cap() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let at_cap = heads_summary_with_authors(MAX_AUTHORS_PER_SYNC);
        // No events stored, so the result is empty — but it must not error.
        let events = store
            .sync_since("srv-1", &at_cap)
            .expect("exactly MAX_AUTHORS_PER_SYNC entries must be accepted");
        assert!(events.is_empty());
    }

    /// A peer-supplied `before` cursor with more than `MAX_AUTHORS_PER_SYNC`
    /// entries must be rejected by `history` before any SQL is built.
    #[test]
    fn history_rejects_oversize_before_cursor() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let oversize = heads_summary_with_authors(MAX_AUTHORS_PER_SYNC + 1);

        let err = store
            .history("srv-1", None, Some(&oversize), 10)
            .expect_err("oversize cursor must be rejected, not processed");
        let msg = format!("{err}");
        assert!(
            msg.contains("too many heads"),
            "error message should mention the cap; got: {msg}"
        );
    }

    /// `history` must accept exactly `MAX_AUTHORS_PER_SYNC` cursor entries.
    #[test]
    fn history_accepts_exact_cap_cursor() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let at_cap = heads_summary_with_authors(MAX_AUTHORS_PER_SYNC);
        let (events, has_more) = store
            .history("srv-1", None, Some(&at_cap), 10)
            .expect("exactly MAX_AUTHORS_PER_SYNC entries must be accepted");
        assert!(events.is_empty());
        assert!(!has_more);
    }

    // -------------------------------------------------------------------------
    // known_authors + per-author sync_since restructure
    // -------------------------------------------------------------------------

    /// `known_authors` returns the distinct set of authors with stored events
    /// for a given server, and nothing from other servers.
    #[test]
    fn known_authors_returns_distinct_authors_per_server() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let other = Identity::generate();

        // Two authors on srv-1, each with two events.
        for (id, srv) in [(&id_a, "srv-1"), (&id_b, "srv-1")] {
            let e1 = make_message(id, 2, EventHash::ZERO, "general");
            let e2 = make_message(id, 3, e1.hash, "general");
            store.store_event(srv, &e1).unwrap();
            store.store_event(srv, &e2).unwrap();
        }
        // A different server must not leak into srv-1's author set.
        let oe = make_message(&other, 2, EventHash::ZERO, "general");
        store.store_event("srv-2", &oe).unwrap();

        let mut got = store.known_authors("srv-1").unwrap();
        got.sort();
        let mut want = vec![id_a.endpoint_id(), id_b.endpoint_id()];
        want.sort();
        assert_eq!(got, want);

        assert_eq!(
            store.known_authors("srv-2").unwrap(),
            vec![other.endpoint_id()]
        );
        assert!(store.known_authors("nonexistent").unwrap().is_empty());
    }

    /// The restructured `sync_since` (per-author `(author = ? AND seq > ?)`
    /// range scans for both mentioned and locally-known-but-unmentioned authors)
    /// must return exactly the same delta as a manual reference computation:
    /// for each author, every stored event with `seq > requester_seq` (0 if the
    /// requester never mentioned that author).
    #[test]
    fn sync_since_serves_unmentioned_authors_via_per_author_scan() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let id_a = Identity::generate();
        let id_b = Identity::generate();

        // Author A: seq 2..=5 ; Author B: seq 2..=4.
        let mut a_prev = EventHash::ZERO;
        for seq in 2..=5 {
            let e = make_message(&id_a, seq, a_prev, "general");
            a_prev = e.hash;
            store.store_event("srv-1", &e).unwrap();
        }
        let mut b_prev = EventHash::ZERO;
        for seq in 2..=4 {
            let e = make_message(&id_b, seq, b_prev, "general");
            b_prev = e.hash;
            store.store_event("srv-1", &e).unwrap();
        }

        // Requester knows A up to seq 3 and has NEVER heard of B.
        let mut heads_map = std::collections::BTreeMap::new();
        heads_map.insert(
            id_a.endpoint_id(),
            willow_state::AuthorHead {
                seq: 3,
                hash: EventHash::ZERO,
            },
        );
        let heads = HeadsSummary { heads: heads_map };

        let delta = store.sync_since("srv-1", &heads).unwrap();

        // Expected: A seq {4,5} (after the requester's seq 3) + all of B {2,3,4}
        // (unmentioned author defaults to seq 0).
        let mut a_seqs: Vec<u64> = delta
            .iter()
            .filter(|e| e.author == id_a.endpoint_id())
            .map(|e| e.seq)
            .collect();
        a_seqs.sort_unstable();
        let mut b_seqs: Vec<u64> = delta
            .iter()
            .filter(|e| e.author == id_b.endpoint_id())
            .map(|e| e.seq)
            .collect();
        b_seqs.sort_unstable();

        assert_eq!(a_seqs, vec![4, 5], "A: only events past requester's seq 3");
        assert_eq!(b_seqs, vec![2, 3, 4], "B: all events (requester unaware)");
        assert_eq!(delta.len(), 5);
    }

    /// The new compound index `idx_events_server_author_seq` must exist after a
    /// fresh open, and the legacy `idx_events_author_seq` must remain (the new
    /// migration is additive — the old index is dropped in a later, separate
    /// migration for rollout reversibility).
    #[test]
    fn server_author_seq_index_exists_and_legacy_index_retained() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let names: Vec<String> = store
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(
            names.iter().any(|n| n == "idx_events_server_author_seq"),
            "new compound index must exist; got {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "idx_events_author_seq"),
            "legacy index must be retained (dropped in a later migration); got {names:?}"
        );
    }
}
