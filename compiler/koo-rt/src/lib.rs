//! The support library linked into every Koo program: allocation, strings, list growth,
//! printing and panics.
//!
//! Everything here is called from generated code through the C ABI. Hot paths (reference
//! counts, indexing, the fast path of `push`) are emitted inline by the code generator and
//! do not come through here.
//!
//! Layouts, which `koo-compiler/src/llvm.rs` shares:
//!
//! * string: `{ rc: i64, len: i64, cap: i64, bytes... }`
//! * list:   `{ rc: i64, len: i64, cap: i64, elements... }`
//!
//! A value is a pointer to its header, which begins with its count. The count says who
//! may reach the value:
//!
//! * above zero, one thread: that many references, counted with plain arithmetic, and
//!   with exactly one the value is changed where it is;
//! * `IMMORTAL`, a constant: never counted, never written, never freed;
//! * below `-2^62`, frozen: the value is never changed again, so any thread may read it,
//!   and its references (the count less `FROZEN`) are counted atomically. What crosses
//!   from one actor to another is frozen first (`actor.rs`). A frozen value with one
//!   reference is that thread's alone, and becomes an ordinary value again when the
//!   thread wants to change it.

#![cfg_attr(not(test), no_std)]
#![allow(clippy::missing_safety_doc)]
// A C `char` is signed on some machines and not on others, so a cast between a C string
// and bytes is needed on the first and does nothing on the second.
#![allow(clippy::unnecessary_cast)]
// The C library's functions are declared here over bytes, where the standard library,
// which the tests link, declares them over `c_void`. The two are the same to the machine.
#![allow(unknown_lints)]
#![allow(suspicious_runtime_symbol_definitions)]

use core::fmt::Write;

pub mod actor;
pub mod client;
pub mod codec;
pub mod digest;
pub mod fiber;
pub mod http;
pub mod log;
pub mod pg;
pub mod thread;
pub mod tls;

unsafe extern "C" {
    fn malloc(size: usize) -> *mut u8;
    fn calloc(count: usize, size: usize) -> *mut u8;
    fn realloc(ptr: *mut u8, size: usize) -> *mut u8;
    fn free(ptr: *mut u8);
    fn write(fd: i32, buf: *const u8, count: usize) -> isize;
    fn exit(code: i32) -> !;
    fn getenv(name: *const u8) -> *const u8;
}

#[cfg(not(test))]
#[panic_handler]
fn rust_panic(_: &core::panic::PanicInfo) -> ! {
    fatal(b"koo runtime: internal error\n")
}

/// `core` is built to unwind and names this symbol; the runtime never unwinds.
#[cfg(not(test))]
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}

const HEADER: usize = 24;
const IMMORTAL: i64 = -1;
const FROZEN: i64 = i64::MIN;

fn is_frozen(rc: i64) -> bool {
    rc < -(1 << 62)
}

unsafe fn count<'a>(value: *mut Header) -> &'a core::sync::atomic::AtomicI64 {
    unsafe { &*(value as *const core::sync::atomic::AtomicI64) }
}

/// Takes one more reference to a value of the program's.
unsafe fn retain(value: *mut Header) {
    use core::sync::atomic::Ordering::Relaxed;
    unsafe {
        let rc = count(value).load(Relaxed);
        if rc > 0 {
            (*value).rc = rc + 1;
        } else if is_frozen(rc) {
            count(value).fetch_add(1, Relaxed);
        }
    }
}

/// Gives up one reference to a value. Returns whether it was the last, and the value is
/// the caller's to free.
unsafe fn release(value: *mut Header) -> bool {
    use core::sync::atomic::Ordering::{AcqRel, Relaxed};
    unsafe {
        let rc = count(value).load(Relaxed);
        if rc > 0 {
            (*value).rc = rc - 1;
            rc == 1
        } else if is_frozen(rc) {
            count(value).fetch_sub(1, AcqRel) == FROZEN + 1
        } else {
            false
        }
    }
}

/// Whether the one reference to a value is the only one, so that the value may be
/// changed where it is. A frozen value that nothing else holds is thawed.
unsafe fn unique(value: *mut Header) -> bool {
    unsafe {
        let rc = count(value).load(core::sync::atomic::Ordering::Relaxed);
        if rc == FROZEN + 1 {
            (*value).rc = 1;
            return true;
        }
        rc == 1
    }
}

#[repr(C)]
pub struct Header {
    rc: i64,
    len: i64,
    cap: i64,
}

fn write_all(fd: i32, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let n = unsafe { write(fd, bytes.as_ptr(), bytes.len()) };
        if n <= 0 {
            return;
        }
        bytes = &bytes[n as usize..];
    }
}

fn fatal(message: &[u8]) -> ! {
    unsafe {
        log::said(b"error", message, message.trim_ascii_end(), &[]);
        exit(101)
    }
}

unsafe fn bytes<'a>(s: *const Header) -> &'a [u8] {
    unsafe { core::slice::from_raw_parts((s as *const u8).add(HEADER), (*s).len as usize) }
}

// ---- allocation -------------------------------------------------------------------------

/// Whether allocations are counted. With `KOO_LEAKS` set in the environment they are, each
/// thread keeping its own count, and a program that ends with any not freed reports them,
/// which is how the compiler's tests check that generated code frees what it allocates.
static COUNTING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[inline]
unsafe fn counted(by: i64) {
    if COUNTING.load(core::sync::atomic::Ordering::Relaxed) {
        unsafe { thread::me().live += by };
    }
}

/// Takes an allocation out of the count: it is the support library's own from here on,
/// and not a leak of the program's.
unsafe fn uncounted() {
    unsafe { counted(-1) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_alloc(size: i64) -> *mut u8 {
    let p = unsafe { malloc(size.max(1) as usize) };
    if p.is_null() {
        fatal(b"koo: out of memory\n");
    }
    unsafe { counted(1) };
    p
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_alloc_zeroed(size: i64) -> *mut u8 {
    let p = unsafe { calloc(1, size.max(1) as usize) };
    if p.is_null() {
        fatal(b"koo: out of memory\n");
    }
    unsafe { counted(1) };
    p
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_free(p: *mut u8) {
    unsafe {
        counted(-1);
        free(p)
    }
}

/// Called when `main` returns: writes buffered output, and checks for leaks if asked to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_exit_main() {
    unsafe {
        koo_flush();
        // What the actors of the program needed of the system is given back, and the
        // connections to its databases are closed.
        actor::at_exit();
        pg::at_exit();
        client::at_exit();
        // The message of the last input or output error is the library's own, and so is a
        // string that was read from JSON and never collected: each thread's.
        thread::every(|thread| {
            for kept in [&mut thread.last_error, &mut thread.json_text] {
                let kept = core::mem::replace(kept, core::ptr::null_mut());
                if !kept.is_null() {
                    koo_free(kept as *mut u8);
                }
            }
        });
    }
    let mut live = 0;
    unsafe { thread::every(|thread| live += thread.live) };
    // After a panic that ended a message and not the program, what the message held was
    // never freed, and the count says nothing.
    if live != 0 && !unsafe { actor::leaked() } {
        let mut out = Buffer::new();
        let _ = writeln!(out, "koo: {live} allocations were not freed");
        out.flush(2);
        unsafe { exit(102) }
    }
}

// ---- panics -----------------------------------------------------------------------------

/// Reports a panic and exits with status 101 (Section 5.9).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_panic(message: *const u8, len: i64, file: *const Header, line: i64) -> ! {
    unsafe { koo_flush() };
    let message = unsafe { core::slice::from_raw_parts(message, len as usize) };
    let mut out = Buffer::new();
    out.push(b"panic: ");
    out.push(message);
    let said = out.len;
    let mut place = Buffer::new();
    if line > 0 || line == WHOLE {
        place.push(unsafe { bytes(file) });
        if line > 0 {
            let _ = write!(place, ":{line}");
        }
        out.push(b"\n  at ");
        out.push(&place.data[..place.len]);
    } else if line < 0 {
        // In a function of the library, which is named in place of a line.
        place.push(unsafe { bytes(file) });
        out.push(b"\n  in `");
        out.push(&place.data[..place.len]);
        out.push(b"`");
    }
    out.push(b"\n");
    let fields = [(
        if line > 0 || line == WHOLE {
            &b"at"[..]
        } else {
            &b"in"[..]
        },
        log::Field::Text(&place.data[..place.len]),
    )];
    unsafe {
        log::said(
            b"error",
            &out.data[..out.len],
            &out.data[..said],
            if line != 0 { &fields } else { &[] },
        )
    };
    // A panic ends the program, unless it is in a message that it only ends.
    unsafe { actor::leave() }
}

/// The line that says `file` is a whole place already, the file and the line of it.
const WHOLE: i64 = -2;

/// A panic in a function of the library: at the place in the program that called the
/// function, if the function was told it, and else in the function, by its name. With
/// neither there is nothing to say of where.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_panic_from(message: *const u8, len: i64, name: *const Header, place: *const Header) -> ! {
    unsafe {
        if !place.is_null() && !bytes(place).is_empty() {
            koo_panic(message, len, place, WHOLE)
        } else if !bytes(name).is_empty() {
            koo_panic(message, len, name, -1)
        } else {
            koo_panic(message, len, name, 0)
        }
    }
}

/// `panic(message)` in a function of the library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_panic_str_from(message: *const Header, name: *const Header, place: *const Header) -> ! {
    let text = unsafe { bytes(message) };
    unsafe { koo_panic_from(text.as_ptr(), text.len() as i64, name, place) }
}

