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

/// A value beside the message of a line that the support library writes itself.
pub enum Field<'a> {
    Text(&'a [u8]),
    Number(i64),
    /// Digits as they are to be written, for a number that is not whole.
    Digits(&'a [u8]),
}

/// Appends text as JSON writes a string, without the quotes.
unsafe fn escaped(out: &mut pg::Buf, text: &[u8]) {
    unsafe {
        for byte in text {
            match byte {
                b'"' => out.push(b"\\\""),
                b'\\' => out.push(b"\\\\"),
                b'\n' => out.push(b"\\n"),
                b'\r' => out.push(b"\\r"),
                b'\t' => out.push(b"\\t"),
                0..=31 => {
                    let mut code = Buffer::new();
                    let _ = write!(code, "\\u{byte:04x}");
                    out.push(&code.data[..code.len]);
                }
                _ => out.push(core::slice::from_ref(byte)),
            }
        }
    }
}

/// The time now, as `2026-01-31T12:00:00.123456Z`.
unsafe fn stamp(out: &mut Buffer) {
    unsafe {
        let mut time = [0i64; 2];
        clock_gettime(0, &mut time);
        let (days, second) = (time[0].div_euclid(86400), time[0].rem_euclid(86400));
        // The date of a count of days, by the calendar's cycle of 400 years, counted
        // from a first of March so that the day a leap year adds comes last.
        let shifted = days + 719_468;
        let era = shifted.div_euclid(146_097);
        let day_of_era = shifted.rem_euclid(146_097);
        let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let turned = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * turned + 2) / 5 + 1;
        let month = if turned < 10 { turned + 3 } else { turned - 9 };
        let year = year_of_era + era * 400 + (month <= 2) as i64;
        let _ = write!(
            out,
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z",
            second / 3600,
            second % 3600 / 60,
            second % 60,
            time[1] / 1000
        );
    }
}

/// Writes a line that the support library itself has to say: a panic, a request that
/// was served, an actor that starts again. When the log is text it is `plain`, as it
/// always was. When the log is JSON it is an object like the program's own lines, so
/// that whatever reads the log can read all of it.
pub unsafe fn said(level: &[u8], plain: &[u8], message: &[u8], fields: &[(&[u8], Field)]) {
    unsafe {
        koo_flush();
        if !koo_log_json() {
            write_all(2, plain);
            return;
        }
        let mut line = pg::Buf::new();
        line.push(b"{");
        if koo_log_timed() {
            let mut time = Buffer::new();
            stamp(&mut time);
            line.push(b"\"time\":\"");
            line.push(&time.data[..time.len]);
            line.push(b"\",");
        }
        line.push(b"\"level\":\"");
        line.push(level);
        line.push(b"\",\"message\":\"");
        escaped(&mut line, message);
        line.push(b"\"");
        for (name, field) in fields {
            line.push(b",\"");
            escaped(&mut line, name);
            line.push(b"\":");
            match field {
                Field::Text(text) => {
                    line.push(b"\"");
                    escaped(&mut line, text);
                    line.push(b"\"");
                }
                Field::Number(number) => {
                    let mut digits = Buffer::new();
                    let _ = write!(digits, "{number}");
                    line.push(&digits.data[..digits.len]);
                }
                Field::Digits(digits) => line.push(digits),
            }
        }
        line.push(b"}\n");
        write_all(2, line.bytes());
        line.release();
    }
}
