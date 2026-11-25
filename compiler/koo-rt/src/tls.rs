//! TLS, for the connections that `std/sql` makes: the system's OpenSSL, found and loaded
//! when a connection first asks for it.
//!
//! The library is not linked into a program, which would make every program need it. It
//! is looked for when wanted: `libssl.so.3`, or the one before it, on Linux, and the
//! one that Homebrew installs on macOS. A program that was linked statically cannot
//! load a library at all, and has no TLS; it says so when it is asked for.
//!
//! What a connection wants of the certificate the server shows is one of three things:
//! nothing (the connection is encrypted, and whoever is at the other end is believed),
//! that it was signed by an authority the system trusts, or one given in a file, or
//! that, and also that it is the certificate of the host that was asked for.

use super::thread::Lock;
use super::*;
use core::ffi::CStr;
use core::sync::atomic::Ordering::{Acquire, Release};
use core::sync::atomic::{AtomicPtr, AtomicU8};

unsafe extern "C" {
    /// Loads a library, or returns nothing: written by the compiler into each program,
    /// which knows whether the program can load one (`koo/src/main.rs`).
    fn koo_dlopen(name: *const u8) -> *mut u8;
    fn koo_dlsym(library: *mut u8, name: *const u8) -> *mut u8;
}

/// How much is asked of the server's certificate.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// Nothing: any certificate will do.
    Any,
    /// That an authority vouches for it.
    Authority,
    /// That an authority vouches for it, and that it names the host.
    Host,
}

type Ptr = *mut u8;

/// What the system says of a read or a write on a socket whose time to wait is up.
pub const AGAIN: i32 = if cfg!(target_os = "macos") { 35 } else { 11 };

/// The functions of OpenSSL that are used, in the order of `NAMES`.
#[repr(C)]
struct Api {
    tls_client_method: unsafe extern "C" fn() -> Ptr,
    ssl_ctx_new: unsafe extern "C" fn(Ptr) -> Ptr,
    ssl_ctx_free: unsafe extern "C" fn(Ptr),
    ssl_ctx_ctrl: unsafe extern "C" fn(Ptr, i32, i64, Ptr) -> i64,
    ssl_ctx_set_verify: unsafe extern "C" fn(Ptr, i32, Ptr),
    ssl_ctx_set_default_verify_paths: unsafe extern "C" fn(Ptr) -> i32,
    ssl_ctx_load_verify_locations: unsafe extern "C" fn(Ptr, *const u8, *const u8) -> i32,
    ssl_new: unsafe extern "C" fn(Ptr) -> Ptr,
    ssl_free: unsafe extern "C" fn(Ptr),
    ssl_set_fd: unsafe extern "C" fn(Ptr, i32) -> i32,
    ssl_ctrl: unsafe extern "C" fn(Ptr, i32, i64, Ptr) -> i64,
    ssl_set1_host: unsafe extern "C" fn(Ptr, *const u8) -> i32,
    ssl_connect: unsafe extern "C" fn(Ptr) -> i32,
    ssl_read: unsafe extern "C" fn(Ptr, *mut u8, i32) -> i32,
    ssl_write: unsafe extern "C" fn(Ptr, *const u8, i32) -> i32,
    ssl_shutdown: unsafe extern "C" fn(Ptr) -> i32,
    ssl_get_error: unsafe extern "C" fn(Ptr, i32) -> i32,
    ssl_get_verify_result: unsafe extern "C" fn(Ptr) -> i64,
    x509_verify_cert_error_string: unsafe extern "C" fn(i64) -> *const u8,
    err_get_error: unsafe extern "C" fn() -> u64,
    err_error_string_n: unsafe extern "C" fn(u64, *mut u8, usize),
}

const NAMES: [&CStr; 21] = [
    c"TLS_client_method",
    c"SSL_CTX_new",
    c"SSL_CTX_free",
    c"SSL_CTX_ctrl",
    c"SSL_CTX_set_verify",
    c"SSL_CTX_set_default_verify_paths",
    c"SSL_CTX_load_verify_locations",
    c"SSL_new",
    c"SSL_free",
    c"SSL_set_fd",
    c"SSL_ctrl",
    c"SSL_set1_host",
    c"SSL_connect",
    c"SSL_read",
    c"SSL_write",
    c"SSL_shutdown",
    c"SSL_get_error",
    c"SSL_get_verify_result",
    c"X509_verify_cert_error_string",
    c"ERR_get_error",
    c"ERR_error_string_n",
];