/// `panic(message)` in a program.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_panic_str(message: *const Header, file: *const Header, line: i64) -> ! {
    let text = unsafe { bytes(message) };
    unsafe { koo_panic(text.as_ptr(), text.len() as i64, file, line) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_exit(code: i32) -> ! {
    unsafe { exit(code) }
}

// ---- strings ----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_new(cap: i64) -> *mut Header {
    let cap = cap.max(16);
    let s = unsafe { koo_alloc(HEADER as i64 + cap) } as *mut Header;
    unsafe { s.write(Header { rc: 1, len: 0, cap }) };
    s
}

/// A new string holding a copy of `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_from_bytes(data: *const u8, len: i64) -> *mut Header {
    let s = unsafe { koo_str_new(len) };
    unsafe {
        core::ptr::copy_nonoverlapping(data, (s as *mut u8).add(HEADER), len as usize);
        (*s).len = len;
    }
    s
}

#[cfg(test)]
unsafe fn str_from(bytes: &[u8]) -> *mut Header {
    unsafe { koo_str_from_bytes(bytes.as_ptr(), bytes.len() as i64) }
}

/// Makes the string in `slot` uniquely owned with room for `extra` more bytes.
/// A string of the program's, holding a copy of these bytes.
unsafe fn new_str(bytes: &[u8]) -> *mut Header {
    unsafe { koo_str_from_bytes(bytes.as_ptr(), bytes.len() as i64) }
}

