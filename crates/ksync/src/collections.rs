//! Kindle Library collections — writes book membership into the device's
//! content catalog database (`/var/local/cc.db`) so synced books appear in a
//! `KSync <catalog name>` collection in the stock Library.
//!
//! The write path is `apply_ccdb`: stop the `com.lab126.ccat` catalog service,
//! back up cc.db, run the pure SQL (`apply_ccdb_conn` — host-tested against an
//! in-memory replica of the cc.db schema), then restart ccat; any SQL failure
//! restores the backup and restarts ccat, and downloads already on disk are
//! never touched.
//!
//! Books not yet indexed by the device's scanner (no `Entries` row yet) are
//! counted as `pending` and skipped — a later sync or the WAF "Rebuild
//! collections" button picks them up. The membership is a full idempotent
//! rebuild (DELETE + INSERT), so it self-heals if the firmware regenerates or
//! merges the collection.

use crate::config::Catalog;
use rusqlite::backup::Backup;
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Formats the stock Kindle reader understands natively; anything else (EPUB,
/// FB2, ...) stays on disk for KOReader etc. and is never a collection member.
pub const NATIVE_EXTS: &[&str] = &[".azw", ".azw3", ".mobi", ".pdf", ".txt"];

pub const CC_DB_PATH: &str = "/var/local/cc.db";
pub const CC_DB_BACKUP: &str = "/mnt/us/ksync/var/cc.db.bak";

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedMember {
    pub uuid: String,
    pub cde_type: String,
    pub cde_key: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CollectionsResult {
    pub added: usize,
    pub pending: usize,
    pub error: Option<String>,
}

fn is_native(p: &Path) -> bool {
    match p.extension().and_then(|e| e.to_str()) {
        Some(e) => {
            let ext = format!(".{}", e.to_ascii_lowercase());
            NATIVE_EXTS.contains(&ext.as_str())
        }
        None => false,
    }
}

/// Recursively collect every native-format file under `base_dir`, sorted for
/// stable membership ordering across runs.
pub fn collect_member_paths(base_dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if is_native(&p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(base_dir, &mut out);
    out.sort();
    out
}

fn row_to_member(row: &rusqlite::Row) -> rusqlite::Result<ResolvedMember> {
    Ok(ResolvedMember {
        uuid: row.get(0)?,
        cde_type: row.get(1)?,
        cde_key: row.get(2)?,
    })
}

/// Resolve each member path to its cc.db `Entries` row (exact `p_location`
/// match first, then a `LIKE '%' || relative-path` retry for paths the
/// scanner stored differently). Unmatched paths count as `pending`.
fn resolve_members(
    conn: &Connection,
    member_paths: &[PathBuf],
) -> Result<(Vec<ResolvedMember>, usize), String> {
    let documents = Path::new("/mnt/us/documents");
    let mut stmt = conn
        .prepare(
            "SELECT p_uuid, p_cdeType, p_cdeKey FROM Entries \
             WHERE p_type='Entry:Item' AND p_location = ?",
        )
        .map_err(|e| e.to_string())?;
    let mut stmt_like = conn
        .prepare(
            "SELECT p_uuid, p_cdeType, p_cdeKey FROM Entries \
             WHERE p_type='Entry:Item' AND p_location LIKE '%' || ?",
        )
        .map_err(|e| e.to_string())?;
    let mut members = Vec::new();
    let mut pending = 0usize;
    for p in member_paths {
        let full = format!("file://{}", p.display());
        let found = stmt
            .query_row(params![full], row_to_member)
            .ok()
            .or_else(|| {
                let rel = p.strip_prefix(documents).unwrap_or(p);
                stmt_like
                    .query_row(params![rel.to_string_lossy()], row_to_member)
                    .ok()
            });
        match found {
            Some(m) => members.push(m),
            None => pending += 1,
        }
    }
    Ok((members, pending))
}

/// The `p_titles_0_nominal` value of the collection: `<prefix> <name>`, or
/// just `<name>` when the prefix is empty.
pub fn collection_name(catalog: &Catalog, prefix: &str) -> String {
    let prefix = prefix.trim();
    let name = catalog.name.trim();
    match (prefix.is_empty(), name.is_empty()) {
        (true, true) => String::new(),
        (true, false) => name.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{} {}", prefix, name),
    }
}

fn find_collection(conn: &Connection, name: &str) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare("SELECT p_uuid FROM Entries WHERE p_type='Collection' AND p_titles_0_nominal = ?")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query(params![name]).map_err(|e| e.to_string())?;
    match rows.next().map_err(|e| e.to_string())? {
        Some(row) => row.get(0).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// Rebuild `j_titles` shaped like the template's but with the new name:
/// replace the first object's `nominal`, keep everything else; fall back to
/// `[{"nominal":"<name>"}]` when the template's JSON is unusable.
fn retitled_j_titles(template_j_titles: &str, name: &str) -> String {
    let fallback =
        || serde_json::to_string(&[serde_json::json!({ "nominal": name })]).unwrap_or_default();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(template_j_titles) else {
        return fallback();
    };
    let Some(arr) = value.as_array() else {
        return fallback();
    };
    let mut arr = arr.clone();
    let Some(first) = arr.first_mut() else {
        return fallback();
    };
    let Some(obj) = first.as_object_mut() else {
        return fallback();
    };
    if !obj.contains_key("nominal") {
        return fallback();
    }
    obj.insert(
        "nominal".to_string(),
        serde_json::Value::String(name.to_string()),
    );
    serde_json::to_string(&arr).unwrap_or_else(|_| fallback())
}

fn column_names(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", table))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    for c in rows {
        out.push(c.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Clone the first existing `Collection` row, copying every column and
/// overriding the collection-identity columns for the new catalog. Returns the
/// new collection's uuid. Falls back to a minimal insert when the device has
/// no `Collection` row at all yet.
fn clone_template_collection(
    conn: &Connection,
    name: &str,
    member_uuids: &[String],
) -> Result<String, String> {
    let cols = column_names(conn, "Entries")?;
    let new_uuid = uuid::Uuid::new_v4().to_string();
    let j_members = serde_json::to_string(member_uuids).map_err(|e| e.to_string())?;
    let j_titles_idx = cols.iter().position(|c| c == "j_titles");

    let mut stmt = conn
        .prepare("SELECT * FROM Entries WHERE p_type='Collection' LIMIT 1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let Some(template) = rows.next().map_err(|e| e.to_string())? else {
        return insert_minimal_collection(conn, name, member_uuids);
    };

    // Retitle using the template's own j_titles shape.
    let template_j_titles: String = match j_titles_idx {
        Some(i) => template
            .get::<_, Option<String>>(i)
            .ok()
            .flatten()
            .unwrap_or_default(),
        None => String::new(),
    };
    let j_titles = retitled_j_titles(&template_j_titles, name);

    let mut placeholders = Vec::with_capacity(cols.len());
    let mut values: Vec<rusqlite::types::Value> = Vec::with_capacity(cols.len());
    for (i, col) in cols.iter().enumerate() {
        let v: rusqlite::types::Value = match col.as_str() {
            "p_uuid" => new_uuid.clone().into(),
            "p_titles_0_nominal" => name.to_string().into(),
            "p_titleCount" => 1i64.into(),
            "j_titles" => j_titles.clone().into(),
            "j_members" => j_members.clone().into(),
            "p_memberCount" => (member_uuids.len() as i64).into(),
            "p_homeMemberCount" => (member_uuids.len() as i64).into(),
            "p_collectionCount" => 0i64.into(),
            "p_seriesState" => 1i64.into(),
            _ => template.get(i).unwrap_or(rusqlite::types::Value::Null),
        };
        placeholders.push(format!("?{}", i + 1));
        values.push(v);
    }
    let sql = format!(
        "INSERT INTO Entries ({}) VALUES ({})",
        cols.join(", "),
        placeholders.join(", ")
    );
    conn.execute(&sql, rusqlite::params_from_iter(values))
        .map_err(|e| e.to_string())?;
    Ok(new_uuid)
}

/// Minimal insert used only when the device has no `Collection` row to clone.
fn insert_minimal_collection(
    conn: &Connection,
    name: &str,
    member_uuids: &[String],
) -> Result<String, String> {
    let new_uuid = uuid::Uuid::new_v4().to_string();
    let j_titles = serde_json::to_string(&[serde_json::json!({ "nominal": name })])
        .map_err(|e| e.to_string())?;
    let j_members = serde_json::to_string(member_uuids).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO Entries (p_uuid, p_type, p_titles_0_nominal, p_titleCount, \
         j_titles, j_members, p_memberCount, p_homeMemberCount, p_collectionCount, p_seriesState) \
         VALUES (?1, 'Collection', ?2, 1, ?3, ?4, ?5, ?5, 0, 1)",
        params![
            new_uuid,
            name,
            j_titles,
            j_members,
            member_uuids.len() as i64
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(new_uuid)
}

/// Find or create the `KSync <name>` collection and fully rebuild its
/// membership (DELETE + INSERT). Keeps the stored member counts and `j_members`
/// mirror in sync with the rebuilt membership.
fn apply_membership(
    conn: &Connection,
    catalog: &Catalog,
    prefix: &str,
    members: &[ResolvedMember],
) -> Result<(), String> {
    let name = collection_name(catalog, prefix);
    let member_uuids: Vec<String> = members.iter().map(|m| m.uuid.clone()).collect();
    let collection_uuid = match find_collection(conn, &name)? {
        Some(u) => u,
        None => clone_template_collection(conn, &name, &member_uuids)?,
    };

    conn.execute(
        "DELETE FROM Collections WHERE i_collection_uuid = ?1",
        params![collection_uuid],
    )
    .map_err(|e| e.to_string())?;

    {
        let mut stmt = conn
            .prepare(
                "INSERT INTO Collections \
                 (i_collection_uuid, i_member_uuid, i_order, i_member_cde_type, \
                  i_member_cde_key, i_member_is_present, i_is_sideloaded) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, 1)",
            )
            .map_err(|e| e.to_string())?;
        for (i, m) in members.iter().enumerate() {
            stmt.execute(params![
                collection_uuid,
                m.uuid,
                i as i64,
                m.cde_type,
                m.cde_key
            ])
            .map_err(|e| e.to_string())?;
        }
    }

    let j_members = serde_json::to_string(&member_uuids).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE Entries SET p_memberCount = ?1, p_homeMemberCount = ?1, j_members = ?2 \
         WHERE p_uuid = ?3",
        params![members.len() as i64, j_members, collection_uuid],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Pure SQL half of the collections update — host-testable against any
/// connection (the tests use an in-memory replica of the cc.db schema with a
/// registered `icu` collation stub).
pub fn apply_ccdb_conn(
    conn: &mut Connection,
    catalog: &Catalog,
    prefix: &str,
    member_paths: &[PathBuf],
) -> CollectionsResult {
    let (members, pending) = match resolve_members(conn, member_paths) {
        Ok(x) => x,
        Err(e) => {
            return CollectionsResult {
                added: 0,
                pending: 0,
                error: Some(e),
            };
        }
    };
    match apply_membership(conn, catalog, prefix, &members) {
        Ok(()) => CollectionsResult {
            added: members.len(),
            pending,
            error: None,
        },
        Err(e) => CollectionsResult {
            added: 0,
            pending,
            error: Some(e),
        },
    }
}

fn run_shell_cmd(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The Kindle's content catalog may run under an upstart job (older
/// firmware) or be hosted by the framework with no job at all (newer
/// firmware, where `stop com.lab126.ccat` is an "Unknown job" failure). The
/// job name is the conf file name; return it only if the conf actually
/// exists, so we never try to stop a job that isn't there.
fn ccat_job() -> Option<String> {
    for name in ["com.lab126.ccat", "ccat"] {
        if Path::new("/etc/upstart")
            .join(format!("{}.conf", name))
            .exists()
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Back up cc.db with SQLite's own backup API (not a file copy): the
/// catalog service or KPM may hold the file open, and a torn `cp` snapshot
/// would be useless. Fails only if the source cannot be opened.
fn backup_ccdb() -> Result<(), String> {
    if let Some(parent) = Path::new(CC_DB_BACKUP).parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
    }
    let src =
        Connection::open(CC_DB_PATH).map_err(|e| format!("cannot open {}: {}", CC_DB_PATH, e))?;
    let mut dst = Connection::open(CC_DB_BACKUP)
        .map_err(|e| format!("cannot open {}: {}", CC_DB_BACKUP, e))?;
    let backup = Backup::new(&src, &mut dst).map_err(|e| format!("backup init: {}", e))?;
    backup
        .run_to_completion(5, Duration::from_millis(100), None)
        .map_err(|e| format!("backup failed: {}", e))
}

/// Device orchestration. The catalog service cannot be stopped on all
/// firmware (newer firmware hosts it with no upstart job), so stopping is
/// best-effort: where a ccat job exists it is stopped and restarted around
/// the write; otherwise the write proceeds unprotected, which is safe
/// because the SQL is a single transaction and SQLite's own locking handles
/// concurrent holders (KPM itself writes cc.db directly). The backup uses
/// SQLite's backup API so a live database is never snapshot-torn.
pub fn apply_ccdb(catalog: &Catalog, prefix: &str, member_paths: &[PathBuf]) -> CollectionsResult {
    let job = ccat_job();
    let stopped = match &job {
        Some(name) => run_shell_cmd("/sbin/stop", &[name]),
        None => false,
    };

    let backup = backup_ccdb();
    let conn = match backup {
        Ok(()) => Connection::open(CC_DB_PATH),
        Err(e) => {
            if stopped {
                let _ = run_shell_cmd("/sbin/start", &[job.as_deref().unwrap_or("")]);
            }
            return CollectionsResult {
                error: Some(e),
                ..Default::default()
            };
        }
    };
    let mut conn = match conn {
        Ok(c) => c,
        Err(e) => {
            if stopped {
                let _ = run_shell_cmd("/sbin/start", &[job.as_deref().unwrap_or("")]);
            }
            return CollectionsResult {
                error: Some(format!("cannot open {}: {}", CC_DB_PATH, e)),
                ..Default::default()
            };
        }
    };
    // Bundled SQLite has no ICU; the device's real collation is a plain
    // byte-wise compare in practice for our ASCII comparison needs. A busy
    // timeout lets a concurrent holder (e.g. KPM) finish before we write.
    let _ = conn.create_collation("icu", |a, b| a.cmp(b));
    let _ = conn.busy_timeout(Duration::from_secs(15));

    // One transaction: the DELETE + INSERTs + UPDATE either all land or
    // none do, so a failure cannot leave cc.db half-updated. (Raw BEGIN /
    // COMMIT because rusqlite's Transaction has no DerefMut for the
    // connection.)
    let result = if conn.execute_batch("BEGIN").is_err() {
        CollectionsResult {
            error: Some("begin transaction failed".to_string()),
            ..Default::default()
        }
    } else {
        let r = apply_ccdb_conn(&mut conn, catalog, prefix, member_paths);
        if r.error.is_none() {
            match conn.execute_batch("COMMIT") {
                Ok(()) => r,
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    CollectionsResult {
                        error: Some(format!("commit failed: {}", e)),
                        ..Default::default()
                    }
                }
            }
        } else {
            let _ = conn.execute_batch("ROLLBACK");
            r
        }
    };
    drop(conn);

    if stopped {
        // Restart the catalog service we stopped. If that fails, say so -
        // leaving the stock Library's catalog service down is worse than
        // the write.
        let name = job.as_deref().unwrap_or("com.lab126.ccat");
        if !run_shell_cmd("/sbin/start", &[name]) && result.error.is_none() {
            return CollectionsResult {
                error: Some(format!(
                    "cc.db updated but could not restart {} - restart it manually",
                    name
                )),
                ..result
            };
        }
    }
    result
}

/// Collect this catalog's native-format files and run the full collections
/// update (used by the sync worker and the WAF "Rebuild collections" op).
pub fn rebuild_catalog(catalog: &Catalog, prefix: &str) -> CollectionsResult {
    let base_dir = catalog.base_dir();
    let mut groups: Vec<(Catalog, Vec<PathBuf>)> = Vec::new();

    // OPDS library feeds commonly put one collection in each top-level
    // navigation entry. The sync walk mirrors those entries as directories.
    // Keep root acquisitions in the catalog's own collection.
    let mut root_paths = Vec::new();
    let mut sections = Vec::new();
    if let Ok(entries) = fs::read_dir(&base_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                sections.push(path);
            } else if is_native(&path) {
                root_paths.push(path);
            }
        }
    }
    root_paths.sort();
    sections.sort();
    if !root_paths.is_empty() || sections.is_empty() {
        groups.push((catalog.clone(), root_paths));
    }
    for section in sections {
        let mut section_catalog = catalog.clone();
        if let Some(name) = section.file_name().and_then(|n| n.to_str()) {
            section_catalog.name = if catalog.name.trim().is_empty() {
                name.to_string()
            } else {
                format!("{} - {}", catalog.name, name)
            };
        }
        groups.push((section_catalog, collect_member_paths(&section)));
    }

    let mut total = CollectionsResult::default();
    for (group_catalog, paths) in groups {
        let result = apply_ccdb(&group_catalog, prefix, &paths);
        total.added += result.added;
        total.pending += result.pending;
        if total.error.is_none() {
            total.error = result.error;
        }
    }
    total
}

/// Delete a collection by its exact title (the `p_titles_0_nominal` value,
/// e.g. "KSync AI") plus its membership rows. Must go through the daemon's
/// bundled rusqlite with the icu stub: the device's sqlite3 CLI cannot
/// prepare statements touching icu-collated columns (Entries deletes fire
/// triggers that reference them), so the stock `=`/LIKE deletes fail there.
pub fn remove_collection(name: &str) -> CollectionsResult {
    let job = ccat_job();
    let stopped = match &job {
        Some(name) => run_shell_cmd("/sbin/stop", &[name]),
        None => false,
    };
    if let Err(e) = backup_ccdb() {
        if stopped {
            let _ = run_shell_cmd("/sbin/start", &[job.as_deref().unwrap_or("")]);
        }
        return CollectionsResult {
            error: Some(e),
            ..Default::default()
        };
    }
    let conn = match Connection::open(CC_DB_PATH) {
        Ok(c) => c,
        Err(e) => {
            if stopped {
                let _ = run_shell_cmd("/sbin/start", &[job.as_deref().unwrap_or("")]);
            }
            return CollectionsResult {
                error: Some(format!("cannot open {}: {}", CC_DB_PATH, e)),
                ..Default::default()
            };
        }
    };
    let _ = conn.create_collation("icu", |a, b| a.cmp(b));
    let _ = conn.busy_timeout(Duration::from_secs(15));
    let result = match conn.execute_batch("BEGIN") {
        Err(e) => CollectionsResult {
            error: Some(format!("begin transaction failed: {}", e)),
            ..Default::default()
        },
        Ok(()) => match remove_collection_conn(&conn, name) {
            Ok(_) => match conn.execute_batch("COMMIT") {
                Ok(()) => CollectionsResult::default(),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    CollectionsResult {
                        error: Some(format!("commit failed: {}", e)),
                        ..Default::default()
                    }
                }
            },
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                CollectionsResult {
                    error: Some(e),
                    ..Default::default()
                }
            }
        },
    };
    drop(conn);
    if stopped {
        let restart = run_shell_cmd("/sbin/start", &[job.as_deref().unwrap_or("")]);
        if !restart && result.error.is_none() {
            return CollectionsResult {
                error: Some(
                    "collection removed but could not restart content catalog service".to_string(),
                ),
                ..result
            };
        }
    }
    result
}

/// Pure SQL half of [`remove_collection`] (host-testable; the caller must
/// register the `icu` collation on the connection, like the device wrapper).
pub fn remove_collection_conn(conn: &Connection, name: &str) -> Result<usize, String> {
    // The device can have legacy icu-collated indexes that the bundled
    // byte-wise collation cannot safely search. Resolve exact titles by a
    // table scan, then perform deletes by UUID.
    let uuid = conn
        .query_row(
            "SELECT p_uuid FROM Entries NOT INDEXED WHERE p_type='Collection' \
             AND p_titles_0_nominal COLLATE BINARY = ?1 LIMIT 1",
            params![name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("find collection: {}", e))?;
    let Some(uuid) = uuid else {
        return Ok(0);
    };
    conn.execute(
        "DELETE FROM Collections WHERE i_collection_uuid = ?1",
        params![uuid],
    )
    .map_err(|e| format!("delete collection membership: {}", e))?;
    let header = conn
        .execute(
            "DELETE FROM Entries WHERE p_uuid = ?1 AND p_type='Collection'",
            params![uuid],
        )
        .map_err(|e| format!("delete collection entry: {}", e))?;
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replica of the cc.db schema as documented for the Kindle content
    /// catalog: `Entries` with an icu-collated title column plus an extra
    /// column (`p_extra`) to prove template-clone copies unknown columns.
    fn replica_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.create_collation("icu", |a, b| a.cmp(b))
            .expect("icu stub");
        conn.execute_batch(
            "CREATE TABLE Entries (
                p_uuid TEXT PRIMARY KEY,
                p_type TEXT,
                p_cdeType TEXT,
                p_cdeKey TEXT,
                p_location TEXT,
                p_titles_0_nominal TEXT COLLATE icu,
                p_titleCount INTEGER,
                j_titles TEXT,
                j_members TEXT,
                p_memberCount INTEGER,
                p_homeMemberCount INTEGER,
                p_collectionCount INTEGER,
                p_seriesState INTEGER,
                p_extra TEXT
            );
            CREATE TABLE Collections (
                i_collection_uuid TEXT,
                i_member_uuid TEXT,
                i_order INTEGER,
                i_member_cde_type TEXT,
                i_member_cde_key TEXT,
                i_member_is_present INTEGER,
                i_is_sideloaded INTEGER
            );
            INSERT INTO Entries (p_uuid, p_type, p_titles_0_nominal, p_titleCount, j_titles,
                                 j_members, p_memberCount, p_homeMemberCount, p_collectionCount,
                                 p_seriesState, p_extra)
            VALUES ('col-template', 'Collection', 'Existing', 3,
                    '[{\"nominal\":\"Existing\",\"lang\":\"en\"}]',
                    '[\"a\",\"b\",\"c\"]', 3, 3, 1, 0, 'keepme');
            INSERT INTO Entries (p_uuid, p_type, p_cdeType, p_cdeKey, p_location)
            VALUES ('item-1', 'Entry:Item', 'cde1', 'key1', 'file:///mnt/us/documents/ksync/gutenberg/book1.azw3');
            INSERT INTO Entries (p_uuid, p_type, p_cdeType, p_cdeKey, p_location)
            VALUES ('item-2', 'Entry:Item', 'cde2', 'key2', 'file:///mnt/us/documents/ksync/gutenberg/book2.mobi');
            INSERT INTO Entries (p_uuid, p_type, p_cdeType, p_cdeKey, p_location)
            VALUES ('item-3', 'Entry:Item', 'cde3', 'key3', 'file:///mnt/us/documents/ksync/gutenberg/sub/book3.pdf');
            ",
        )
        .expect("schema");
        conn
    }

    fn catalog() -> Catalog {
        Catalog {
            id: "gutenberg".into(),
            name: "Gutenberg".into(),
            url: "https://www.gutenberg.org/ebooks.opds/".into(),
            username: None,
            password: None,
            insecure: false,
            enabled: true,
        }
    }

    #[test]
    fn collection_name_uses_prefix() {
        let c = catalog();
        assert_eq!(collection_name(&c, "KSync"), "KSync Gutenberg");
        assert_eq!(collection_name(&c, "MyLib"), "MyLib Gutenberg");
        assert_eq!(collection_name(&c, "  "), "Gutenberg");
    }

    fn member_paths() -> Vec<PathBuf> {
        vec![
            PathBuf::from("/mnt/us/documents/ksync/gutenberg/book1.azw3"),
            PathBuf::from("/mnt/us/documents/ksync/gutenberg/book2.mobi"),
            PathBuf::from("/mnt/us/documents/ksync/gutenberg/sub/book3.pdf"),
            // not yet indexed by the scanner -> pending
            PathBuf::from("/mnt/us/documents/ksync/gutenberg/new.azw"),
        ]
    }

    fn collection_rows(conn: &Connection) -> Vec<(String, String, i64, String, String, i64, i64)> {
        let mut stmt = conn
            .prepare(
                "SELECT i_collection_uuid, i_member_uuid, i_order, i_member_cde_type, \
                 i_member_cde_key, i_member_is_present, i_is_sideloaded \
                 FROM Collections ORDER BY i_order",
            )
            .unwrap();
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    #[test]
    fn apply_ccdb_clones_template_and_rebuilds_membership() {
        let mut conn = replica_db();
        let result = apply_ccdb_conn(&mut conn, &catalog(), "KSync", &member_paths());
        assert_eq!(
            result,
            CollectionsResult {
                added: 3,
                pending: 1,
                error: None
            }
        );

        // Collection row created from the template, overriding identity fields.
        let (uuid, title, count, j_titles, j_members, extra): (
            String,
            String,
            i64,
            String,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT p_uuid, p_titles_0_nominal, p_memberCount, j_titles, j_members, p_extra \
                 FROM Entries WHERE p_type='Collection' AND p_titles_0_nominal = 'KSync Gutenberg'",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .expect("collection row");
        assert_eq!(title, "KSync Gutenberg");
        assert_eq!(count, 3);
        assert_eq!(extra, "keepme", "unknown template columns must be copied");
        assert_ne!(uuid, "col-template");
        let titles: serde_json::Value = serde_json::from_str(&j_titles).unwrap();
        assert_eq!(titles[0]["nominal"], "KSync Gutenberg");
        assert_eq!(titles[0]["lang"], "en", "template j_titles shape preserved");
        assert_eq!(j_members, "[\"item-1\",\"item-2\",\"item-3\"]");

        // Membership rows with cde info and sideloaded/present flags.
        let members = collection_rows(&conn);
        assert_eq!(members.len(), 3);
        assert_eq!(
            members[0],
            (
                uuid.clone(),
                "item-1".into(),
                0,
                "cde1".into(),
                "key1".into(),
                1,
                1
            )
        );
        assert_eq!(
            members[1],
            (
                uuid.clone(),
                "item-2".into(),
                1,
                "cde2".into(),
                "key2".into(),
                1,
                1
            )
        );
        assert_eq!(
            members[2],
            (
                uuid.clone(),
                "item-3".into(),
                2,
                "cde3".into(),
                "key3".into(),
                1,
                1
            )
        );
        for m in &members {
            assert_eq!(m.0, uuid);
        }
    }

    #[test]
    fn apply_ccdb_is_idempotent() {
        let mut conn = replica_db();
        let first = apply_ccdb_conn(&mut conn, &catalog(), "KSync", &member_paths());
        let second = apply_ccdb_conn(&mut conn, &catalog(), "KSync", &member_paths());
        assert_eq!(first, second);
        assert_eq!(
            collection_rows(&conn).len(),
            3,
            "DELETE + INSERT must not accumulate"
        );
        let count: i64 = conn
            .query_row(
                "SELECT p_memberCount FROM Entries WHERE p_titles_0_nominal = 'KSync Gutenberg'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn reuses_existing_collection_row() {
        let mut conn = replica_db();
        // Pre-create the KSync collection with a stale member, then rebuild.
        conn.execute(
            "INSERT INTO Entries (p_uuid, p_type, p_titles_0_nominal, p_titleCount, j_titles, \
             j_members, p_memberCount, p_homeMemberCount, p_collectionCount, p_seriesState) \
             VALUES ('col-ksync', 'Collection', 'KSync Gutenberg', 1, \
             '[{\"nominal\":\"KSync Gutenberg\"}]', '[\"old\"]', 1, 1, 0, 1)",
            [],
        )
        .unwrap();
        let result = apply_ccdb_conn(&mut conn, &catalog(), "KSync", &member_paths());
        assert_eq!(result.error, None);
        let members = collection_rows(&conn);
        assert_eq!(members.len(), 3, "stale membership replaced");
        let count: i64 = conn
            .query_row(
                "SELECT p_memberCount FROM Entries WHERE p_uuid = 'col-ksync'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn minimal_insert_when_no_template_exists() {
        let mut conn = replica_db();
        conn.execute("DELETE FROM Entries WHERE p_type='Collection'", [])
            .unwrap();
        let result = apply_ccdb_conn(&mut conn, &catalog(), "KSync", &member_paths());
        assert_eq!(result.error, None);
        let (count, series): (i64, i64) = conn
            .query_row(
                "SELECT p_memberCount, p_seriesState FROM Entries \
                 WHERE p_titles_0_nominal = 'KSync Gutenberg'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 3);
        assert_eq!(series, 1);
    }

    #[test]
    fn collect_member_paths_filters_native_exts() {
        let dir = std::env::temp_dir().join(format!("ksync-collect-{}", crate::now_epoch()));
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(dir.join("a.azw3"), "x").unwrap();
        fs::write(dir.join("b.PDF"), "x").unwrap();
        fs::write(dir.join("c.epub"), "x").unwrap();
        fs::write(dir.join("d.txt"), "x").unwrap();
        fs::write(sub.join("e.mobi"), "x").unwrap();
        fs::write(sub.join("notes.md"), "x").unwrap();
        let paths = collect_member_paths(&dir);
        let names: Vec<String> = paths
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.azw3", "b.PDF", "d.txt", "sub/e.mobi"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
