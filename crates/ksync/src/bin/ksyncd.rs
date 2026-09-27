//! ksyncd — the LIPC daemon behind the ksync Kindle package.
//!
//! Registers the `dev.qingshan.ksyncd` LIPC service (same skeleton as tsctl:
//! liblipc string properties, classic double-fork daemonize, `-n` foreground
//! mode for the upstart job):
//!
//! - `cmd` (setter) — a JSON op (`{"op": "catalog_add", ...}`, see the README
//!   for the full list) parsed and pushed to the dispatcher over an mpsc
//!   channel; callbacks never block. The getter echoes the last command back.
//! - `status` (getter) — the current status JSON (the same document the WAF
//!   polls from `/var/local/mesquite/ksync/status.json`).
//! - `exit` (setter) — stop the daemon.
//! - `info` (getter) — build info.
//!
//! A single worker thread runs sync tasks (`sync_start`/`sync_all`/`collections_rebuild`);
//! `sync_stop` flips a shared `AtomicBool` the sync engine checks before every
//! download. After a task finishes the status stays `done` (the WAF shows the
//! result) until the next op starts a run.

use std::ffi::{c_char, c_int, c_void, CString};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use ksyncd::collections;
use ksyncd::command::{self, Op};
use ksyncd::config::{self, Catalog};
use ksyncd::jobs::Job;
use ksyncd::status::{self, Status};
use ksyncd::sync::log_line;

#[path = "ksyncd/worker.rs"]
mod worker;
use ksyncd::{
    lock_status, mutate_status, read_string_prop, write_string_prop,
    LIPC_ERROR_DUPLICATE_SERVICE_NAME, LIPC_ERROR_INVALID_ARG, LIPC_OK,
};
use worker::start_job;

const SERVICE_NAME: &str = "dev.qingshan.ksyncd";
const CONFIG_PATH: &str = "/mnt/us/ksync/var/catalogs.json";
const STATUS_DIR: &str = "/var/local/mesquite/ksync";
const STATUS_PATH: &str = "/var/local/mesquite/ksync/status.json";
const LOG_PATH: &str = "/mnt/us/ksync/var/sync_log.txt";

type LipcHandle = *mut c_void;
type LipcCallback = extern "C" fn(LipcHandle, *const c_char, *mut c_void, *mut c_void) -> c_int;

// liblipc.so.1 ships with every Kindle firmware; the cross toolchain resolves
// it from its own sysroot at link time.
#[link(name = "lipc")]
extern "C" {
    fn LipcOpenEx(service: *const c_char, code: *mut c_int) -> LipcHandle;
    fn LipcClose(handle: LipcHandle) -> c_int;
    fn LipcRegisterStringProperty(
        lipc: LipcHandle,
        prop: *const c_char,
        getter: Option<LipcCallback>,
        setter: Option<LipcCallback>,
        data: *mut c_void,
    ) -> c_int;
}

static KEEP_RUNNING: AtomicBool = AtomicBool::new(true);
static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static LAST_CMD: Mutex<Option<String>> = Mutex::new(None);
static OP_TX: OnceLock<Sender<Op>> = OnceLock::new();
static STATUS: OnceLock<Mutex<Status>> = OnceLock::new();

fn status_mutex() -> &'static Mutex<Status> {
    STATUS.get_or_init(|| Mutex::new(Status::default()))
}

fn last_cmd() -> std::sync::MutexGuard<'static, Option<String>> {
    LAST_CMD.lock().unwrap_or_else(|p| p.into_inner())
}

// --- status.json writer ----------------------------------------------------
// Every status mutation goes through `mutate_status`, which stamps
// `updated_at` and then runs this writer, so the WAF's polled status.json is
// always current (written atomically: tmp + rename).

fn write_status_file() {
    let json = lock_status(status_mutex()).to_json();
    let _ = std::fs::create_dir_all(STATUS_DIR);
    let tmp = format!("{}.tmp", STATUS_PATH);
    if std::fs::write(&tmp, &json).is_ok() {
        let _ = std::fs::rename(&tmp, STATUS_PATH);
    }
}

fn set_status(f: impl FnOnce(&mut Status)) {
    let on_update = || write_status_file();
    mutate_status(status_mutex(), &on_update, f);
}

/// Refresh `status.catalogs` from the config file (passwords excluded).
fn refresh_status_catalogs() {
    let catalogs = config::load(Path::new(CONFIG_PATH));
    let summaries: Vec<_> = catalogs.iter().map(|c| c.summary()).collect();
    set_status(|s| s.catalogs = summaries);
}