unsafe fn str_reserve(slot: *mut *mut Header, extra: usize) -> *mut Header {
    let s = unsafe { *slot };
    let len = unsafe { (*s).len } as usize;
    let needed = len + extra;
    if unsafe { unique(s) } {
        if needed <= unsafe { (*s).cap } as usize {
            return s;
        }
        let cap = needed.max(len * 2).max(16);
        let grown = unsafe { realloc(s as *mut u8, HEADER + cap) } as *mut Header;
        if grown.is_null() {
            fatal(b"koo: out of memory\n");
        }
        unsafe {
            (*grown).cap = cap as i64;
            *slot = grown;
        }
        return grown;
    }
    // Shared or static: copy, and give up one reference to the original.
    let cap = needed.max(len * 2).max(16);
    let copy = unsafe { koo_str_new(cap as i64) };
    unsafe {
        core::ptr::copy_nonoverlapping((s as *const u8).add(HEADER), (copy as *mut u8).add(HEADER), len);
        (*copy).len = len as i64;
        // Others held it too; if it is frozen, they may have let go of it since.
        if release(s) {
            koo_free(s as *mut u8);
        }
        *slot = copy;
    }
    copy
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_append_bytes(slot: *mut *mut Header, data: *const u8, len: i64) {
    let len = len as usize;
    let s = unsafe { str_reserve(slot, len) };
    unsafe {
        let at = (s as *mut u8).add(HEADER + (*s).len as usize);
        core::ptr::copy_nonoverlapping(data, at, len);
        (*s).len += len as i64;
    }
}

/// Appends `other` to the string in `slot`, in place when that string is uniquely owned.
/// The caller holds its own reference to `other`, so the two are never the same buffer
/// with a count of one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_append(slot: *mut *mut Header, other: *const Header) {
    let len = unsafe { (*other).len };
    unsafe { koo_str_append_bytes(slot, (other as *const u8).add(HEADER), len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_concat(a: *const Header, b: *const Header) -> *mut Header {
    let (a, b) = unsafe { (bytes(a), bytes(b)) };
    let s = unsafe { koo_str_new((a.len() + b.len()) as i64) };
    unsafe {
        let data = (s as *mut u8).add(HEADER);
        core::ptr::copy_nonoverlapping(a.as_ptr(), data, a.len());
        core::ptr::copy_nonoverlapping(b.as_ptr(), data.add(a.len()), b.len());
        (*s).len = (a.len() + b.len()) as i64;
    }
    s
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_eq(a: *const Header, b: *const Header) -> bool {
    a == b || unsafe { bytes(a) == bytes(b) }
}

/// Byte order, which is scalar-value order (Section 4.2): negative, zero or positive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_cmp(a: *const Header, b: *const Header) -> i32 {
    match unsafe { bytes(a).cmp(bytes(b)) } {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// A hash of the bytes of a string (FNV-1a, then mixed).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_hash(s: *const Header) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in unsafe { bytes(s) } {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= hash >> 32;
    hash as i64
}

/// The order of two floats (Section 4.2): negative, zero or positive. It is the IEEE 754
/// `totalOrder`, except that what is not a number is one value, after every number.
/// `totalOrder` tells such values apart by a sign that a processor is free to choose:
/// `0.0 / 0.0` is negative on x86-64 and positive on arm64, and a program must not sort
/// differently on the two.
#[unsafe(no_mangle)]
pub extern "C" fn koo_float_compare(a: f64, b: f64) -> i64 {
    let (a, b) = (
        if a.is_nan() { f64::NAN } else { a },
        if b.is_nan() { f64::NAN } else { b },
    );
    match a.total_cmp(&b) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// A small stack buffer that formats numbers without allocating.
struct Buffer {
    data: [u8; 512],
    len: usize,
}

impl Buffer {
    fn new() -> Buffer {
        Buffer { data: [0; 512], len: 0 }
    }

    fn push(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.data.len() - self.len);
        self.data[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
    }

    fn flush(&mut self, fd: i32) {
        write_all(fd, &self.data[..self.len]);
        self.len = 0;
    }
}

impl Write for Buffer {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.push(s.as_bytes());
        Ok(())
    }
}

/// Appends a string as it would be written in a program: between quotes, with a
/// backslash before a quote, a backslash and an opening brace, and with the characters
/// that cannot be seen written as their escapes. This is the form of a string inside a
/// list, a struct or another value, where it must be told from what is around it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_append_quoted(slot: *mut *mut Header, value: *const Header) {
    unsafe {
        let text = bytes(value);
        let plain = |byte: &u8| *byte >= b' ' && !matches!(*byte, b'"' | b'\\' | b'{' | 127);
        koo_str_append_bytes(slot, b"\"".as_ptr(), 1);
        let mut rest = text;
        while !rest.is_empty() {
            // What needs nothing is appended in one piece.
            let run = rest.iter().position(|byte| !plain(byte)).unwrap_or(rest.len());
            koo_str_append_bytes(slot, rest.as_ptr(), run as i64);
            rest = &rest[run..];
            let Some((&byte, after)) = rest.split_first() else {
                break;
            };
            let mut escape = Buffer::new();
            match byte {
                b'"' => escape.push(b"\\\""),
                b'\\' => escape.push(b"\\\\"),
                b'{' => escape.push(b"\\{"),
                b'\n' => escape.push(b"\\n"),
                b'\r' => escape.push(b"\\r"),
                b'\t' => escape.push(b"\\t"),
                0 => escape.push(b"\\0"),
                other => {
                    let _ = write!(escape, "\\u{{{other:x}}}");
                }
            }
            koo_str_append_bytes(slot, escape.data.as_ptr(), escape.len as i64);
            rest = after;
        }
        koo_str_append_bytes(slot, b"\"".as_ptr(), 1);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_append_int(slot: *mut *mut Header, value: i64) {
    let mut buffer = Buffer::new();
    let _ = write!(buffer, "{value}");
    unsafe { koo_str_append_bytes(slot, buffer.data.as_ptr(), buffer.len as i64) }
}

/// Appends a float. A negative `precision` asks for the shortest text that reads back as
/// the same value, always with a fraction or an exponent so that it reads as a float.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_append_float(slot: *mut *mut Header, value: f64, precision: i32) {
    let mut buffer = Buffer::new();
    let _ = if precision < 0 {
        write!(buffer, "{value:?}")
    } else {
        write!(buffer, "{value:.*}", precision as usize)
    };
    unsafe { koo_str_append_bytes(slot, buffer.data.as_ptr(), buffer.len as i64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_append_bool(slot: *mut *mut Header, value: bool) {
    let text: &[u8] = if value { b"true" } else { b"false" };
    unsafe { koo_str_append_bytes(slot, text.as_ptr(), text.len() as i64) }
}

/// Appends `piece` padded to `width` characters. `align` is 0 for left, 1 for right and 2
/// for the center; with `zero` a number is padded with zeros after its sign.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_append_padded(
    slot: *mut *mut Header,
    piece: *const Header,
    width: i64,
    align: i32,
    zero: bool,
) {
    let text = unsafe { text(piece) };
    let pad = (width as usize).saturating_sub(text.chars().count());
    let push = |slot: *mut *mut Header, bytes: &[u8]| unsafe {
        koo_str_append_bytes(slot, bytes.as_ptr(), bytes.len() as i64)
    };
    let fill = |slot: *mut *mut Header, count: usize, with: &[u8]| {
        for _ in 0..count {
            push(slot, with);
        }
    };
    if zero {
        let (sign, digits) = match text.strip_prefix('-') {
            Some(digits) => ("-", digits),
            None => ("", text),
        };
        push(slot, sign.as_bytes());
        fill(slot, pad, b"0");
        push(slot, digits.as_bytes());
        return;
    }
    let before = match align {
        1 => pad,
        2 => pad / 2,
        _ => 0,
    };
    fill(slot, before, b" ");
    push(slot, text.as_bytes());
    fill(slot, pad - before, b" ");
}

/// Parses a decimal integer, with an optional sign. Returns whether it succeeded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_int_parse(s: *const Header, out: *mut i64) -> bool {
    let text = unsafe { bytes(s) };
    let parsed = core::str::from_utf8(text).ok().and_then(|t| t.parse::<i64>().ok());
    match parsed {
        Some(value) => {
            unsafe { *out = value };
            true
        }
        None => false,
    }
}

/// Parses a float. Returns whether it succeeded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_float_parse(s: *const Header, out: *mut f64) -> bool {
    let text = unsafe { bytes(s) };
    let parsed = core::str::from_utf8(text).ok().and_then(|t| t.parse::<f64>().ok());
    match parsed {
        Some(value) => {
            unsafe { *out = value };
            true
        }
        None => false,
    }
}

// ---- string operations for the prelude --------------------------------------------------

unsafe fn text<'a>(s: *const Header) -> &'a str {
    // A Koo string is always valid UTF-8.
    unsafe { core::str::from_utf8_unchecked(bytes(s)) }
}

unsafe fn str_of(text: &str) -> *mut Header {
    unsafe { koo_str_from_bytes(text.as_ptr(), text.len() as i64) }
}

/// A list of `len` string handles, to be filled by the caller.
unsafe fn strs_new(len: usize) -> *mut Header {
    let list = unsafe { koo_list_new(len as i64, 8) };
    unsafe { (*list).len = len as i64 };
    list
}

unsafe fn strs_set(list: *mut Header, index: usize, value: *mut Header) {
    unsafe {
        ((list as *mut u8).add(HEADER) as *mut *mut Header)
            .add(index)
            .write(value)
    }
}

/// The byte offset of the first `needle` at or after `from`, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_find(s: *const Header, needle: *const Header, from: i64) -> i64 {
    let (hay, needle) = unsafe { (bytes(s), bytes(needle)) };
    if from < 0 || from as usize > hay.len() {
        return -1;
    }
    let from = from as usize;
    if needle.is_empty() {
        return from as i64;
    }
    if needle.len() > hay.len() - from {
        return -1;
    }
    let first = needle[0];
    let last_start = hay.len() - needle.len();
    let mut i = from;
    while i <= last_start {
        if hay[i] == first && &hay[i..i + needle.len()] == needle {
            return i as i64;
        }
        i += 1;
    }
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_starts_with(s: *const Header, prefix: *const Header) -> bool {
    unsafe { bytes(s).starts_with(bytes(prefix)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_ends_with(s: *const Header, suffix: *const Header) -> bool {
    unsafe { bytes(s).ends_with(bytes(suffix)) }
}

/// Whether a text has a part from byte offset `from` to `to`: the offsets are in range,
/// in order, and neither is inside a scalar value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_slice_ok(s: *const Header, from: i64, to: i64) -> bool {
    let text = unsafe { text(s) };
    from >= 0
        && from <= to
        && to as usize <= text.len()
        && text.is_char_boundary(from as usize)
        && text.is_char_boundary(to as usize)
}

/// A panic that the support library raises itself, with nothing to say of where: like
/// any panic, it ends the program, or only the message or the request it is in.
fn panic_here(message: &[u8]) -> ! {
    unsafe { koo_panic(message.as_ptr(), message.len() as i64, core::ptr::null(), 0) }
}

/// The text from byte offset `from` to `to`. An offset out of range or inside a scalar
/// value is a programming error (Section 4.4), which `slice` in the prelude has
/// looked for already.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_slice(s: *const Header, from: i64, to: i64) -> *mut Header {
    if !unsafe { koo_str_slice_ok(s, from, to) } {
        panic_here(b"string offsets out of range or not on a character boundary");
    }
    let text = unsafe { text(s) };
    unsafe { str_of(&text[from as usize..to as usize]) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_trim(s: *const Header) -> *mut Header {
    unsafe { str_of(text(s).trim()) }
}

unsafe fn map_chars(s: *const Header, upper: bool) -> *mut Header {
    let text = unsafe { text(s) };
    let mut out = unsafe { koo_str_new(text.len() as i64) };
    let mut buffer = [0u8; 4];
    for c in text.chars() {
        if upper {
            for mapped in c.to_uppercase() {
                let encoded = mapped.encode_utf8(&mut buffer);
                unsafe { koo_str_append_bytes(&mut out, encoded.as_ptr(), encoded.len() as i64) };
            }
        } else {
            for mapped in c.to_lowercase() {
                let encoded = mapped.encode_utf8(&mut buffer);
                unsafe { koo_str_append_bytes(&mut out, encoded.as_ptr(), encoded.len() as i64) };
            }
        }
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_upper(s: *const Header) -> *mut Header {
    unsafe { map_chars(s, true) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_lower(s: *const Header) -> *mut Header {
    unsafe { map_chars(s, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_replace(s: *const Header, from: *const Header, to: *const Header) -> *mut Header {
    let (text, from, to) = unsafe { (text(s), text(from), text(to)) };
    if from.is_empty() {
        return unsafe { str_of(text) };
    }
    let mut out = unsafe { koo_str_new(text.len() as i64) };
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        unsafe {
            koo_str_append_bytes(&mut out, rest.as_ptr(), at as i64);
            koo_str_append_bytes(&mut out, to.as_ptr(), to.len() as i64);
        }
        rest = &rest[at + from.len()..];
    }
    unsafe { koo_str_append_bytes(&mut out, rest.as_ptr(), rest.len() as i64) };
    out
}

/// The pieces of `s` between occurrences of `separator`. With an empty separator the
/// result is `s` alone.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_split(s: *const Header, separator: *const Header) -> *mut Header {
    let (text, separator) = unsafe { (text(s), text(separator)) };
    if separator.is_empty() {
        let list = unsafe { strs_new(1) };
        unsafe { strs_set(list, 0, str_of(text)) };
        return list;
    }
    let count = text.matches(separator).count() + 1;
    let list = unsafe { strs_new(count) };
    for (index, piece) in text.split(separator).enumerate() {
        unsafe { strs_set(list, index, str_of(piece)) };
    }
    list
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_repeat(s: *const Header, times: i64) -> *mut Header {
    let text = unsafe { text(s) };
    let times = times.max(0);
    let total = (text.len() as i64).checked_mul(times).unwrap_or(-1);
    if total < 0 {
        panic_here(b"a string this large cannot be allocated");
    }
    let mut out = unsafe { koo_str_new(total) };
    for _ in 0..times {
        unsafe { koo_str_append_bytes(&mut out, text.as_ptr(), text.len() as i64) };
    }
    out
}

/// The number of Unicode scalar values in the string (Section 4.4).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_count(s: *const Header) -> i64 {
    unsafe { text(s).chars().count() as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_bytes(s: *const Header) -> *mut Header {
    let data = unsafe { bytes(s) };
    let list = unsafe { koo_list_new(data.len() as i64, 1) };
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), (list as *mut u8).add(HEADER), data.len());
        (*list).len = data.len() as i64;
    }
    list
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_bytes_valid_utf8(list: *const Header) -> bool {
    core::str::from_utf8(unsafe { bytes(list) }).is_ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_bytes_to_str(list: *const Header) -> *mut Header {
    let data = unsafe { bytes(list) };
    unsafe { koo_str_from_bytes(data.as_ptr(), data.len() as i64) }
}

// ---- the program's surroundings: arguments, environment, files -------------------------

static mut ARGC: i32 = 0;
static mut ARGV: *const *const u8 = core::ptr::null();

/// Notes that an operation that can fail did, and why: the message is kept for the thread
/// to ask for.
unsafe fn failed_with(message: *mut Header) {
    unsafe {
        let me = thread::me();
        if !me.last_error.is_null() {
            koo_free(me.last_error as *mut u8);
        }
        me.last_error = message;
        me.failed = true;
    }
}

unsafe extern "C" {
    fn strlen(s: *const u8) -> usize;
    fn fopen(path: *const u8, mode: *const u8) -> *mut u8;
    fn fclose(file: *mut u8) -> i32;
    fn ferror(file: *mut u8) -> i32;
    fn fread(buffer: *mut u8, size: usize, count: usize, file: *mut u8) -> usize;
    fn fwrite(buffer: *const u8, size: usize, count: usize, file: *mut u8) -> usize;
    fn read(fd: i32, buffer: *mut u8, count: usize) -> isize;
    fn strerror(code: i32) -> *const u8;
    #[cfg_attr(target_os = "macos", link_name = "__error")]
    #[cfg_attr(not(target_os = "macos"), link_name = "__errno_location")]
    fn errno_location() -> *mut i32;
    fn clock_gettime(clock: i32, time: *mut [i64; 2]) -> i32;
}

/// Called first by `main`, with the program's arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_init(argc: i32, argv: *const *const u8) {
    unsafe {
        ARGC = argc;
        ARGV = argv;
        let counting = !getenv(c"KOO_LEAKS".as_ptr() as *const u8).is_null();
        COUNTING.store(counting, core::sync::atomic::Ordering::Relaxed);
        STARTED.store(koo_now_nanos(), core::sync::atomic::Ordering::Relaxed);
        thread::watch_stack();
    }
}

/// Reports that a thread's stack ran out, from the handler of the signal that says so.
/// What the program had printed is written first, unless the thread was in the middle
/// of printing, when it cannot be.
fn overflowed() {
    if PRINTING.try_lock() {
        let out = unsafe { &mut *core::ptr::addr_of_mut!(STDOUT) };
        write_all(1, &out.data[..out.len]);
        out.len = 0;
    }
    // This runs where a signal is handled, on a small stack and with whatever the
    // program was in the middle of left as it was: nothing is allocated, and the line
    // for a log that is JSON is written out whole.
    if !log::koo_log_json() {
        write_all(
            2,
            b"panic: the call stack is exhausted: a function calls itself too deeply or without end, or a value is nested too deeply\n",
        );
        return;
    }
    // One line, put together here and written in one piece, with its time like every
    // other line of the log.
    let mut line = Buffer::new();
    line.push(b"{");
    if log::koo_log_timed() {
        line.push(b"\"time\":\"");
        unsafe { log::stamp(&mut line) };
        line.push(b"\",");
    }
    line.push(b"\"level\":\"error\",\"message\":\"panic: the call stack is exhausted: a function calls itself too deeply or without end, or a value is nested too deeply\"}\n");
    write_all(2, &line.data[..line.len]);
}

unsafe fn c_text<'a>(s: *const u8) -> &'a [u8] {
    unsafe { core::slice::from_raw_parts(s, strlen(s)) }
}

/// A copy of a string with a NUL after it, for the C library. The caller frees it.
unsafe fn c_string(s: *const Header) -> *mut u8 {
    let data = unsafe { bytes(s) };
    let copy = unsafe { malloc(data.len() + 1) };
    if copy.is_null() {
        fatal(b"koo: out of memory\n");
    }
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), copy, data.len());
        *copy.add(data.len()) = 0;
    }
    copy
}

/// Opens a file. A path with a NUL in it names no file: the C library would stop at the
/// NUL and open some other one.
unsafe fn open(path: *const Header, mode: &core::ffi::CStr) -> *mut u8 {
    if unsafe { bytes(path) }.contains(&0) {
        unsafe { *errno_location() = 22 };
        return core::ptr::null_mut();
    }
    let c_path = unsafe { c_string(path) };
    let file = unsafe { fopen(c_path, mode.as_ptr() as *const u8) };
    unsafe { free(c_path) };
    file
}

unsafe fn fail_with(prefix: &[u8], path: *const Header) {
    let mut message = unsafe { koo_str_new(64) };
    unsafe {
        koo_str_append_bytes(&mut message, prefix.as_ptr(), prefix.len() as i64);
        for byte in bytes(path) {
            // The path as it can be shown: a NUL would end the message early.
            let shown: &[u8] = if *byte == 0 {
                b"\\0"
            } else {
                core::slice::from_ref(byte)
            };
            koo_str_append_bytes(&mut message, shown.as_ptr(), shown.len() as i64);
        }
        let reason = c_text(strerror(*errno_location()));
        koo_str_append_bytes(&mut message, b": ".as_ptr(), 2);
        koo_str_append_bytes(&mut message, reason.as_ptr(), reason.len() as i64);
        failed_with(message);
    }
}

/// A string from bytes that came from outside the program: arguments, the environment,
/// standard input. Every `string` is valid UTF-8 (Section 4.3), so bytes that are not
/// become U+FFFD, the replacement character.
unsafe fn str_from_outside(data: &[u8]) -> *mut Header {
    if core::str::from_utf8(data).is_ok() {
        return unsafe { koo_str_from_bytes(data.as_ptr(), data.len() as i64) };
    }
    let mut out = unsafe { koo_str_new(data.len() as i64 + 8) };
    for chunk in data.utf8_chunks() {
        let valid = chunk.valid().as_bytes();
        unsafe { koo_str_append_bytes(&mut out, valid.as_ptr(), valid.len() as i64) };
        if !chunk.invalid().is_empty() {
            let replacement = "\u{FFFD}".as_bytes();
            unsafe { koo_str_append_bytes(&mut out, replacement.as_ptr(), replacement.len() as i64) };
        }
    }
    out
}

/// The program's arguments, without its own name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_args() -> *mut Header {
    let count = unsafe { (ARGC - 1).max(0) } as usize;
    let list = unsafe { strs_new(count) };
    for index in 0..count {
        let arg = unsafe { c_text(*ARGV.add(index + 1)) };
        unsafe { strs_set(list, index, str_from_outside(arg)) };
    }
    list
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_env_has(name: *const Header) -> bool {
    let c_name = unsafe { c_string(name) };
    let found = !unsafe { getenv(c_name) }.is_null();
    unsafe { free(c_name) };
    found
}

/// The value of an environment variable, or the empty string if it is not set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_env(name: *const Header) -> *mut Header {
    let c_name = unsafe { c_string(name) };
    let value = unsafe { getenv(c_name) };
    unsafe { free(c_name) };
    if value.is_null() {
        return unsafe { koo_str_new(0) };
    }
    unsafe { str_from_outside(c_text(value)) }
}

struct Stdin {
    data: [u8; 8192],
    start: usize,
    end: usize,
}

static mut STDIN: Stdin = Stdin {
    data: [0; 8192],
    start: 0,
    end: 0,
};

/// The next line of standard input, without its line ending. At the end of the input it
/// is empty and `koo_at_end` turns true.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_read_line() -> *mut Header {
    let input = unsafe { &mut *core::ptr::addr_of_mut!(STDIN) };
    let mut line = unsafe { koo_str_new(64) };
    let mut any = false;
    loop {
        if input.start == input.end {
            let n = unsafe { read(0, input.data.as_mut_ptr(), input.data.len()) };
            if n <= 0 {
                unsafe { thread::me().at_end = !any };
                break;
            }
            input.start = 0;
            input.end = n as usize;
        }
        any = true;
        let chunk = &input.data[input.start..input.end];
        match chunk.iter().position(|&b| b == b'\n') {
            Some(at) => {
                unsafe { koo_str_append_bytes(&mut line, chunk.as_ptr(), at as i64) };
                input.start += at + 1;
                break;
            }
            None => {
                unsafe { koo_str_append_bytes(&mut line, chunk.as_ptr(), chunk.len() as i64) };
                input.start = input.end;
            }
        }
    }
    // A line that ended in CR LF.
    unsafe {
        let len = (*line).len as usize;
        if len > 0 && *(line as *const u8).add(HEADER + len - 1) == b'\r' {
            (*line).len -= 1;
        }
        if core::str::from_utf8(bytes(line)).is_err() {
            let valid = str_from_outside(bytes(line));
            koo_free(line as *mut u8);
            line = valid;
        }
    }
    line
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_at_end() -> bool {
    unsafe { thread::me().at_end }
}

/// The text of a file. On failure the result is empty and `koo_failed` turns true.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_read_file(path: *const Header) -> *mut Header {
    let mut text = unsafe { koo_str_new(4096) };
    let file = unsafe { open(path, c"rb") };
    if file.is_null() {
        unsafe { fail_with(b"cannot read ", path) };
        return text;
    }
    let mut buffer = [0u8; 16384];
    loop {
        let n = unsafe { fread(buffer.as_mut_ptr(), 1, buffer.len(), file) };
        if n == 0 {
            break;
        }
        unsafe { koo_str_append_bytes(&mut text, buffer.as_ptr(), n as i64) };
    }
    // Reading stops at the end of the file or at an error, such as the path being a
    // directory.
    let failed = unsafe { ferror(file) } != 0;
    let reason = unsafe { *errno_location() };
    unsafe { fclose(file) };
    if failed {
        unsafe {
            (*text).len = 0;
            *errno_location() = reason;
            fail_with(b"cannot read ", path);
        }
        return text;
    }
    if core::str::from_utf8(unsafe { bytes(text) }).is_err() {
        unsafe {
            (*text).len = 0;
            *errno_location() = 92;
            fail_with(b"not UTF-8 text: ", path);
        }
    }
    text
}

/// Writes a file, replacing what was there. Returns whether it worked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_write_file(path: *const Header, text: *const Header) -> bool {
    let file = unsafe { open(path, c"wb") };
    if file.is_null() {
        unsafe { fail_with(b"cannot write ", path) };
        return false;
    }
    let data = unsafe { bytes(text) };
    let written = unsafe { fwrite(data.as_ptr(), 1, data.len(), file) };
    let closed = unsafe { fclose(file) } == 0;
    if written != data.len() || !closed {
        unsafe { fail_with(b"cannot write ", path) };
        return false;
    }
    true
}

/// Whether the last operation that can fail did. Asking clears the answer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_failed() -> bool {
    unsafe { core::mem::replace(&mut thread::me().failed, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_last_error() -> *mut Header {
    unsafe {
        let last = thread::me().last_error;
        if last.is_null() {
            return koo_str_new(0);
        }
        let message = bytes(last);
        koo_str_from_bytes(message.as_ptr(), message.len() as i64)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_eprint(s: *const Header) {
    unsafe { koo_flush() };
    write_all(2, unsafe { bytes(s) });
    write_all(2, b"\n");
}

/// Standard output without a line feed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_write(s: *const Header) {
    PRINTING.lock();
    unsafe { stdout_write(bytes(s)) }
    PRINTING.unlock();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_exit_now(code: i64) {
    unsafe {
        koo_flush();
        exit(code as i32)
    }
}

/// When the program started, on the clock that only goes forward.
static STARTED: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);

/// Nanoseconds from the beginning of 1970 to now, by the machine's clock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_time_now() -> i64 {
    let mut time = [0i64; 2];
    unsafe { clock_gettime(0, &mut time) };
    time[0] * 1_000_000_000 + time[1]
}

/// Nanoseconds since the program started.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_uptime() -> i64 {
    unsafe { koo_now_nanos() - STARTED.load(core::sync::atomic::Ordering::Relaxed) }
}

/// Waits for some nanoseconds. What the program has printed is written first, so that
/// it is seen before the wait and not after.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sleep(nanos: i64) {
    unsafe extern "C" {
        fn nanosleep(wanted: *const [i64; 2], left: *mut [i64; 2]) -> i32;
    }
    if nanos <= 0 {
        return;
    }
    unsafe { koo_flush() };
    // A handler of a server sleeps without the server's sleeping with it.
    if unsafe { fiber::pause(nanos) } {
        return;
    }
    let mut wanted = [nanos / 1_000_000_000, nanos % 1_000_000_000];
    let mut left = [0i64; 2];
    // A signal ends a wait early, with what is left of it.
    while unsafe { nanosleep(&wanted, &mut left) } != 0 && unsafe { *errno_location() } == 4 {
        wanted = left;
    }
}

/// Nanoseconds on a clock that only goes forward.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_now_nanos() -> i64 {
    let clock = if cfg!(target_os = "macos") { 6 } else { 1 };
    let mut time = [0i64; 2];
    unsafe { clock_gettime(clock, &mut time) };
    time[0] * 1_000_000_000 + time[1]
}

/// The test that `koo test` asked this run for: the number in the environment variable
/// `KOO_TEST`, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_test_index() -> i64 {
    let value = unsafe { getenv(c"KOO_TEST".as_ptr() as *const u8) };
    if value.is_null() {
        return -1;
    }
    let value = unsafe { c_text(value) };
    core::str::from_utf8(value)
        .ok()
        .and_then(|t| t.parse::<i64>().ok())
        .unwrap_or(-1)
}

// ---- output -----------------------------------------------------------------------------

/// Output is collected and written when the buffer fills and when the program ends.
struct Stdout {
    data: [u8; 16384],
    len: usize,
}

static mut STDOUT: Stdout = Stdout {
    data: [0; 16384],
    len: 0,
};

/// In a process just forked: what a thread of the parent held is let go of.
unsafe fn threads_after_fork() {
    PRINTING.unlock();
    unsafe { thread::after_fork() };
}

/// Held while the output buffer is used, so that what two threads print does not mix
/// inside a line.
static PRINTING: thread::Lock = thread::Lock::new();

unsafe fn stdout_write(bytes: &[u8]) {
    let out = unsafe { &mut *core::ptr::addr_of_mut!(STDOUT) };
    if bytes.len() > out.data.len() - out.len {
        write_all(1, &out.data[..out.len]);
        out.len = 0;
        if bytes.len() > out.data.len() {
            write_all(1, bytes);
            return;
        }
    }
    out.data[out.len..out.len + bytes.len()].copy_from_slice(bytes);
    out.len += bytes.len();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_flush() {
    PRINTING.lock();
    let out = unsafe { &mut *core::ptr::addr_of_mut!(STDOUT) };
    write_all(1, &out.data[..out.len]);
    out.len = 0;
    PRINTING.unlock();
}

/// `print(s)`: the text and a line feed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_print(s: *const Header) {
    PRINTING.lock();
    unsafe {
        stdout_write(bytes(s));
        stdout_write(b"\n");
    }
    PRINTING.unlock();
}

/// Reports an error returned from `main` and exits with status 1 (Section 6.1).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_main_failed(description: *const Header) -> ! {
    unsafe {
        let description = bytes(description);
        let mut plain = pg::Buf::new();
        plain.push(b"error: ");
        plain.push(description);
        plain.push(b"\n");
        // For a log that is JSON, the error is the message, as a line like the rest.
        log::said(b"error", plain.bytes(), &plain.bytes()[..plain.len - 1], &[]);
        exit(1)
    }
}

// ---- lists ------------------------------------------------------------------------------

/// The bytes a list of `cap` elements needs. A size that cannot be computed is one that
/// cannot be allocated.
fn list_bytes(cap: i64, elem_size: i64) -> i64 {
    let bytes = cap
        .checked_mul(elem_size)
        .and_then(|data| data.checked_add(HEADER as i64));
    match bytes {
        Some(bytes) if cap >= 0 => bytes,
        _ => panic_here(b"a list this large cannot be allocated"),
    }
}

/// Grows the uniquely owned `list` to hold at least `min_cap` elements, and returns where
/// it now is. The old pointer is dead.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_list_grow(list: *mut Header, elem_size: i64, min_cap: i64) -> *mut Header {
    let cap = unsafe { (*list).cap };
    let new_cap = min_cap.max(cap.saturating_mul(2)).max(4);
    let size = list_bytes(new_cap, elem_size) as usize;
    let grown = unsafe { realloc(list as *mut u8, size) } as *mut Header;
    if grown.is_null() {
        fatal(b"koo: out of memory\n");
    }
    unsafe { (*grown).cap = new_cap };
    grown
}

/// Sorts a uniquely owned list of integers. Equal integers cannot be told apart, so the
/// sort need not be stable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_list_sort_ints(list: *mut Header) {
    let items =
        unsafe { core::slice::from_raw_parts_mut((list as *mut u8).add(HEADER) as *mut i64, (*list).len as usize) };
    items.sort_unstable();
}

/// A new list of `len` elements, each all zero bits.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_list_zeroed(len: i64, elem_size: i64) -> *mut Header {
    let list = unsafe { koo_alloc_zeroed(list_bytes(len, elem_size)) } as *mut Header;
    unsafe { list.write(Header { rc: 1, len, cap: len }) };
    list
}

