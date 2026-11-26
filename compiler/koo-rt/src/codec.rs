//! What `std/hash`, `std/encoding`, `std/crypto` and `std/random` are made of: digests,
//! the ways of writing bytes as text, and numbers from the system's source of
//! randomness. A list of bytes is a list whose elements are one byte each.

use super::digest::{base64_decode, base64_encode, hex, hmac_sha256, pbkdf2_sha256, sha256};
use super::thread::me;
use super::*;

unsafe extern "C" {
    fn getentropy(data: *mut u8, len: usize) -> i32;
}

/// A new list of bytes.
unsafe fn new_bytes(data: &[u8]) -> *mut Header {
    unsafe {
        let list = koo_list_new(data.len() as i64, 1);
        core::ptr::copy_nonoverlapping(data.as_ptr(), (list as *mut u8).add(HEADER), data.len());
        (*list).len = data.len() as i64;
        list
    }
}

/// A new list of `len` bytes, for the caller to fill.
unsafe fn empty_bytes<'a>(len: usize) -> (*mut Header, &'a mut [u8]) {
    unsafe {
        let list = koo_list_new(len as i64, 1);
        (*list).len = len as i64;
        let data = core::slice::from_raw_parts_mut((list as *mut u8).add(HEADER), len);
        (list, data)
    }
}

// ---- digests ----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sha256(data: *const Header) -> *mut Header {
    unsafe { new_bytes(&sha256(bytes(data))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_hmac_sha256(key: *const Header, data: *const Header) -> *mut Header {
    unsafe { new_bytes(&hmac_sha256(bytes(key), &[bytes(data)])) }
}

/// PBKDF2 with HMAC-SHA-256: the key that a password and a salt give after `rounds`
/// rounds, which is what makes a stolen table of them slow to search.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_pbkdf2_sha256(password: *const Header, salt: *const Header, rounds: i64) -> *mut Header {
    unsafe {
        let rounds = rounds.clamp(1, u32::MAX as i64) as u32;
        new_bytes(&pbkdf2_sha256(bytes(password), bytes(salt), rounds))
    }
}

/// Whether two lists of bytes are the same, found out in a time that depends on their
/// lengths and not on where they differ, so that the time tells an attacker nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_bytes_same(a: *const Header, b: *const Header) -> bool {
    unsafe {
        let (a, b) = (bytes(a), bytes(b));
        let mut differ = (a.len() ^ b.len()) as u8 | ((a.len() ^ b.len()) >> 8 != 0) as u8;
        for index in 0..a.len().min(b.len()) {
            differ |= core::ptr::read_volatile(&a[index]) ^ core::ptr::read_volatile(&b[index]);
        }
        differ == 0
    }
}

// ---- bytes as text ----------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_hex(data: *const Header) -> *mut Header {
    unsafe {
        let data = bytes(data);
        let text = koo_str_new(data.len() as i64 * 2);
        hex(
            data,
            core::slice::from_raw_parts_mut((text as *mut u8).add(HEADER), data.len() * 2),
        );
        (*text).len = data.len() as i64 * 2;
        text
    }
}

fn hex_digit(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|digit| digit as u8)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_hex_valid(text: *const Header) -> bool {
    unsafe {
        let text = bytes(text);
        text.len().is_multiple_of(2) && text.iter().all(|byte| hex_digit(*byte).is_some())
    }
}

/// The bytes that hexadecimal text stands for; the text is valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_hex_decode(text: *const Header) -> *mut Header {
    unsafe {
        let text = bytes(text);
        let (list, out) = empty_bytes(text.len() / 2);
        for (byte, pair) in out.iter_mut().zip(text.as_chunks::<2>().0) {
            *byte = hex_digit(pair[0]).unwrap_or(0) << 4 | hex_digit(pair[1]).unwrap_or(0);
        }
        list
    }
}

