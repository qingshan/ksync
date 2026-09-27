//! Device I/O for the single background worker.

use std::path::Path;
use std::sync::atomic::Ordering;

use ksyncd::collections;
use ksyncd::config::{self, Catalog};
use ksyncd::jobs::Job;
use ksyncd::status;
use ksyncd::sync::{self, log_line};

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
    match job.catalogs(config::load(Path::new(CONFIG_PATH))) {
        Ok(catalogs) => {
            for catalog in &catalogs {
                if STOP.load(Ordering::SeqCst) {
                    break;
                }
                match job {
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