/// A new, empty list with room for `cap` elements.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_list_new(cap: i64, elem_size: i64) -> *mut Header {
    let list = unsafe { koo_alloc(list_bytes(cap, elem_size)) } as *mut Header;
    unsafe { list.write(Header { rc: 1, len: 0, cap }) };
    list
}

// ---- JSON ---------------------------------------------------------------------------------
//
// The prelude reads JSON with these: each looks at the text from a position and returns
// the position after what it read, or -1 if the text is not that. A function that reads a
// value leaves it here for the call that follows to collect, since it has only one result.

// It is kept for the thread (`thread.rs`), so that two threads can read at once.

/// How deep arrays and objects may nest in a value that is skipped.
const JSON_DEPTH: usize = 512;

fn json_space(text: &[u8], mut at: usize) -> usize {
    while at < text.len() && matches!(text[at], b' ' | b'\n' | b'\r' | b'\t') {
        at += 1;
    }
    at
}

/// The position of the first byte at or after `at` that is not white space.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_ws(text: *const Header, at: i64) -> i64 {
    let text = unsafe { bytes(text) };
    json_space(text, (at.max(0) as usize).min(text.len())) as i64
}

/// The byte at a position, or -1 past the end.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_str_byte(text: *const Header, at: i64) -> i64 {
    let text = unsafe { bytes(text) };
    if at < 0 || at as usize >= text.len() {
        return -1;
    }
    text[at as usize] as i64
}

