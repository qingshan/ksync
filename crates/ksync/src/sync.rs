//! The sync engine: a single-threaded recursive walk of an OPDS catalog,
//! ported from `tools/opds-sync.py walk_catalog` (navigation feeds become
//! subfolders, acquisition links download the first linked format, feed-level
//! `next` links continue pagination in the same folder).
//!
//! No format filtering (all acquisition formats are downloaded — native
//! formats land in the Library + collections, EPUB/FB2/etc. stay on disk for
//! KOReader), no conversion, no force re-download, no scheduling (manual via
//! the WAF only). The stop flag is checked before every download; a set flag
//! flips the status to `stopping` and aborts cleanly.

use crate::config::Catalog;
use crate::http;
use crate::mutate_status;
use crate::opds;
use crate::status::{self, Status};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Accept header sent for OPDS feeds.
pub const FEED_ACCEPT: &str = "application/atom+xml,application/xml,*/*";

/// Append a timestamped line to the sync log (created on demand). The daemon
/// logs start/stop, task start/end, feed errors, download errors and
/// collections errors here.
pub fn log_line(log_path: &Path, msg: &str) {
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let line = format!("[{}] {}\n", crate::now_epoch(), msg);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

struct WalkContext<'a> {
    stop: &'a AtomicBool,
    status: &'a Mutex<Status>,
    log_path: &'a Path,
    on_update: &'a dyn Fn(),
    seen_feeds: HashSet<String>,
}

/// HTTP does not send URL fragments to the server, so they must not create
/// distinct traversal nodes.
fn feed_key(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            parsed.set_fragment(None);
            parsed.to_string()
        }
        Err(_) => url.to_string(),
    }
}

/// Walk one feed: download acquisition entries into `folder`, recurse into
/// navigation entries as `folder/<slugified title>`, follow `next` in the same
/// folder. Returns `false` when this feed could not be fetched/parsed or the
/// walk aborted; the engine uses the stop flag to tell those apart.
fn walk_feed(catalog: &Catalog, feed_url: &str, folder: &Path, ctx: &mut WalkContext<'_>) -> bool {
    if ctx.stop.load(Ordering::SeqCst) {
        mutate_status(ctx.status, ctx.on_update, |s| {
            s.state = status::STATE_STOPPING.to_string()
        });
        return false;
    }
    if !ctx.seen_feeds.insert(feed_key(feed_url)) {
        log_line(
            ctx.log_path,
            &format!("warning: skipping already visited feed {}", feed_url),
        );
        return true;
    }
    let body = match http::fetch(feed_url, catalog, Some(FEED_ACCEPT)) {
        Ok(b) => b,
        Err(e) => {
            log_line(
                ctx.log_path,
                &format!("warning: failed to fetch feed {}: {}", feed_url, e),
            );
            return false;
        }
    };
    let text = match std::str::from_utf8(&body) {
        Ok(t) => t,
        Err(e) => {
            log_line(
                ctx.log_path,
                &format!("warning: feed {} is not UTF-8: {}", feed_url, e),
            );
            return false;
        }
    };
    let (_, entries) = match opds::parse_feed(text) {
        Ok(x) => x,
        Err(e) => {
            log_line(
                ctx.log_path,
                &format!("warning: failed to parse feed {}: {}", feed_url, e),
            );
            return false;
        }
    };

    for entry in entries {
        if ctx.stop.load(Ordering::SeqCst) {
            mutate_status(ctx.status, ctx.on_update, |s| {
                s.state = status::STATE_STOPPING.to_string()
            });
            return false;
        }
        if let Some(link) = entry
            .links
            .iter()
            .find(|l| opds::is_acquisition_link(&l.rel))
        {
            // Prefer the first acquisition link (typically the primary format).
            let href = opds::urljoin(feed_url, &link.href);
            let ext = opds::guess_ext(&link.link_type, &href);
            let slug = crate::slugify(&entry.title);
            let dest = folder.join(format!("{}{}", slug, ext));
            if dest.exists() {
                mutate_status(ctx.status, ctx.on_update, |s| s.skipped += 1);
            } else {
                mutate_status(ctx.status, ctx.on_update, |s| {
                    s.current = entry.title.clone();
                    s.downloaded += 1;
                });
                if let Err(e) = http::download(&href, catalog, &dest) {
                    mutate_status(ctx.status, ctx.on_update, |s| s.failed += 1);
                    log_line(
                        ctx.log_path,
                        &format!("error: download {} -> {}: {}", entry.title, href, e),
                    );
                }
            }
        } else if let Some(link) = entry
            .links
            .iter()
            .find(|l| opds::is_navigation_link(&l.rel, &l.link_type))
        {
            // A sub-catalog (folder): recurse, mirroring the title as a subdir.
            let sub_url = opds::urljoin(feed_url, &link.href);
            let sub_dir = folder.join(crate::slugify(&entry.title));
            walk_feed(catalog, &sub_url, &sub_dir, ctx);
        }
        // else: entry has neither acquisition nor navigation links; ignore.
    }

    // OPDS pagination: follow the feed-level "next" link in the same folder.
    if let Some(next_url) = opds::next_href(text, feed_url) {
        walk_feed(catalog, &next_url, folder, ctx);
    }
    true
}

/// Run a full sync of one catalog into `base_dir` (`/mnt/us/documents/ksync/<id>`).
/// Resets the per-run counters, walks the catalog, and sets the state to
/// `error` when the root feed itself fails (sub-feed failures are warnings).
pub fn run_catalog_sync(
    catalog: &Catalog,
    base_dir: &Path,
    stop: &AtomicBool,
    status: &Mutex<Status>,
    log_path: &Path,
    on_update: &dyn Fn(),
) {
    mutate_status(status, on_update, |s| {
        s.phase = status::PHASE_SYNCING.to_string();
        s.downloaded = 0;
        s.skipped = 0;
        s.failed = 0;
        s.current.clear();
        s.last_error = None;
        s.collections_added = 0;
        s.collections_pending = 0;
        s.collections_error = None;
        s.catalog_id = Some(catalog.id.clone());
        s.catalog_name = Some(catalog.name.clone());
    });

    let stopped = stop.load(Ordering::SeqCst);
    let root_ok = if stopped {
        false
    } else {
        let mut ctx = WalkContext {
            stop,
            status,
            log_path,
            on_update,
            seen_feeds: HashSet::new(),
        };
        walk_feed(catalog, &catalog.url, base_dir, &mut ctx)
    };
    if !stopped && !root_ok {
        mutate_status(status, on_update, |s| {
            s.state = status::STATE_ERROR.to_string();
            if s.last_error.is_none() {
                s.last_error = Some(format!("failed to fetch root feed {}", catalog.url));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::feed_key;

    #[test]
    fn feed_key_ignores_fragments() {
        assert_eq!(
            feed_key("https://catalog.example/opds#page-one"),
            feed_key("https://catalog.example/opds#page-two")
        );
    }
}