/// Where the library may be.
#[cfg(target_os = "macos")]
const PLACES: [&CStr; 5] = [
    c"/opt/homebrew/opt/openssl@3/lib/libssl.3.dylib",
    c"/usr/local/opt/openssl@3/lib/libssl.3.dylib",
    c"/opt/homebrew/lib/libssl.3.dylib",
    c"/usr/local/lib/libssl.3.dylib",
    c"/opt/local/lib/libssl.3.dylib",
];
#[cfg(not(target_os = "macos"))]
const PLACES: [&CStr; 2] = [c"libssl.so.3", c"libssl.so.1.1"];

static mut API: [Ptr; 21] = [core::ptr::null_mut(); 21];
/// 0: not looked for yet. 1: found. 2: not there.
static FOUND: AtomicU8 = AtomicU8::new(0);
static FINDING: Lock = Lock::new();
/// What is needed of OpenSSL to make connections, made once for each way of trusting:
/// the same settings serve every connection.
static CONTEXTS: [AtomicPtr<u8>; 3] = [const { AtomicPtr::new(core::ptr::null_mut()) }; 3];

/// The functions, if the library is there.
#[allow(static_mut_refs)]
unsafe fn api() -> Option<&'static Api> {
    unsafe {
        if FOUND.load(Acquire) == 0 {
            FINDING.lock();
            if FOUND.load(Acquire) == 0 {
                let mut found = 2;
                for place in PLACES {
                    let library = koo_dlopen(place.as_ptr() as *const u8);
                    if library.is_null() {
                        continue;
                    }
                    let mut whole = true;
                    for (slot, name) in API.iter_mut().zip(NAMES) {
                        *slot = koo_dlsym(library, name.as_ptr() as *const u8);
                        whole &= !slot.is_null();
                    }
                    if whole {
                        found = 1;
                        break;
                    }
                }
                FOUND.store(found, Release);
            }
            FINDING.unlock();
        }
        (FOUND.load(Acquire) == 1).then(|| &*(API.as_ptr() as *const Api))
    }
}

/// Whether this program can speak TLS.
pub unsafe fn available() -> bool {
    unsafe { api().is_some() }
}

/// Why it cannot, for the one who runs the program.
pub fn missing() -> &'static [u8] {
    if cfg!(target_os = "macos") {
        b"TLS needs OpenSSL, which was not found: install it with `brew install openssl@3`"
    } else {
        b"TLS needs the system's OpenSSL, `libssl.so.3`, which was not found, or cannot be loaded because the program was built with `--static`: install `libssl3`, or build the program without `--static`"
    }
}

/// A connection that is encrypted.
pub struct Tls {
    ssl: Ptr,
}

/// What OpenSSL last complained of, as text.
unsafe fn complaint(api: &Api, into: &mut [u8; 200]) -> usize {
    unsafe {
        let mut last = 0;
        loop {
            let code = (api.err_get_error)();
            if code == 0 {
                break;
            }
            last = code;
        }
        if last == 0 {
            return 0;
        }
        (api.err_error_string_n)(last, into.as_mut_ptr(), into.len());
        c_text(into.as_ptr()).len()
    }
}