fn hex4(text: &[u8], at: usize) -> Option<u32> {
    let digits = text.get(at..at + 4)?;
    let mut value = 0u32;
    for &digit in digits {
        value = value * 16 + (digit as char).to_digit(16)?;
    }
    Some(value)
}

/// Reads the string that begins with the quote at `at`. Calls `piece` with each run of
/// its bytes, escapes resolved, and returns the position after the closing quote.
fn json_string(text: &[u8], at: usize, piece: &mut dyn FnMut(&[u8])) -> Option<usize> {
    if text.get(at) != Some(&b'"') {
        return None;
    }
    let mut i = at + 1;
    let mut run = i;
    loop {
        let byte = *text.get(i)?;
        match byte {
            b'"' => {
                piece(&text[run..i]);
                return Some(i + 1);
            }
            b'\\' => {
                piece(&text[run..i]);
                let escape = *text.get(i + 1)?;
                i += 2;
                let plain: &[u8] = match escape {
                    b'"' => b"\"",
                    b'\\' => b"\\",
                    b'/' => b"/",
                    b'b' => b"\x08",
                    b'f' => b"\x0c",
                    b'n' => b"\n",
                    b'r' => b"\r",
                    b't' => b"\t",
                    b'u' => {
                        let mut code = hex4(text, i)?;
                        i += 4;
                        if (0xD800..0xDC00).contains(&code) {
                            // The first half of a pair: the second must follow.
                            if text.get(i) != Some(&b'\\') || text.get(i + 1) != Some(&b'u') {
                                return None;
                            }
                            let low = hex4(text, i + 2)?;
                            if !(0xDC00..0xE000).contains(&low) {
                                return None;
                            }
                            i += 6;
                            code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                        }
                        let mut buffer = [0u8; 4];
                        // A second half on its own is not a character.
                        let encoded = char::from_u32(code)?.encode_utf8(&mut buffer);
                        piece(encoded.as_bytes());
                        b""
                    }
                    _ => return None,
                };
                piece(plain);
                run = i;
            }
            // A control character must be written as an escape.
            0..=0x1F => return None,
            _ => i += 1,
        }
    }
}