/// Bytes as one process of a program writes them for another, and reads them: Base64,
/// which is a third longer than the bytes, where JSON's own form for them, an array
/// of numbers, is three and a half times as long and is read a number at a time.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_wire_bytes(data: *const Header) -> *mut Header {
    unsafe { koo_base64(data, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_wire_bytes_decode(text: *const Header) -> *mut Header {
    unsafe { koo_base64_decode(text, false) }
}

/// Base64. With `url`, the alphabet that is safe in an address, with `-` and `_`, and
/// no `=` at the end.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_base64(data: *const Header, url: bool) -> *mut Header {
    unsafe {
        let data = bytes(data);
        let longest = data.len().div_ceil(3) * 4;
        let text = koo_str_new(longest as i64);
        let out = core::slice::from_raw_parts_mut((text as *mut u8).add(HEADER), longest);
        let mut len = base64_encode(data, out);
        if url {
            while len > 0 && out[len - 1] == b'=' {
                len -= 1;
            }
            for byte in out[..len].iter_mut() {
                *byte = match *byte {
                    b'+' => b'-',
                    b'/' => b'_',
                    other => other,
                };
            }
        }
        (*text).len = len as i64;
        text
    }
}

/// The text as the standard alphabet has it, whichever of the two it was written in,
/// or nothing if it is not Base64 at all.
unsafe fn base64_bytes(text: &[u8], url: bool, out: &mut [u8]) -> Option<usize> {
    let text = match text {
        [rest @ .., b'=', b'='] | [rest @ .., b'='] if !url => rest,
        all => all,
    };
    // Four letters are three bytes, and what is left over is two letters or three.
    if text.len() % 4 == 1 || text.contains(&b'=') {
        return None;
    }
    let mut at = 0;
    for chunk in text.chunks(4) {
        let mut letters = [b'A'; 4];
        for (letter, byte) in letters.iter_mut().zip(chunk) {
            *letter = match *byte {
                b'-' if url => b'+',
                b'_' if url => b'/',
                b'+' | b'/' if url => return None,
                other => other,
            };
        }
        let mut three = [0u8; 3];
        base64_decode(&letters, &mut three)?;
        let kept = chunk.len() * 3 / 4;
        out.get_mut(at..at + kept)?.copy_from_slice(&three[..kept]);
        at += kept;
    }
    Some(at)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_base64_valid(text: *const Header, url: bool) -> bool {
    unsafe {
        let text = bytes(text);
        let (list, out) = empty_bytes(text.len() / 4 * 3 + 3);
        let valid = base64_bytes(text, url, out).is_some();
        koo_free(list as *mut u8);
        valid
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_base64_decode(text: *const Header, url: bool) -> *mut Header {
    unsafe {
        let text = bytes(text);
        let (list, out) = empty_bytes(text.len() / 4 * 3 + 3);
        (*list).len = base64_bytes(text, url, out).unwrap_or(0) as i64;
        list
    }
}

/// Text as it is written in a part of an address: letters, digits and `-._~` as they
/// are, and every other byte as `%` and two hexadecimal digits.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_url_encode(text: *const Header) -> *mut Header {
    unsafe {
        let text = bytes(text);
        let out = koo_str_new(text.len() as i64 * 3);
        let data = core::slice::from_raw_parts_mut((out as *mut u8).add(HEADER), text.len() * 3);
        let mut at = 0;
        for byte in text {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(byte) {
                data[at] = *byte;
                at += 1;
            } else {
                data[at] = b'%';
                data[at + 1] = b"0123456789ABCDEF"[(byte >> 4) as usize];
                data[at + 2] = b"0123456789ABCDEF"[(byte & 15) as usize];
                at += 3;
            }
        }
        (*out).len = at as i64;
        out
    }
}

/// The reverse, with `+` for a space as a form writes it; nothing if a `%` is not
/// followed by two digits or what results is not UTF-8.
unsafe fn url_decoded(text: &[u8], out: &mut [u8]) -> Option<usize> {
    let (mut at, mut from) = (0, 0);
    while from < text.len() {
        out[at] = match text[from] {
            b'%' => {
                let high = hex_digit(*text.get(from + 1)?)?;
                let low = hex_digit(*text.get(from + 2)?)?;
                from += 2;
                high << 4 | low
            }
            b'+' => b' ',
            other => other,
        };
        at += 1;
        from += 1;
    }
    core::str::from_utf8(&out[..at]).ok()?;
    Some(at)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_url_valid(text: *const Header) -> bool {
    unsafe {
        let text = bytes(text);
        let (list, out) = empty_bytes(text.len());
        let valid = url_decoded(text, out).is_some();
        koo_free(list as *mut u8);
        valid
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_url_decode(text: *const Header) -> *mut Header {
    unsafe {
        let text = bytes(text);
        let out = koo_str_new(text.len() as i64);
        let data = core::slice::from_raw_parts_mut((out as *mut u8).add(HEADER), text.len());
        (*out).len = url_decoded(text, data).unwrap_or(0) as i64;
        out
    }
}

// ---- randomness -------------------------------------------------------------------------

/// Fills `out` from the system's source of randomness, the one fit for keys. A thread
/// asks the system for 256 bytes at a time and hands them out; a process that was
/// forked asks afresh, so that it does not hand out what its parent did.
unsafe fn random(out: &mut [u8]) {
    unsafe {
        let thread = me();
        for byte in out {
            if thread.random_left == 0 {
                if getentropy(thread.random.as_mut_ptr(), thread.random.len()) != 0 {
                    fatal(b"koo: the system has no randomness to give\n");
                }
                thread.random_left = thread.random.len();
            }
            thread.random_left -= 1;
            *byte = core::mem::replace(&mut thread.random[thread.random_left], 0);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_random_bytes(count: i64) -> *mut Header {
    unsafe {
        let (list, out) = empty_bytes(count.max(0) as usize);
        random(out);
        list
    }
}

/// A number from zero up to, and not including, `below`, each as likely as another.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_random_below(below: i64) -> i64 {
    unsafe {
        let below = below.max(1) as u64;
        // The numbers that would make the low ones likelier are thrown away.
        let limit = u64::MAX - u64::MAX % below;
        loop {
            let mut eight = [0u8; 8];
            random(&mut eight);
            let number = u64::from_ne_bytes(eight);
            if number < limit {
                return (number % below) as i64;
            }
        }
    }
}

/// A number from `low` up to, and not including, `high`, each as likely as another,
/// however far apart the two are: the distance between them may be more than the
/// largest integer, and is counted without a sign.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_random_between(low: i64, high: i64) -> i64 {
    unsafe {
        let span = (high as u64).wrapping_sub(low as u64).max(1);
        let limit = u64::MAX - u64::MAX % span;
        loop {
            let mut eight = [0u8; 8];
            random(&mut eight);
            let number = u64::from_ne_bytes(eight);
            if number < limit {
                return low.wrapping_add((number % span) as i64);
            }
        }
    }
}

/// A number from zero up to, and not including, one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_random_float() -> f64 {
    unsafe {
        let mut eight = [0u8; 8];
        random(&mut eight);
        (u64::from_ne_bytes(eight) >> 11) as f64 / (1u64 << 53) as f64
    }
}
