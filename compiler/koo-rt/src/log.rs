//! What `std/log` is made of: which lines are wanted, in which form, and the writing of
//! a line, whole, to the standard error output.
//!
//! The environment says how a program logs, so that the one who runs a program decides
//! and not only the one who wrote it: `KOO_LOG_LEVEL` is `debug`, `info`, `warn`,
//! `error` or `off`, and is `info` if it is not set; `KOO_LOG` is `text` or `json`, and
//! is `text` if it is not set; and `KOO_LOG_TIME=off` leaves the time out of each line,
//! for a collector that adds its own and for a test that compares what is logged.

use super::*;
use core::sync::atomic::AtomicI32;
use core::sync::atomic::Ordering::Relaxed;

unsafe extern "C" {
    fn getenv(name: *const u8) -> *const u8;
}

/// The lowest level that is written, or -1 until the environment has been asked.
static LEVEL: AtomicI32 = AtomicI32::new(-1);
/// Bit 1: lines are JSON. Bit 2: lines have no time. -1 until asked.
static FORM: AtomicI32 = AtomicI32::new(-1);

unsafe fn setting(name: &core::ffi::CStr) -> &'static [u8] {
    unsafe {
        let value = getenv(name.as_ptr() as *const u8);
        if value.is_null() { b"" } else { c_text(value) }
    }
}

fn level() -> i32 {
    let known = LEVEL.load(Relaxed);
    if known >= 0 {
        return known;
    }
    let asked = unsafe { setting(c"KOO_LOG_LEVEL") };
    let level = match asked.first().map(u8::to_ascii_lowercase) {
        Some(b'd') => 0,
        Some(b'w') => 2,
        Some(b'e') => 3,
        Some(b'o') => 4,
        _ => 1,
    };
    LEVEL.store(level, Relaxed);
    level
}

fn form() -> i32 {
    let known = FORM.load(Relaxed);
    if known >= 0 {
        return known;
    }
    let json = unsafe { setting(c"KOO_LOG") }.eq_ignore_ascii_case(b"json");
    let timeless = unsafe { setting(c"KOO_LOG_TIME") }.eq_ignore_ascii_case(b"off");
    let form = json as i32 | (timeless as i32) << 1;
    FORM.store(form, Relaxed);
    form
}

/// Whether a line of this level, from 0 for debug to 3 for error, is written.
#[unsafe(no_mangle)]
pub extern "C" fn koo_log_on(wanted: i64) -> bool {
    wanted >= level() as i64
}

/// Sets the lowest level that is written, whatever the environment said.
#[unsafe(no_mangle)]
pub extern "C" fn koo_log_level(lowest: i64) {
    LEVEL.store(lowest.clamp(0, 4) as i32, Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn koo_log_json() -> bool {
    form() & 1 != 0
}

#[unsafe(no_mangle)]
pub extern "C" fn koo_log_timed() -> bool {
    form() & 2 == 0
}

/// Writes a line and its newline in one piece, so that the lines of several threads and
/// processes do not run into each other.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_log_write(line: *const Header) {
    unsafe {
        koo_flush();
        let text = bytes(line);
        let whole = malloc(text.len() + 1);
        if whole.is_null() {
            return;
        }
        core::ptr::copy_nonoverlapping(text.as_ptr(), whole, text.len());
        *whole.add(text.len()) = b'\n';
        write_all(2, core::slice::from_raw_parts(whole, text.len() + 1));
        free(whole);
    }
}