/// Reads a string at `at` and keeps it for `koo_json_text`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_string(text: *const Header, at: i64) -> i64 {
    let text = unsafe { bytes(text) };
    if at < 0 {
        return -1;
    }
    let mut out = unsafe { koo_str_new(16) };
    let end = json_string(text, at as usize, &mut |piece| unsafe {
        koo_str_append_bytes(&mut out, piece.as_ptr(), piece.len() as i64)
    });
    unsafe {
        let me = thread::me();
        let kept = core::mem::replace(&mut me.json_text, core::ptr::null_mut());
        if !kept.is_null() {
            koo_free(kept as *mut u8);
        }
        match end {
            Some(end) => {
                me.json_text = out;
                end as i64
            }
            None => {
                koo_free(out as *mut u8);
                -1
            }
        }
    }
}

/// The string that `koo_json_string` read last. It is given away: a second call gets "".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_text() -> *mut Header {
    unsafe {
        let kept = core::mem::replace(&mut thread::me().json_text, core::ptr::null_mut());
        if kept.is_null() { koo_str_new(0) } else { kept }
    }
}

/// The end of the number that begins at `at`, by the grammar of JSON.
fn json_number(text: &[u8], at: usize) -> Option<(usize, bool)> {
    let mut i = at;
    let digits = |i: &mut usize| {
        let start = *i;
        while text.get(*i).is_some_and(|b| b.is_ascii_digit()) {
            *i += 1;
        }
        *i > start
    };
    if text.get(i) == Some(&b'-') {
        i += 1;
    }
    match text.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            digits(&mut i);
        }
        _ => return None,
    }
    let mut whole = true;
    if text.get(i) == Some(&b'.') {
        i += 1;
        whole = false;
        if !digits(&mut i) {
            return None;
        }
    }
    if matches!(text.get(i), Some(b'e' | b'E')) {
        i += 1;
        whole = false;
        if matches!(text.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !digits(&mut i) {
            return None;
        }
    }
    Some((i, whole))
}

