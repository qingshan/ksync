//! Host-testable core for the ksync Kindle daemon.
//!
//! Command decoding, catalog edits, and job selection live alongside OPDS,
//! sync, and collection logic. The binary owns LIPC, process lifecycle, and
//! worker I/O. LIPC buffer helpers here do not link against liblipc.

use std::ffi::{c_char, c_int, c_void};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod collections;
pub mod command;
pub mod config;
pub mod http;
pub mod jobs;
pub mod opds;
pub mod status;
pub mod sync;

/// Root under which each catalog's books land: `/mnt/us/documents/ksync/<id>`.
pub const DOCUMENTS_KSYNC: &str = "/mnt/us/documents/ksync";

/// Current time as unix epoch seconds (matches the `date +%s` convention the
/// tailscale WAF uses for `updatedAt`).
pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Port of `tools/opds-sync.py`'s `slugify`: trim, replace every char that is
/// not alphanumeric/`_`/`-`/` `/`.` with `_`, collapse whitespace runs to one
/// space, trim ` ` and `.`; empty result falls back to `"untitled"`.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = false;
    for c in text.trim().chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' || c == ' ' || c == '.' {
            if c == ' ' {
                if prev_space {
                    continue;
                }
                prev_space = true;
            } else {
                prev_space = false;
            }
            out.push(c);
        } else {
            prev_space = false;
            out.push('_');
        }
    }
    let trimmed = out.trim_matches(|c| c == ' ' || c == '.');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Lock the shared status, recovering from poisoning instead of panicking (a
/// panic in a LIPC callback would unwind through C frames).
pub fn lock_status(status: &Mutex<status::Status>) -> MutexGuard<'_, status::Status> {
    status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Mutate the shared status under a poison-recovered lock, stamp `updated_at`,
/// then invoke the writer callback (on the device this writes status.json).
pub fn mutate_status(
    status: &Mutex<status::Status>,
    on_update: &dyn Fn(),
    f: impl FnOnce(&mut status::Status),
) {
    {
        let mut s = lock_status(status);
        f(&mut s);
        s.updated_at = now_epoch();
    }
    on_update();
}

// --- vendored from tsctl's lib.rs ------------------------------------------
// Callback convention (verified against utild's StringHandler.h and openlipc's
// lipc-test-prop.c): liblipc calls string-property callbacks as
// `fn(lipc, property, value, data)` where for a GETTER `value` is the output
// `char*` buffer and `data` points to a `size_t` holding its capacity, and for
// a SETTER `value` is the NUL-terminated input string. `value` is NEVER a
// pointer to a wrapper struct.

// LIPCcode values (from lipc.h)
pub const LIPC_OK: c_int = 0;
pub const LIPC_ERROR_INTERNAL: c_int = 2;
pub const LIPC_ERROR_BUFFER_TOO_SMALL: c_int = 10;
pub const LIPC_ERROR_INVALID_ARG: c_int = 12;
pub const LIPC_ERROR_DUPLICATE_SERVICE_NAME: c_int = 17;

/// Copy `s` into the caller's buffer (getter path).
///
/// `value` is the output buffer, `data` points to the capacity. Returns
/// [`LIPC_ERROR_BUFFER_TOO_SMALL`] and stores the needed size in `capacity` when
/// the buffer is too small — the same convention as utild's `LIPCString::set`,
/// so liblipc clients (lipc-get-prop et al.) retry with a bigger buffer and
/// then succeed.
///
/// # Safety
/// `value` must point to a writable buffer of at least `capacity` bytes and
/// `data` must point to a `size_t` holding that capacity.
pub unsafe fn write_string_prop(value: *mut c_void, data: *mut c_void, s: &str) -> c_int {
    let buf = value as *mut c_char;
    let capacity = &mut *(data as *mut usize);
    let needed = s.len() + 1;
    if needed > *capacity {
        *capacity = needed;
        return LIPC_ERROR_BUFFER_TOO_SMALL;
    }
    // SAFETY: caller-provided buffer of at least `needed` bytes, s is valid for
    // `needed` bytes. Copy s.len() bytes, then write the NUL explicitly — the
    // byte after a &str is not guaranteed to be readable (adjacent literals get
    // merged without interior NULs).
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), buf as *mut u8, s.len());
        *buf.add(s.len()) = 0;
    };
    LIPC_OK
}

/// Read the caller-provided input string (setter path). Returns `None` on a
/// null buffer.
///
/// # Safety
/// `value` must point to a NUL-terminated C string (or be null).
pub unsafe fn read_string_prop(value: *mut c_void) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // SAFETY: caller-provided NUL-terminated buffer.
    Some(
        unsafe { std::ffi::CStr::from_ptr(value as *const c_char) }
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_ports_python_semantics() {
        assert_eq!(slugify("Dune: Part 2!"), "Dune_ Part 2_");
        assert_eq!(slugify("  hello   world  "), "hello world");
        assert_eq!(slugify("book.epub"), "book.epub");
        assert_eq!(slugify(".hidden."), "hidden");
        assert_eq!(slugify("."), "untitled");
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("caf\u{e9} au lait"), "caf\u{e9} au lait");
        assert_eq!(slugify("a\tb"), "a_b");
        assert_eq!(slugify("a  b"), "a b");
        assert_eq!(slugify("100% Pure!"), "100_ Pure_");
        assert_eq!(slugify("a.b.c"), "a.b.c");
    }

    #[test]
    fn fits_exactly() {
        let mut cap = 6usize;
        let mut buf = [0u8; 6];
        let rc = unsafe {
            write_string_prop(
                buf.as_mut_ptr() as *mut c_void,
                &mut cap as *mut usize as *mut c_void,
                "hello",
            )
        };
        assert_eq!(rc, LIPC_OK);
        assert_eq!(&buf, b"hello\0");
    }

    #[test]
    fn too_small_reports_needed_size() {
        let mut cap = 3usize;
        let mut buf = [0u8; 3];
        let rc = unsafe {
            write_string_prop(
                buf.as_mut_ptr() as *mut c_void,
                &mut cap as *mut usize as *mut c_void,
                "hello",
            )
        };
        assert_eq!(rc, LIPC_ERROR_BUFFER_TOO_SMALL);
        assert_eq!(cap, 6, "capacity must be updated so the client can retry");
    }

    #[test]
    fn read_input_string() {
        let mut buf = *b"ls\0";
        assert_eq!(
            unsafe { read_string_prop(buf.as_mut_ptr() as *mut c_void) },
            Some("ls".to_string())
        );
    }

    #[test]
    fn mutate_status_stamps_updated_at_and_calls_writer() {
        let status = Mutex::new(status::Status::default());
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let on_update = || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        };
        mutate_status(&status, &on_update, |s| {
            s.downloaded = 3;
        });
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let s = lock_status(&status);
        assert_eq!(s.downloaded, 3);
        assert!(s.updated_at > 0);
    }
}