/// Begins TLS on a socket that is connected, as a client of `host`. On failure, what
/// went wrong is written into `why`, and its length returned.
pub unsafe fn connect(
    fd: i32,
    host: &[u8],
    trust: Trust,
    authorities: &[u8],
    why: &mut [u8; 300],
) -> Result<*mut Tls, usize> {
    unsafe {
        let say = |why: &mut [u8; 300], parts: &[&[u8]]| {
            let mut at = 0;
            for part in parts {
                let take = part.len().min(why.len() - at);
                why[at..at + take].copy_from_slice(&part[..take]);
                at += take;
            }
            at
        };
        let Some(api) = api() else {
            return Err(say(why, &[missing()]));
        };
        let mut text = [0u8; 200];
        // The settings: TLS 1.2 at the least, and whom to believe. A file of
        // authorities gets settings of its own each time, since files differ.
        let kind = trust as usize;
        let mut context = if authorities.is_empty() {
            CONTEXTS[kind].load(Acquire)
        } else {
            core::ptr::null_mut()
        };
        let shared = !context.is_null();
        if context.is_null() {
            context = (api.ssl_ctx_new)((api.tls_client_method)());
            if context.is_null() {
                let len = complaint(api, &mut text);
                return Err(say(why, &[b"TLS could not be set up: ", &text[..len]]));
            }
            // 123 sets the oldest version that is spoken; 0x0303 is TLS 1.2.
            (api.ssl_ctx_ctrl)(context, 123, 0x0303, core::ptr::null_mut());
            if trust != Trust::Any {
                (api.ssl_ctx_set_verify)(context, 1, core::ptr::null_mut());
                let loaded = if authorities.is_empty() {
                    (api.ssl_ctx_set_default_verify_paths)(context)
                } else {
                    let mut path = [0u8; 1024];
                    let take = authorities.len().min(path.len() - 1);
                    path[..take].copy_from_slice(&authorities[..take]);
                    (api.ssl_ctx_load_verify_locations)(context, path.as_ptr(), core::ptr::null())
                };
                if loaded != 1 {
                    while (api.err_get_error)() != 0 {}
                    (api.ssl_ctx_free)(context);
                    return Err(if authorities.is_empty() {
                        say(why, &[b"the system's list of authorities to trust could not be read"])
                    } else {
                        say(
                            why,
                            &[
                                b"`",
                                authorities,
                                b"`, named by `sslrootcert`, is not a file of certificates that can be read",
                            ],
                        )
                    });
                }
            }
            if authorities.is_empty() {
                // Kept for the connections to come; if another thread was quicker, its
                // settings are the ones kept.
                match CONTEXTS[kind].compare_exchange(core::ptr::null_mut(), context, Release, Acquire) {
                    Ok(_) => {}
                    Err(kept) => {
                        (api.ssl_ctx_free)(context);
                        context = kept;
                    }
                }
            }
        }
        let keeps_context = shared || authorities.is_empty();
        let ssl = (api.ssl_new)(context);
        if !keeps_context {
            // The connection holds the settings for as long as it lives.
            (api.ssl_ctx_free)(context);
        }
        if ssl.is_null() {
            let len = complaint(api, &mut text);
            return Err(say(why, &[b"TLS could not be set up: ", &text[..len]]));
        }
        let mut name = [0u8; 256];
        let take = host.len().min(name.len() - 1);
        name[..take].copy_from_slice(&host[..take]);
        let numeric = host.iter().all(|byte| byte.is_ascii_digit() || *byte == b'.');
        (api.ssl_set_fd)(ssl, fd);
        if !numeric {
            // 55 names the host that is wanted, for a server that answers to several.
            (api.ssl_ctrl)(ssl, 55, 0, name.as_mut_ptr());
        }
        if trust == Trust::Host {
            (api.ssl_set1_host)(ssl, name.as_ptr());
        }
        let done = quietly(|| (api.ssl_connect)(ssl));
        if done != 1 {
            let refused = (api.ssl_get_verify_result)(ssl);
            let len = if trust != Trust::Any && refused != 0 {
                let reason = c_text((api.x509_verify_cert_error_string)(refused));
                while (api.err_get_error)() != 0 {}
                say(why, &[b"the server's certificate is not to be trusted: ", reason])
            } else {
                let slow = *errno_location() == AGAIN;
                let len = complaint(api, &mut text);
                let reason: &[u8] = match len {
                    0 if slow => b"the server did not answer in time",
                    0 => b"the server ended the connection",
                    _ => &text[..len],
                };
                say(why, &[b"TLS could not be begun: ", reason])
            };
            (api.ssl_free)(ssl);
            return Err(len);
        }
        let tls = malloc(core::mem::size_of::<Tls>()) as *mut Tls;
        if tls.is_null() {
            (api.ssl_free)(ssl);
            return Err(say(why, &[b"out of memory"]));
        }
        (*tls).ssl = ssl;
        Ok(tls)
    }
}