/// Reads a number at `at` and keeps it for `koo_json_int`, `koo_json_is_int` and
/// `koo_json_float`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_number(text: *const Header, at: i64) -> i64 {
    let text = unsafe { bytes(text) };
    if at < 0 {
        return -1;
    }
    let Some((end, whole)) = json_number(text, at as usize) else {
        return -1;
    };
    // The grammar admits only ASCII, so this is text.
    let Ok(number) = core::str::from_utf8(&text[at as usize..end]) else {
        return -1;
    };
    let int = if whole { number.parse::<i64>().ok() } else { None };
    let Ok(float) = number.parse::<f64>() else {
        return -1;
    };
    unsafe {
        let me = thread::me();
        me.json_is_int = int.is_some();
        me.json_int = int.unwrap_or(0);
        me.json_float = float;
    }
    end as i64
}

/// Whether the number read last is an integer that fits in an `int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_is_int() -> bool {
    unsafe { thread::me().json_is_int }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_int() -> i64 {
    unsafe { thread::me().json_int }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_float() -> f64 {
    unsafe { thread::me().json_float }
}

/// The position after `word` if the text has it at `at`, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_word(text: *const Header, at: i64, word: *const Header) -> i64 {
    let (text, word) = unsafe { (bytes(text), bytes(word)) };
    if at < 0 || at as usize > text.len() {
        return -1;
    }
    if text[at as usize..].starts_with(word) {
        at + word.len() as i64
    } else {
        -1
    }
}

