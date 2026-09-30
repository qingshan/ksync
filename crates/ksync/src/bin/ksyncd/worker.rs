//! Device I/O for the single background worker.

use std::path::Path;
use std::sync::atomic::Ordering;

use ksyncd::collections;
use ksyncd::config::{self, Catalog};
use ksyncd::jobs::Job;
use ksyncd::status;
use ksyncd::sync::{self, log_line};
use ksyncd::{http, opds};

use super::{
    set_status, status_mutex, write_status_file, CONFIG_PATH, LOG_PATH, STOP, WORKER_RUNNING,
};

// --- worker ----------------------------------------------------------------

pub(super) fn start_job(job: Job) {
    if WORKER_RUNNING.swap(true, Ordering::SeqCst) {
        set_status(|s| {
            s.state = status::STATE_ERROR.to_string();
            s.last_error = Some("sync already running".to_string());
        });
        return;
    }
    STOP.store(false, Ordering::SeqCst);
    std::thread::spawn(move || run_job(job));
}

fn run_job(job: Job) {
    let on_update = || write_status_file();
    log_line(Path::new(LOG_PATH), "task start");
    let mut saved_catalogs = config::load(Path::new(CONFIG_PATH));
    match job.catalogs(saved_catalogs.clone()) {
        Ok(mut catalogs) => {
            for catalog in &mut catalogs {
                if STOP.load(Ordering::SeqCst) {
                    break;
                }
                if let Some(title) = opds_title(catalog) {
                    if catalog.name != title {
                        catalog.name = title.clone();
                        if let Some(saved) = saved_catalogs.iter_mut().find(|c| c.id == catalog.id)
                        {
                            saved.name = title;
                        }
                        if let Err(e) = config::save(Path::new(CONFIG_PATH), &saved_catalogs) {
                            log_line(
                                Path::new(LOG_PATH),
                                &format!("warning: could not save OPDS title: {}", e),
                            );
                        } else {
                            let summaries = saved_catalogs.iter().map(Catalog::summary).collect();
                            set_status(|s| s.catalogs = summaries);
                        }
                    }
                }
                migrate_legacy_downloads(catalog);
                match &job {
                    Job::Sync(_) => sync_one(catalog, &on_update),
                    Job::CollectionsOnly => collections_one(catalog),
                }
            }
        }
        Err(error) => set_status(|s| {
            if matches!(job, Job::Sync(Some(_))) {
                s.state = status::STATE_ERROR.to_string();
            }
            s.last_error = Some(error);
        }),
    }
    STOP.store(false, Ordering::SeqCst);
    set_status(|s| {
        s.phase = status::PHASE_DONE.to_string();
        if s.state != status::STATE_ERROR {
            s.state = status::STATE_DONE.to_string();
        }
        s.finished_at = Some(ksyncd::now_epoch());
    });
    // Publish the terminal state before admitting another worker, otherwise a
    // just-started worker can have its running status overwritten with done.
    WORKER_RUNNING.store(false, Ordering::SeqCst);
    log_line(Path::new(LOG_PATH), "task end");
}

fn opds_title(catalog: &Catalog) -> Option<String> {
    let body = http::fetch(&catalog.url, catalog, Some(sync::FEED_ACCEPT)).ok()?;
    let text = std::str::from_utf8(&body).ok()?;
    let (title, _) = opds::parse_feed(text).ok()?;
    if title.trim().is_empty() {
        None
    } else {
        Some(title.trim().to_string())
    }
}

fn migrate_legacy_downloads(catalog: &Catalog) {
    fn move_contents(from: &Path, to: &Path) {
        let Ok(entries) = std::fs::read_dir(from) else {
            return;
        };
        for entry in entries.flatten() {
            let source = entry.path();
            let target = to.join(entry.file_name());
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                move_contents(&source, &target);
                let _ = std::fs::remove_dir(&source);
            } else if kind.is_file() {
                if target.exists() {
                    log_line(
                        Path::new(LOG_PATH),
                        &format!(
                            "warning: keeping existing file during migration: {}",
                            target.display()
                        ),
                    );
                    continue;
                }
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::rename(&source, &target) {
                    log_line(
                        Path::new(LOG_PATH),
                        &format!("warning: could not migrate {}: {}", source.display(), e),
                    );
                }
            }
        }
    }

    let old = catalog.legacy_base_dir();
    let new = catalog.base_dir();
    if old != new && old.is_dir() {
        move_contents(&old, &new);
        let _ = std::fs::remove_dir(&old);
        if let Some(parent) = old.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

fn sync_one(catalog: &Catalog, on_update: &dyn Fn()) {
    log_line(
        Path::new(LOG_PATH),
        &format!("sync start: {} ({})", catalog.name, catalog.id),
    );
    set_status(|s| {
        s.state = status::STATE_RUNNING.to_string();
        s.catalog_id = Some(catalog.id.clone());
        s.catalog_name = Some(catalog.name.clone());
        s.started_at = Some(ksyncd::now_epoch());
    });
    sync::run_catalog_sync(
        catalog,
        &catalog.base_dir(),
        &STOP,
        status_mutex(),
        Path::new(LOG_PATH),
        on_update,
    );
    if STOP.load(Ordering::SeqCst) {
        log_line(
            Path::new(LOG_PATH),
            &format!("sync stopped by user: {}", catalog.id),
        );
    } else {
        collections_one(catalog);
    }
}

fn collections_one(catalog: &Catalog) {
    log_line(
        Path::new(LOG_PATH),
        &format!("collections rebuild start: {}", catalog.id),
    );
    set_status(|s| {
        s.phase = status::PHASE_COLLECTIONS.to_string();
    });
    let result = collections::rebuild_catalog(catalog, &config::load_settings().collection_prefix);
    set_status(|s| {
        s.collections_added = result.added as u64;
        s.collections_pending = result.pending as u64;
        s.collections_error = result.error.clone();
    });
    match &result.error {
        Some(e) => log_line(
            Path::new(LOG_PATH),
            &format!("collections error for {}: {}", catalog.id, e),
        ),
        None => log_line(
            Path::new(LOG_PATH),
            &format!(
                "collections done for {}: {} added, {} pending",
                catalog.id, result.added, result.pending
            ),
        ),
    }
}