/// Persist only successful edits and publish their password-free summaries.
fn edit_catalogs(edit: impl FnOnce(&mut Vec<Catalog>) -> Result<String, String>) {
    let mut catalogs = config::load(Path::new(CONFIG_PATH));
    let result = edit(&mut catalogs).and_then(|message| {
        config::save(Path::new(CONFIG_PATH), &catalogs)
            .map_err(|e| format!("could not save {}: {}", CONFIG_PATH, e))?;
        Ok(message)
    });
    match result {
        Ok(message) => {
            refresh_status_catalogs();
            log_line(Path::new(LOG_PATH), &message);
        }
        Err(error) => set_status(|s| s.last_error = Some(error)),
    }
}

// --- cmd property ----------------------------------------------------------

extern "C" fn cmd_getter(
    _h: LipcHandle,
    _p: *const c_char,
    value: *mut c_void,
    data: *mut c_void,
) -> c_int {
    let out = last_cmd()
        .clone()
        .unwrap_or_else(|| "No command yet.".to_string());
    unsafe { write_string_prop(value, data, &out) }
}

extern "C" fn cmd_setter(
    _h: LipcHandle,
    _p: *const c_char,
    value: *mut c_void,
    _d: *mut c_void,
) -> c_int {
    let Some(cmd) = (unsafe { read_string_prop(value) }) else {
        return LIPC_ERROR_INVALID_ARG;
    };
    match serde_json::from_str::<Op>(&cmd) {
        Ok(op) => {
            *last_cmd() = Some(format!("command received: {}", op.name()));
            if let Some(tx) = OP_TX.get() {
                let _ = tx.send(op);
            }
        }
        Err(e) => {
            *last_cmd() = Some("invalid command".to_string());
            set_status(|s| {
                s.last_error = Some(format!("invalid command: {}", e));
            });
        }
    }
    LIPC_OK
}

// --- status property -------------------------------------------------------

extern "C" fn status_getter(
    _h: LipcHandle,
    _p: *const c_char,
    value: *mut c_void,
    data: *mut c_void,
) -> c_int {
    let json = lock_status(status_mutex()).to_json();
    unsafe { write_string_prop(value, data, &json) }
}

// --- exit ------------------------------------------------------------------

extern "C" fn exit_getter(
    _h: LipcHandle,
    _p: *const c_char,
    value: *mut c_void,
    data: *mut c_void,
) -> c_int {
    unsafe { write_string_prop(value, data, "Write into this property to exit ksyncd") }
}

extern "C" fn exit_setter(
    _h: LipcHandle,
    _p: *const c_char,
    _value: *mut c_void,
    _d: *mut c_void,
) -> c_int {
    KEEP_RUNNING.store(false, Ordering::SeqCst);
    LIPC_OK
}

// --- info ------------------------------------------------------------------

extern "C" fn info_getter(
    _h: LipcHandle,
    _p: *const c_char,
    value: *mut c_void,
    data: *mut c_void,
) -> c_int {
    let msg = format!(
        "Build Info: Branch: {}, Commit: {}, Built On: {}",
        env!("GIT_BRANCH"),
        env!("GIT_COMMIT"),
        env!("BUILD_TIME")
    );
    unsafe { write_string_prop(value, data, &msg) }
}

// --- dispatcher ------------------------------------------------------------