/// Reads what there is, up to the length of `into`. Zero is the end of the connection,
/// and a negative number an error.
pub unsafe fn read(tls: *mut Tls, into: &mut [u8]) -> isize {
    unsafe {
        let Some(api) = api() else { return -1 };
        loop {
            let length = into.len().min(i32::MAX as usize) as i32;
            let got = (api.ssl_read)((*tls).ssl, into.as_mut_ptr(), length);
            if got > 0 {
                return got as isize;
            }
            match (api.ssl_get_error)((*tls).ssl, got) {
                // The server said that it is done.
                6 => return 0,
                // More of the record is on its way; unless the socket was given a time
                // to wait and the time is up, which is -2.
                2 | 3 if *errno_location() == AGAIN => return -2,
                2 | 3 => continue,
                _ => {
                    while (api.err_get_error)() != 0 {}
                    return -1;
                }
            }
        }
    }
}

/// Writes all of `data`, and returns whether it could.
pub unsafe fn write_all(tls: *mut Tls, mut data: &[u8]) -> bool {
    unsafe {
        let Some(api) = api() else { return false };
        while !data.is_empty() {
            let length = data.len().min(i32::MAX as usize) as i32;
            let sent = quietly(|| (api.ssl_write)((*tls).ssl, data.as_ptr(), length));
            if sent > 0 {
                data = &data[sent as usize..];
                continue;
            }
            match (api.ssl_get_error)((*tls).ssl, sent) {
                2 | 3 if *errno_location() == AGAIN => return false,
                2 | 3 => continue,
                _ => {
                    while (api.err_get_error)() != 0 {}
                    return false;
                }
            }
        }
        true
    }
}

/// Ends TLS on a connection, with a word to the server if `polite`; the socket is the
/// caller's to close.
pub unsafe fn close(tls: *mut Tls, polite: bool) {
    unsafe {
        if tls.is_null() {
            return;
        }
        if let Some(api) = api() {
            if polite {
                quietly(|| (api.ssl_shutdown)((*tls).ssl));
            }
            (api.ssl_free)((*tls).ssl);
            while (api.err_get_error)() != 0 {}
        }
        free(tls as *mut u8);
    }
}

/// Runs what may write to a socket whose other end is gone. On Linux such a write
/// raises a signal that would end the program, and OpenSSL writes in its own way, so
/// the signal is held back for this thread meanwhile, and taken away if it was raised.
/// (On macOS the socket itself was told to raise none.)
#[cfg(target_os = "linux")]
unsafe fn quietly(work: impl FnOnce() -> i32) -> i32 {
    unsafe extern "C" {
        fn pthread_sigmask(how: i32, set: *const u64, old: *mut u64) -> i32;
        fn sigemptyset(set: *mut u64) -> i32;
        fn sigaddset(set: *mut u64, signal: i32) -> i32;
        fn sigpending(set: *mut u64) -> i32;
        fn sigismember(set: *const u64, signal: i32) -> i32;
        fn sigwait(set: *const u64, signal: *mut i32) -> i32;
    }
    const PIPE: i32 = 13;
    unsafe {
        // A set of signals is at most 1024 bits.
        let (mut pipe, mut before, mut waiting) = ([0u64; 16], [0u64; 16], [0u64; 16]);
        sigemptyset(pipe.as_mut_ptr());
        sigaddset(pipe.as_mut_ptr(), PIPE);
        sigpending(waiting.as_mut_ptr());
        let already = sigismember(waiting.as_ptr(), PIPE) == 1;
        pthread_sigmask(0, pipe.as_ptr(), before.as_mut_ptr());
        let result = work();
        // One that this write raised is taken; one that was waiting before is another's.
        sigpending(waiting.as_mut_ptr());
        if !already && sigismember(waiting.as_ptr(), PIPE) == 1 {
            let mut taken = 0;
            sigwait(pipe.as_ptr(), &mut taken);
        }
        pthread_sigmask(2, before.as_ptr(), core::ptr::null_mut());
        result
    }
}

#[cfg(not(target_os = "linux"))]
unsafe fn quietly(work: impl FnOnce() -> i32) -> i32 {
    work()
}

/// In the tests of this library there is no program to say how a library is loaded.
#[cfg(test)]
mod stand_in {
    #[unsafe(no_mangle)]
    pub extern "C" fn koo_dlopen(_name: *const u8) -> *mut u8 {
        core::ptr::null_mut()
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn koo_dlsym(_library: *mut u8, _name: *const u8) -> *mut u8 {
        core::ptr::null_mut()
    }
}