/// The position after the value that begins at `at`, whatever it is.
fn json_skip(text: &[u8], at: usize) -> Option<usize> {
    // Whether each array or object that is open is an object.
    let mut open = [false; JSON_DEPTH];
    let mut depth = 0;
    let mut i = at;
    loop {
        // A value.
        i = json_space(text, i);
        match *text.get(i)? {
            b'"' => i = json_string(text, i, &mut |_| {})?,
            b'-' | b'0'..=b'9' => i = json_number(text, i)?.0,
            b't' if text[i..].starts_with(b"true") => i += 4,
            b'f' if text[i..].starts_with(b"false") => i += 5,
            b'n' if text[i..].starts_with(b"null") => i += 4,
            opener @ (b'[' | b'{') => {
                if depth == JSON_DEPTH {
                    return None;
                }
                open[depth] = opener == b'{';
                depth += 1;
                i = json_space(text, i + 1);
                let closer = if opener == b'{' { b'}' } else { b']' };
                if text.get(i) == Some(&closer) {
                    depth -= 1;
                    i += 1;
                } else if opener == b'{' {
                    i = json_space(text, json_string(text, i, &mut |_| {})?);
                    if text.get(i) != Some(&b':') {
                        return None;
                    }
                    i += 1;
                    continue;
                } else {
                    continue;
                }
            }
            _ => return None,
        }
        // What follows a value: the end of everything, a comma, or the end of what holds it.
        loop {
            if depth == 0 {
                return Some(i);
            }
            i = json_space(text, i);
            let object = open[depth - 1];
            match *text.get(i)? {
                b',' => {
                    i += 1;
                    if object {
                        i = json_space(text, i);
                        i = json_space(text, json_string(text, i, &mut |_| {})?);
                        if text.get(i) != Some(&b':') {
                            return None;
                        }
                        i += 1;
                    }
                    break;
                }
                b'}' if object => {
                    depth -= 1;
                    i += 1;
                }
                b']' if !object => {
                    depth -= 1;
                    i += 1;
                }
                _ => return None,
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_skip(text: *const Header, at: i64) -> i64 {
    let text = unsafe { bytes(text) };
    if at < 0 {
        return -1;
    }
    json_skip(text, at as usize).map_or(-1, |end| end as i64)
}

/// The value of the key `key` of the object that begins at `at`, when that value is a
/// string: it is kept for `koo_json_text`. Returns whether there is one. Nothing is
/// consumed: an enum reads its tag this way before it reads the object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_tag(text: *const Header, at: i64, key: *const Header) -> bool {
    let (bytes_of_text, key) = unsafe { (bytes(text), bytes(key)) };
    if at < 0 {
        return false;
    }
    let find = || -> Option<usize> {
        let mut i = json_space(bytes_of_text, at as usize);
        if bytes_of_text.get(i) != Some(&b'{') {
            return None;
        }
        i = json_space(bytes_of_text, i + 1);
        loop {
            let mut same = true;
            let mut seen = 0;
            let after = json_string(bytes_of_text, i, &mut |piece| {
                same = same && key.get(seen..seen + piece.len()) == Some(piece);
                seen += piece.len();
            })?;
            i = json_space(bytes_of_text, after);
            if bytes_of_text.get(i) != Some(&b':') {
                return None;
            }
            i = json_space(bytes_of_text, i + 1);
            if same && seen == key.len() {
                return (bytes_of_text.get(i) == Some(&b'"')).then_some(i);
            }
            i = json_space(bytes_of_text, json_skip(bytes_of_text, i)?);
            if bytes_of_text.get(i) != Some(&b',') {
                return None;
            }
            i = json_space(bytes_of_text, i + 1);
        }
    };
    match find() {
        Some(value) => unsafe { koo_json_string(text, value as i64) >= 0 },
        None => false,
    }
}

/// How deep the arrays and objects being read are nested. A text nested deeper than
/// `JSON_NEST_MAX` is refused, so that reading it cannot run out of stack.
const JSON_NEST_MAX: i64 = 128;

/// Begins reading a text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_begin() {
    unsafe {
        let me = thread::me();
        me.json_nest = 0;
        // What is read is anyone's, until the reader says it is another process's.
        me.json_wire = false;
    }
}

/// Says whether the JSON the thread writes from here on is for another process of the
/// program, which must read back exactly what was written, or for anyone.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_wire(on: bool) {
    unsafe { thread::me().json_wire = on };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_is_wire() -> bool {
    unsafe { thread::me().json_wire }
}

/// Goes one array or object deeper. Returns whether that is allowed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_enter() -> bool {
    unsafe {
        let me = thread::me();
        me.json_nest += 1;
        me.json_nest <= JSON_NEST_MAX
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_leave() {
    unsafe { thread::me().json_nest -= 1 };
}

/// The string as JSON writes it: in quotes, with what must be escaped escaped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_json_quote(s: *const Header) -> *mut Header {
    let text = unsafe { bytes(s) };
    let mut out = unsafe { koo_str_new(text.len() as i64 + 2) };
    let mut add = |piece: &[u8]| unsafe { koo_str_append_bytes(&mut out, piece.as_ptr(), piece.len() as i64) };
    add(b"\"");
    let mut run = 0;
    for (i, &byte) in text.iter().enumerate() {
        let escape: &[u8] = match byte {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            0x08 => b"\\b",
            0x0C => b"\\f",
            0..=0x1F => {
                add(&text[run..i]);
                const HEX: &[u8; 16] = b"0123456789abcdef";
                add(&[
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX[(byte >> 4) as usize],
                    HEX[(byte & 15) as usize],
                ]);
                run = i + 1;
                continue;
            }
            _ => continue,
        };
        add(&text[run..i]);
        add(escape);
        run = i + 1;
    }
    add(&text[run..]);
    add(b"\"");
    out
}

/// Whether a count marks a static value. Exposed for the tests.
pub fn is_immortal(rc: i64) -> bool {
    rc == IMMORTAL
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn text(s: *const Header) -> String {
        String::from_utf8(unsafe { bytes(s) }.to_vec()).unwrap()
    }

    #[test]
    fn strings_append_in_place_when_unique() {
        unsafe {
            let mut s = koo_str_new(0);
            let first = s;
            koo_str_append_bytes(&mut s, b"hello".as_ptr(), 5);
            koo_str_append_int(&mut s, -42);
            koo_str_append_bool(&mut s, true);
            assert_eq!(s, first, "a short append reuses the buffer");
            assert_eq!(text(s), "hello-42true");
            koo_free(s as *mut u8);
        }
    }

    #[test]
    fn appending_to_a_shared_string_copies() {
        unsafe {
            let mut a = str_from(b"abc");
            let original = a;
            (*a).rc = 2; // a second owner
            koo_str_append_bytes(&mut a, b"def".as_ptr(), 3);
            assert_ne!(a, original);
            assert_eq!(text(a), "abcdef");
            assert_eq!(text(original), "abc");
            assert_eq!((*original).rc, 1);
            koo_free(a as *mut u8);
            koo_free(original as *mut u8);
        }
    }

    #[test]
    fn strings_grow() {
        unsafe {
            let mut s = koo_str_new(0);
            for i in 0..1000 {
                koo_str_append_int(&mut s, i);
            }
            assert_eq!((*s).len, 2890);
            assert!(text(s).ends_with("998999"));
            koo_free(s as *mut u8);
        }
    }

    #[test]
    fn floats_read_as_floats() {
        unsafe {
            let mut s = koo_str_new(0);
            koo_str_append_float(&mut s, 1.0, -1);
            koo_str_append_bytes(&mut s, b" ".as_ptr(), 1);
            koo_str_append_float(&mut s, 0.1 + 0.2, -1);
            koo_str_append_bytes(&mut s, b" ".as_ptr(), 1);
            koo_str_append_float(&mut s, 2.0 / 3.0, 3);
            assert_eq!(text(s), "1.0 0.30000000000000004 0.667");
            koo_free(s as *mut u8);
        }
    }

    #[test]
    fn comparison_and_parsing() {
        unsafe {
            let a = str_from(b"apple");
            let b = str_from(b"banana");
            let a2 = koo_str_concat(str_from(b"app"), str_from(b"le"));
            assert!(koo_str_eq(a, a2));
            assert!(!koo_str_eq(a, b));
            assert_eq!(koo_str_cmp(a, b), -1);
            assert_eq!(koo_str_cmp(b, a), 1);
            assert_eq!(koo_str_cmp(a, a2), 0);
            let mut n = 0;
            assert!(koo_int_parse(str_from(b"-123"), &mut n));
            assert_eq!(n, -123);
            assert!(!koo_int_parse(str_from(b"12x"), &mut n));
            assert!(!koo_int_parse(str_from(b"99999999999999999999"), &mut n));
        }
    }

    #[test]
    fn json_is_read() {
        unsafe {
            let read = |source: &[u8]| -> Option<String> {
                let s = str_from(source);
                let end = koo_json_string(s, 0);
                koo_free(s as *mut u8);
                if end < 0 {
                    return None;
                }
                assert_eq!(end as usize, source.len());
                let kept = koo_json_text();
                let out = text(kept);
                koo_free(kept as *mut u8);
                Some(out)
            };
            assert_eq!(read(br#""plain""#).as_deref(), Some("plain"));
            assert_eq!(read(br#""a\"b\\c\/d\n\t""#).as_deref(), Some("a\"b\\c/d\n\t"));
            assert_eq!(read(br#""\u00e9\u20ac""#).as_deref(), Some("é€"));
            assert_eq!(read(br#""\ud83d\ude00""#).as_deref(), Some("😀"));
            assert_eq!(read(br#""\ud83d""#), None);
            assert_eq!(read(br#""\ude00""#), None);
            assert_eq!(read(br#""\x""#), None);
            assert_eq!(read(b"\"a\nb\""), None);
            assert_eq!(read(br#""open"#), None);
            assert_eq!(read(br#""\u12"#), None);
            assert_eq!(read(br#""\"#), None);

            let number = |source: &[u8]| -> Option<(bool, i64, f64)> {
                let s = str_from(source);
                let end = koo_json_number(s, 0);
                koo_free(s as *mut u8);
                (end == source.len() as i64).then(|| (koo_json_is_int(), koo_json_int(), koo_json_float()))
            };
            assert_eq!(number(b"0"), Some((true, 0, 0.0)));
            assert_eq!(number(b"-42"), Some((true, -42, -42.0)));
            assert_eq!(
                number(b"9223372036854775807"),
                Some((true, i64::MAX, 9223372036854775807.0))
            );
            assert_eq!(number(b"9223372036854775808"), Some((false, 0, 9223372036854775808.0)));
            assert_eq!(number(b"1.5"), Some((false, 0, 1.5)));
            assert_eq!(number(b"-2e3"), Some((false, 0, -2000.0)));
            assert_eq!(number(b"1E-2"), Some((false, 0, 0.01)));
            for bad in [&b"01"[..], b"-", b"1.", b".5", b"1e", b"+1", b"1e+", b"NaN", b""] {
                assert_eq!(number(bad), None, "{:?}", core::str::from_utf8(bad));
            }

            let skip = |source: &str| -> i64 {
                let s = str_from(source.as_bytes());
                let end = koo_json_skip(s, 0);
                koo_free(s as *mut u8);
                end
            };
            for good in [
                "1",
                "\"a\"",
                "true",
                "false",
                "null",
                "[]",
                "{}",
                "[1, 2, [3, {\"a\": [], \"b\": {}}], \"x\"]",
                "{\"a\": 1, \"b\": [true, null], \"c\": {\"d\": \"}\"}}",
                " [ 1 , 2 ] ",
            ] {
                assert_eq!(skip(good), good.trim_end().len() as i64, "{good}");
            }
            for bad in [
                "",
                "[",
                "[1,",
                "[1 2]",
                "[1,]",
                "{\"a\"}",
                "{\"a\":}",
                "{a: 1}",
                "{\"a\": 1,}",
                "[}",
                "{]",
                "tru",
                "nul",
            ] {
                assert_eq!(skip(bad), -1, "{bad}");
            }
            let deep = "[".repeat(JSON_DEPTH) + &"]".repeat(JSON_DEPTH);
            assert_eq!(skip(&deep), deep.len() as i64);
            let deeper = "[".repeat(JSON_DEPTH + 1) + &"]".repeat(JSON_DEPTH + 1);
            assert_eq!(skip(&deeper), -1);

            let tag = |source: &str| -> Option<String> {
                let (s, key) = (str_from(source.as_bytes()), str_from(b"type"));
                let found = koo_json_tag(s, 0, key);
                koo_free(s as *mut u8);
                koo_free(key as *mut u8);
                let kept = koo_json_text();
                let out = text(kept);
                koo_free(kept as *mut u8);
                found.then_some(out)
            };
            assert_eq!(tag("{\"type\": \"card\"}").as_deref(), Some("card"));
            assert_eq!(
                tag(" {\"n\": [1, {\"type\": \"no\"}], \"ty\\u0070e\": \"yes\"}").as_deref(),
                Some("yes")
            );
            assert_eq!(tag("{\"types\": \"a\", \"typ\": \"b\"}"), None);
            assert_eq!(tag("{\"type\": 3}"), None);
            assert_eq!(tag("{}"), None);
            assert_eq!(tag("[\"type\"]"), None);

            let quote = |source: &str| -> String {
                let s = str_from(source.as_bytes());
                let quoted = koo_json_quote(s);
                let out = text(quoted);
                koo_free(s as *mut u8);
                koo_free(quoted as *mut u8);
                out
            };
            assert_eq!(quote("plain"), "\"plain\"");
            assert_eq!(quote("a\"b\\c\nd\te\u{1}é"), "\"a\\\"b\\\\c\\nd\\te\\u0001é\"");
            assert_eq!(quote(""), "\"\"");
        }
    }

    #[test]
    fn lists_grow_geometrically() {
        unsafe {
            let mut list = koo_list_new(0, 8);
            list = koo_list_grow(list, 8, 1);
            assert_eq!((*list).cap, 4);
            list = koo_list_grow(list, 8, 5);
            assert_eq!((*list).cap, 8);
            list = koo_list_grow(list, 8, 100);
            assert_eq!((*list).cap, 100);
            koo_free(list as *mut u8);
            let zeroed = koo_list_zeroed(10, 8);
            assert_eq!((*zeroed).len, 10);
            assert_eq!(*((zeroed as *const u8).add(HEADER) as *const i64), 0);
            koo_free(zeroed as *mut u8);
        }
    }
}