fn dispatch(rx: mpsc::Receiver<Op>) {
    for op in rx {
        match op {
            Op::CatalogAdd(catalog) => edit_catalogs(|catalogs| {
                let id = catalog.add_to(catalogs);
                Ok(format!("catalog added: {}", id))
            }),
            Op::CatalogUpdate { id, catalog } => edit_catalogs(|catalogs| {
                catalog.update_in(&id, catalogs)?;
                Ok(format!("catalog updated: {}", id))
            }),
            Op::CatalogRemove { id } => edit_catalogs(|catalogs| {
                command::remove_catalog(&id, catalogs)?;
                Ok(format!("catalog removed: {}", id))
            }),
            Op::CatalogSetEnabled { id, enabled } => edit_catalogs(|catalogs| {
                command::set_catalog_enabled(&id, enabled, catalogs)?;
                Ok(format!("catalog {} enabled={}", id, enabled))
            }),
            Op::SyncStart { id } => start_job(Job::Sync(Some(id))),
            Op::SyncAll => start_job(Job::Sync(None)),
            Op::SyncStop => {
                STOP.store(true, Ordering::SeqCst);
                set_status(|s| {
                    s.state = status::STATE_STOPPING.to_string();
                });
                log_line(Path::new(LOG_PATH), "stop requested");
            }
            Op::CollectionsRebuild => start_job(Job::CollectionsOnly),
            Op::SetCollectionPrefix { prefix } => {
                let settings = config::Settings {
                    collection_prefix: prefix.trim().to_string(),
                };
                match config::save_settings(&settings) {
                    Ok(()) => {
                        set_status(|s| {
                            s.collection_prefix = settings.collection_prefix.clone();
                        });
                        log_line(
                            Path::new(LOG_PATH),
                            &format!("collection prefix set to {:?}", settings.collection_prefix),
                        );
                    }
                    Err(e) => set_status(|s| {
                        s.last_error = Some(format!("could not save settings: {}", e));
                    }),
                }
            }
            Op::CollectionsRemove { name } => {
                let result = collections::remove_collection(&name);
                set_status(|s| {
                    s.collections_error = result.error.clone();
                });
                match result.error {
                    Some(e) => log_line(
                        Path::new(LOG_PATH),
                        &format!("collection remove failed for {}: {}", name, e),
                    ),
                    None => log_line(
                        Path::new(LOG_PATH),
                        &format!("collection removed: {}", name),
                    ),
                }
            }
        }
    }
}

// --- daemon / signals ------------------------------------------------------
// Same skeleton as tsctl (classic double-fork).

fn daemonize() {
    unsafe {
        if libc::fork() > 0 {
            std::process::exit(0);
        }
        if libc::setsid() < 0 {
            std::process::exit(1);
        }
        libc::signal(libc::SIGCHLD, libc::SIG_IGN);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        if libc::fork() > 0 {
            std::process::exit(0);
        }
        libc::umask(0o000);
        libc::chdir(c"/mnt/us/ksync".as_ptr());
        for fd in 0..libc::sysconf(libc::_SC_OPEN_MAX) {
            libc::close(fd as c_int);
        }
    }
}

extern "C" fn handle_signal(_sig: c_int) {
    KEEP_RUNNING.store(false, Ordering::SeqCst);
}

// --- main ------------------------------------------------------------------

fn main() {
    let run_as_daemon = !std::env::args()
        .skip(1)
        .any(|a| a == "-n" || a == "--no-daemon");
    if run_as_daemon {
        println!("Forking into the background.");
        daemonize();
    } else {
        println!("Running in foreground mode.");
        unsafe {
            libc::signal(
                libc::SIGINT,
                handle_signal as *const () as libc::sighandler_t,
            );
            libc::signal(
                libc::SIGTERM,
                handle_signal as *const () as libc::sighandler_t,
            );
        }
    }

    // Seed the shared state and write the first status.json so the WAF renders
    // immediately (idle, with the current catalog list + global prefix).
    status_mutex();
    set_status(|s| {
        s.collection_prefix = config::load_settings().collection_prefix;
    });
    refresh_status_catalogs();
    log_line(Path::new(LOG_PATH), "daemon start");

    let (tx, rx) = mpsc::channel::<Op>();
    let _ = OP_TX.set(tx);
    std::thread::spawn(move || dispatch(rx));

    let service = CString::new(SERVICE_NAME).expect("static, no NUL");
    let mut code: c_int = -1;
    let handle = unsafe { LipcOpenEx(service.as_ptr(), &mut code) };
    if code != LIPC_OK {
        if code == LIPC_ERROR_DUPLICATE_SERVICE_NAME {
            // Another ksyncd instance already owns this service - it's
            // serving the WAF, so starting again is a no-op, not an error.
            return;
        }
        eprintln!("Failed to open LIPC (code {code})");
        std::process::exit(1);
    }

    let props: [(&str, Option<LipcCallback>, Option<LipcCallback>); 4] = [
        ("cmd", Some(cmd_getter), Some(cmd_setter)),
        ("status", Some(status_getter), None),
        ("exit", Some(exit_getter), Some(exit_setter)),
        ("info", Some(info_getter), None),
    ];
    for (prop, getter, setter) in props {
        let prop = CString::new(prop).expect("static, no NUL");
        unsafe {
            LipcRegisterStringProperty(handle, prop.as_ptr(), getter, setter, std::ptr::null_mut())
        };
    }

    while KEEP_RUNNING.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_secs(1));
    }

    unsafe { LipcClose(handle) };
    log_line(Path::new(LOG_PATH), "daemon stop");
    println!("ksyncd shutting down.");
}
