//! A client for HTTP, behind `http.get` and the rest (`std/http`).
//!
//! A request is one connection: the host is found and connected to, with TLS if the
//! address begins `https`, the request is written, and the answer is read whole, by its
//! stated length, in chunks, or to the end of the connection, as the server chooses.
//! What the answer was is kept for the calling thread, which then asks for its parts.
//!
//! The whole of a request has a time it may take. Each wait, for the connection, for
//! the handshake, for the next piece of the answer, is given what is left of it.

use super::pg::{Buf, UNANSWERED, dial, has_something, send_all};
use super::thread::Lock;
use super::thread::me;
use super::tls::{self, AGAIN};
use super::*;
use core::sync::atomic::AtomicI32;
use core::sync::atomic::Ordering::Relaxed;

unsafe extern "C" {
    fn getpid() -> i32;
    fn close(fd: i32) -> i32;
    fn setsockopt(fd: i32, level: i32, name: i32, value: *const i32, len: u32) -> i32;
}

// The kinds of failure, as `std/http` numbers them.
const ADDRESS: i64 = 1;
const CONNECTION: i64 = 2;
const TIMEOUT: i64 = 3;
const PROTOCOL: i64 = 4;

/// The longest head of an answer, and the longest body, that is read.
const MOST_HEAD: usize = 1 << 16;
const MOST_BODY: usize = 1 << 28;

/// What a thread keeps of the last answer it was given.
struct Reply {
    status: i64,
    kind: i64,
    /// The headers, each as `name: value` and a newline, the names in lower case.
    headers: Buf,
    body: Buf,
}

unsafe fn reply<'a>() -> &'a mut Reply {
    unsafe {
        let thread = me();
        if thread.reply.is_null() {
            thread.reply = calloc(1, core::mem::size_of::<Reply>());
            if thread.reply.is_null() {
                fatal(b"koo: out of memory\n");
            }
        }
        &mut *(thread.reply as *mut Reply)
    }
}

/// Notes why a request failed, and returns false.
unsafe fn failed(kind: i64, parts: &[&[u8]]) -> bool {
    unsafe {
        reply().kind = kind;
        let length: usize = parts.iter().map(|part| part.len()).sum();
        let message = koo_str_new(length as i64);
        let mut at = (message as *mut u8).add(HEADER);
        for part in parts {
            core::ptr::copy_nonoverlapping(part.as_ptr(), at, part.len());
            at = at.add(part.len());
        }
        (*message).len = length as i64;
        failed_with(message);
        false
    }
}

// ---- connections that are kept ----------------------------------------------------------
//
// A connection that has carried a request and its answer can carry another, and making
// one is the dearest part of a request, most of all over TLS. So a connection is kept
// when its answer is whole, for the next request to the same server, for a minute.

/// The most connections that are kept, for all servers together.
const MOST_KEPT: usize = 32;
/// How long one is kept without being used, in nanoseconds.
const KEPT_FOR: i64 = 60_000_000_000;

#[derive(Clone, Copy)]
struct Kept {
    fd: i32,
    tls: *mut tls::Tls,
    /// Whose it is: `https` or not, the host, the port.
    key: [u8; 96],
    key_len: usize,
    /// When it was put by.
    since: i64,
}

const NO_KEPT: Kept = Kept {
    fd: -1,
    tls: core::ptr::null_mut(),
    key: [0; 96],
    key_len: 0,
    since: 0,
};

static mut KEPT: [Kept; MOST_KEPT] = [NO_KEPT; MOST_KEPT];
static KEEPING: Lock = Lock::new();
/// The process the kept connections belong to: one that was forked has none.
static KEEPER: AtomicI32 = AtomicI32::new(0);

/// What names a server's connections. Nothing for a host too long to be worth it.
fn key_of(secure: bool, host: &[u8], port: u16) -> Option<([u8; 96], usize)> {
    let mut key = [0u8; 96];
    if host.len() > 80 {
        return None;
    }
    key[0] = secure as u8;
    key[1..3].copy_from_slice(&port.to_be_bytes());
    key[3..3 + host.len()].copy_from_slice(host);
    key[3..3 + host.len()].make_ascii_lowercase();
    Some((key, 3 + host.len()))
}

/// In a process that was forked, the connections its parent kept are the parent's.
#[allow(static_mut_refs)]
unsafe fn kept_here() {
    unsafe {
        let pid = getpid();
        if KEEPER.load(Relaxed) != pid {
            if KEEPER.load(Relaxed) != 0 {
                KEEPING.unlock();
                for kept in KEPT.iter_mut() {
                    if kept.fd >= 0 {
                        tls::close(kept.tls, false);
                        close(kept.fd);
                    }
                    *kept = NO_KEPT;
                }
            }
            KEEPER.store(pid, Relaxed);
        }
    }
}

/// A connection to a server that was kept, if there is one that is still good: the one
/// put by last. One that the server has closed meanwhile, or that has waited too long,
/// is closed and passed over.
#[allow(static_mut_refs)]
unsafe fn take_kept(key: &[u8]) -> Option<(i32, *mut tls::Tls)> {
    unsafe {
        kept_here();
        let now = koo_now_nanos();
        KEEPING.lock();
        let mut found = None;
        loop {
            let newest = KEPT
                .iter_mut()
                .filter(|kept| kept.fd >= 0 && &kept.key[..kept.key_len] == key)
                .max_by_key(|kept| kept.since);
            let Some(kept) = newest else { break };
            let (fd, tls) = (kept.fd, kept.tls);
            let stale = now - kept.since > KEPT_FOR;
            *kept = NO_KEPT;
            if stale || has_something(fd) {
                tls::close(tls, false);
                close(fd);
                continue;
            }
            found = Some((fd, tls));
            break;
        }
        KEEPING.unlock();
        found
    }
}

/// Puts a connection by for the next request to its server. If there is no room, the
/// one that has waited longest makes it.
#[allow(static_mut_refs)]
unsafe fn keep(key: &[u8], fd: i32, tls: *mut tls::Tls) {
    unsafe {
        kept_here();
        KEEPING.lock();
        let slot = match KEPT.iter_mut().find(|kept| kept.fd < 0) {
            Some(free) => free,
            None => {
                let oldest = KEPT.iter_mut().min_by_key(|kept| kept.since).unwrap_unchecked();
                tls::close(oldest.tls, false);
                close(oldest.fd);
                oldest
            }
        };
        *slot = Kept {
            fd,
            tls,
            since: koo_now_nanos(),
            ..NO_KEPT
        };
        slot.key[..key.len()].copy_from_slice(key);
        slot.key_len = key.len();
        KEEPING.unlock();
    }
}

/// A connection to a server, encrypted or not, and when the request must be over.
struct Line {
    fd: i32,
    tls: *mut tls::Tls,
    /// Whether anything of an answer has been read.
    heard: bool,
    /// On the clock that only goes forward, in nanoseconds.
    deadline: i64,
    /// What has been read and not yet looked at.
    input: Buf,
    at: usize,
}

enum Ended {
    /// The server closed the connection.
    Closed,
    TimeUp,
    Broken,
}

impl Line {
    /// Gives the socket what is left of the time, for its next wait. False if none is.
    unsafe fn wait_at_most(&self) -> bool {
        unsafe {
            let left = self.deadline - koo_now_nanos();
            if left <= 0 {
                return false;
            }
            let time: [i64; 2] = [left / 1_000_000_000, (left % 1_000_000_000) / 1000];
            let (level, receive, send) = if cfg!(target_os = "macos") {
                (0xffff, 0x1006, 0x1005)
            } else {
                (1, 20, 21)
            };
            setsockopt(self.fd, level, receive, time.as_ptr() as *const i32, 16);
            setsockopt(self.fd, level, send, time.as_ptr() as *const i32, 16);
            true
        }
    }

    unsafe fn send(&self, data: &[u8]) -> bool {
        unsafe {
            if !self.wait_at_most() {
                return false;
            }
            if self.tls.is_null() {
                send_all(self.fd, data)
            } else {
                tls::write_all(self.tls, data)
            }
        }
    }

    /// Reads some more into `input`. Returns how much, which is never nothing.
    unsafe fn more(&mut self) -> Result<usize, Ended> {
        unsafe {
            loop {
                if !self.wait_at_most() {
                    return Err(Ended::TimeUp);
                }
                self.input.room(16384);
                let room = core::slice::from_raw_parts_mut(
                    self.input.ptr.add(self.input.len),
                    self.input.cap - self.input.len,
                );
                // A handler of a server lets the server do other work meanwhile.
                if (self.tls.is_null() || tls::pending(self.tls) == 0) && !fiber::wait_readable(self.fd, self.deadline)
                {
                    return Err(Ended::TimeUp);
                }
                let got = if self.tls.is_null() {
                    match read(self.fd, room.as_mut_ptr(), room.len()) {
                        n if n < 0 && *errno_location() == AGAIN => -2,
                        // Interrupted: once more.
                        n if n < 0 && *errno_location() == 4 => continue,
                        n => n,
                    }
                } else {
                    tls::read(self.tls, room)
                };
                return match got {
                    0 => Err(Ended::Closed),
                    -2 => Err(Ended::TimeUp),
                    n if n < 0 => Err(Ended::Broken),
                    n => {
                        self.heard = true;
                        self.input.len += n as usize;
                        Ok(n as usize)
                    }
                };
            }
        }
    }

    /// What is read and not yet looked at.
    fn unread(&self) -> &[u8] {
        &self.input.bytes()[self.at..]
    }

    /// Reads until a line is there, and returns it without its end. Nothing if the
    /// connection ends first, or the line is longer than a head may be.
    unsafe fn line(&mut self) -> Result<&[u8], Ended> {
        unsafe {
            loop {
                if let Some(end) = self.unread().iter().position(|byte| *byte == b'\n') {
                    let start = self.at;
                    self.at += end + 1;
                    let line = &self.input.bytes()[start..start + end];
                    return Ok(line.strip_suffix(b"\r").unwrap_or(line));
                }
                if self.unread().len() > MOST_HEAD {
                    return Err(Ended::Broken);
                }
                self.more()?;
            }
        }
    }

    /// Reads until `count` bytes are there, and moves them to `out`.
    unsafe fn take(&mut self, count: usize, out: &mut Buf) -> Result<(), Ended> {
        unsafe {
            while self.unread().len() < count {
                self.more()?;
            }
            out.push(&self.input.bytes()[self.at..self.at + count]);
            self.at += count;
            // What was looked at is not kept.
            if self.at == self.input.len {
                self.at = 0;
                self.input.len = 0;
            }
            Ok(())
        }
    }

    unsafe fn hang_up(&mut self) {
        unsafe {
            if !self.tls.is_null() {
                tls::close(self.tls, false);
            }
            close(self.fd);
            self.input.release();
        }
    }
}

/// The parts of an address: whether it is `https`, the host, the port, and the rest.
fn address(url: &[u8]) -> Option<(bool, &[u8], u16, &[u8])> {
    let (secure, rest) = match url {
        [b'h', b't', b't', b'p', b's', b':', b'/', b'/', rest @ ..] => (true, rest),
        [b'h', b't', b't', b'p', b':', b'/', b'/', rest @ ..] => (false, rest),
        _ => return None,
    };
    // Nothing that would end the first line of the request, or begin another.
    if url.iter().any(|byte| *byte <= b' ' || *byte == 127) {
        return None;
    }
    let rest = rest.split(|byte| *byte == b'#').next()?;
    let end = rest
        .iter()
        .position(|byte| *byte == b'/' || *byte == b'?')
        .unwrap_or(rest.len());
    let (place, path) = rest.split_at(end);
    if place.contains(&b'@') {
        return None;
    }
    // An address of the new kind is in brackets, and has colons of its own.
    let bracket = match (place.first(), place.iter().rposition(|byte| *byte == b']')) {
        (Some(b'['), Some(end)) => end,
        (Some(b'['), None) | (_, Some(_)) => return None,
        (_, None) => 0,
    };
    let (host, port) = match place[bracket..]
        .iter()
        .rposition(|byte| *byte == b':')
        .map(|at| at + bracket)
    {
        Some(at) => {
            let digits = &place[at + 1..];
            if digits.is_empty() || digits.len() > 5 || !digits.iter().all(u8::is_ascii_digit) {
                return None;
            }
            let port = digits.iter().fold(0u32, |sum, digit| sum * 10 + (digit - b'0') as u32);
            (&place[..at], u16::try_from(port).ok().filter(|port| *port > 0)?)
        }
        None => (place, if secure { 443 } else { 80 }),
    };
    // Colons belong to an address in brackets, and to nothing else in a host.
    (!host.is_empty() && (bracket > 0 || !host.contains(&b':'))).then_some((secure, host, port, path))
}

/// Whether a name or a value of a header can be sent as it is.
fn sendable(text: &[u8], name: bool) -> bool {
    !text.iter().any(|byte| *byte == b'\r' || *byte == b'\n' || *byte == 0)
        && !(name && (text.is_empty() || text.iter().any(|byte| *byte == b':' || *byte <= b' ')))
}

/// Makes a request and reads its answer. `headers` are names and values in turn.
/// Returns whether an answer came, whatever its status; if not, the thread has why.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_http_fetch(
    method: *const Header,
    url: *const Header,
    headers: *const Header,
    body: *const Header,
    nanos: i64,
) -> bool {
    unsafe {
        koo_flush();
        let (method, url, body) = (bytes(method), bytes(url), bytes(body));
        let kept = reply();
        (kept.status, kept.kind) = (0, 0);
        (kept.headers.len, kept.body.len) = (0, 0);
        let Some((secure, host, port, path)) = address(url) else {
            return failed(
                ADDRESS,
                &[b"`", url, b"` is not an address that can be asked: one is `http://host/path` or `https://host:port/path?query`"],
            );
        };
        if method.is_empty() || !method.iter().all(u8::is_ascii_uppercase) {
            return failed(
                ADDRESS,
                &[b"`", method, b"` is not a method: one is `GET`, `POST`, and so on"],
            );
        }
        let count = (*headers).len as usize;
        let given = (headers as *const u8).add(HEADER) as *const *const Header;
        let mut request = Buf::new();
        request.push(method);
        request.push(b" ");
        if !path.starts_with(b"/") {
            request.push(b"/");
        }
        request.push(path);
        request.push(b" HTTP/1.1\r\nHost: ");
        request.push(host);
        if port != if secure { 443 } else { 80 } {
            let mut number = Buffer::new();
            let _ = write!(number, ":{port}");
            request.push(&number.data[..number.len]);
        }
        request.push(b"\r\nAccept-Encoding: identity\r\n");
        let mut names_agent = false;
        let mut names_accept = false;
        for pair in 0..count / 2 {
            let (name, value) = (bytes(*given.add(2 * pair)), bytes(*given.add(2 * pair + 1)));
            if !sendable(name, true) || !sendable(value, false) {
                request.release();
                return failed(
                    ADDRESS,
                    &[
                        b"the header `",
                        name,
                        b"` cannot be sent: it has a line end in it, or no name",
                    ],
                );
            }
            names_agent |= name.eq_ignore_ascii_case(b"user-agent");
            names_accept |= name.eq_ignore_ascii_case(b"accept");
            // These three are the client's to say.
            if name.eq_ignore_ascii_case(b"host")
                || name.eq_ignore_ascii_case(b"content-length")
                || name.eq_ignore_ascii_case(b"connection")
            {
                continue;
            }
            request.push(name);
            request.push(b": ");
            request.push(value);
            request.push(b"\r\n");
        }
        if !names_agent {
            request.push(b"User-Agent: koo\r\n");
        }
        if !names_accept {
            request.push(b"Accept: */*\r\n");
        }
        if !body.is_empty() || matches!(method, b"POST" | b"PUT" | b"PATCH") {
            let mut length = Buffer::new();
            let _ = write!(length, "Content-Length: {}\r\n", body.len());
            request.push(&length.data[..length.len]);
        }
        request.push(b"\r\n");
        request.push(body);

        // A time that is used up before the request is made, as what is left of a
        // budget may be, is no time: the request is not made, and it is a timeout.
        if nanos <= 0 {
            request.release();
            return failed(TIMEOUT, &[host, b" was given no time to answer"]);
        }
        let started = koo_now_nanos();
        let deadline = started.saturating_add(nanos.max(1_000_000));
        let key = key_of(secure, host, port);
        // A request that may be made twice without harm is made again, once, if the
        // connection it went out on was one that had been kept and turned out to have
        // been closed: nothing of an answer came back on it.
        let again = matches!(method, b"GET" | b"HEAD" | b"PUT" | b"DELETE" | b"OPTIONS");
        let mut attempt = 0;
        let outcome = loop {
            attempt += 1;
            let kept_one = match &key {
                Some((key, len)) if attempt == 1 => take_kept(&key[..*len]),
                _ => None,
            };
            let mut line = Line {
                fd: -1,
                tls: core::ptr::null_mut(),
                heard: false,
                deadline,
                input: Buf::new(),
                at: 0,
            };
            match kept_one {
                Some((fd, tls)) => (line.fd, line.tls) = (fd, tls),
                None => {
                    let mut why = Buffer::new();
                    let left = (deadline - koo_now_nanos()) / 1_000_000;
                    line.fd = dial(host, port, left, &mut why);
                    if line.fd < 0 {
                        request.release();
                        if line.fd == UNANSWERED {
                            return failed(TIMEOUT, &[host, b" did not answer in the time it was given"]);
                        }
                        return failed(CONNECTION, &[&why.data[..why.len]]);
                    }
                    if secure {
                        line.wait_at_most();
                        let mut why = [0u8; 300];
                        match tls::connect(line.fd, host, tls::Trust::Host, b"", &mut why) {
                            Ok(made) => line.tls = made,
                            Err(len) => {
                                request.release();
                                line.hang_up();
                                let slow = why[..len].ends_with(b"in time");
                                return failed(if slow { TIMEOUT } else { CONNECTION }, &[&why[..len]]);
                            }
                        }
                    }
                }
            }
            let sent = line.send(request.bytes());
            let answered = if sent {
                answer(&mut line, method == b"HEAD", kept)
            } else {
                Err(Ended::Broken)
            };
            // An answer that is whole, on a connection the server will go on with,
            // leaves the connection good for another request.
            match (&answered, &key) {
                (Ok(true), Some((key, len))) if line.unread().is_empty() => {
                    keep(&key[..*len], line.fd, line.tls);
                    line.input.release();
                }
                _ => line.hang_up(),
            }
            let lost_unheard = !line.heard && matches!(answered, Err(Ended::Closed) | Err(Ended::Broken));
            if lost_unheard && kept_one.is_some() && (again || !sent) && koo_now_nanos() < deadline {
                continue;
            }
            break (answered, sent);
        };
        request.release();
        match outcome {
            (Ok(_), _) => true,
            (Err(Ended::TimeUp), _) => failed(TIMEOUT, &[host, b" did not answer in the time it was given"]),
            (Err(_), _) if koo_now_nanos() >= deadline => {
                failed(TIMEOUT, &[host, b" did not answer in the time it was given"])
            }
            (Err(Ended::Closed), _) => failed(CONNECTION, &[host, b" closed the connection before it had answered"]),
            (Err(Ended::Broken), false) => failed(CONNECTION, &[b"the connection to ", host, b" was lost"]),
            (Err(Ended::Broken), true) => failed(PROTOCOL, &[b"what ", host, b" answered is not HTTP"]),
        }
    }
}

/// Reads an answer: its first line, its headers, and its body.
///
/// Returns whether the connection is good for another request after it: the answer said
/// where it ends, and did not say that the connection ends with it.
unsafe fn answer(line: &mut Line, headless: bool, kept: &mut Reply) -> Result<bool, Ended> {
    unsafe {
        loop {
            // `HTTP/1.1 200 OK`
            let first = line.line()?;
            let mut words = first.split(|byte| *byte == b' ');
            let version = words.next().unwrap_or(b"");
            let status = words.next().unwrap_or(b"");
            if !version.starts_with(b"HTTP/1.") || status.len() != 3 || !status.iter().all(u8::is_ascii_digit) {
                return Err(Ended::Broken);
            }
            kept.status = status.iter().fold(0, |sum, digit| sum * 10 + (digit - b'0') as i64);
            kept.headers.len = 0;
            let (mut length, mut chunked) = (None, false);
            let mut goes_on = version != b"HTTP/1.0";
            let mut total = 0;
            loop {
                let header = line.line()?;
                if header.is_empty() {
                    break;
                }
                total += header.len();
                let colon = header.iter().position(|byte| *byte == b':').ok_or(Ended::Broken)?;
                if total > MOST_HEAD {
                    return Err(Ended::Broken);
                }
                let (name, value) = (&header[..colon], header[colon + 1..].trim_ascii());
                if name.eq_ignore_ascii_case(b"content-length") {
                    let digits = value.iter().all(u8::is_ascii_digit) && !value.is_empty() && value.len() < 16;
                    if !digits {
                        return Err(Ended::Broken);
                    }
                    length = Some(
                        value
                            .iter()
                            .fold(0usize, |sum, digit| sum * 10 + (digit - b'0') as usize),
                    );
                }
                if name.eq_ignore_ascii_case(b"connection") && value.eq_ignore_ascii_case(b"close") {
                    goes_on = false;
                }
                if name.eq_ignore_ascii_case(b"transfer-encoding") {
                    chunked = value.to_ascii_lowercase_ends_with(b"chunked");
                }
                // The name in lower case, so that it is found however it was written.
                let start = kept.headers.len;
                kept.headers.push(name);
                core::slice::from_raw_parts_mut(kept.headers.ptr.add(start), name.len()).make_ascii_lowercase();
                kept.headers.push(b": ");
                kept.headers.push(value);
                kept.headers.push(b"\n");
            }
            // An answer that only says to go on is followed by the answer itself.
            if (100..200).contains(&kept.status) {
                continue;
            }
            kept.body.len = 0;
            if headless || kept.status == 204 || kept.status == 304 {
                return Ok(goes_on);
            }
            if chunked {
                loop {
                    let size = line.line()?;
                    let digits = size.split(|byte| *byte == b';').next().unwrap_or(b"").trim_ascii();
                    if digits.is_empty() || digits.len() > 8 {
                        return Err(Ended::Broken);
                    }
                    let mut count = 0usize;
                    for digit in digits {
                        count = count * 16 + (*digit as char).to_digit(16).ok_or(Ended::Broken)? as usize;
                    }
                    if kept.body.len + count > MOST_BODY {
                        return Err(Ended::Broken);
                    }
                    if count == 0 {
                        // After the last chunk there may be more headers, which are of
                        // no use here, up to an empty line.
                        while !line.line()?.is_empty() {}
                        return Ok(goes_on);
                    }
                    line.take(count, &mut kept.body)?;
                    if !line.line()?.is_empty() {
                        return Err(Ended::Broken);
                    }
                }
            }
            return match length {
                Some(length) if length > MOST_BODY => Err(Ended::Broken),
                Some(length) => line.take(length, &mut kept.body).map(|()| goes_on),
                // Neither a length nor chunks: the body is all there is until the end.
                None => loop {
                    let waiting = line.unread().len();
                    line.take(waiting, &mut kept.body)?;
                    if kept.body.len > MOST_BODY {
                        break Err(Ended::Broken);
                    }
                    match line.more() {
                        Ok(_) => {}
                        Err(Ended::Closed) => break Ok(false),
                        Err(other) => break Err(other),
                    }
                },
            };
        }
    }
}

trait EndsWith {
    fn to_ascii_lowercase_ends_with(&self, end: &[u8]) -> bool;
}

impl EndsWith for [u8] {
    fn to_ascii_lowercase_ends_with(&self, end: &[u8]) -> bool {
        self.len() >= end.len() && self[self.len() - end.len()..].eq_ignore_ascii_case(end)
    }
}

/// The status of the last answer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_http_reply_status() -> i64 {
    unsafe { reply().status }
}

/// What kind of failure the last one was.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_http_reply_kind() -> i64 {
    unsafe { reply().kind }
}

/// The headers of the last answer: a line for each, `name: value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_http_reply_headers() -> *mut Header {
    unsafe { str_from_outside(reply().headers.bytes()) }
}

/// Whether the body of the last answer is text, in UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_http_reply_text() -> bool {
    unsafe { core::str::from_utf8(reply().body.bytes()).is_ok() }
}

/// The body of the last answer as the bytes it is. What is returned is not text that a
/// program may have: the prelude makes a list of bytes of it at once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_http_reply_raw() -> *mut Header {
    unsafe {
        let body = reply().body.bytes();
        crate::koo_str_from_bytes(body.as_ptr(), body.len() as i64)
    }
}

/// The body of the last answer, which the thread then no longer keeps.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_http_reply_body() -> *mut Header {
    unsafe {
        let kept = reply();
        let body = str_from_outside(kept.body.bytes());
        kept.body.release();
        body
    }
}

/// When `main` returns: what each thread kept of its last answer is given back.
#[allow(static_mut_refs)]
pub unsafe fn at_exit() {
    unsafe {
        if KEEPER.load(Relaxed) == getpid() {
            for kept in KEPT.iter_mut() {
                if kept.fd >= 0 {
                    tls::close(kept.tls, true);
                    close(kept.fd);
                }
                *kept = NO_KEPT;
            }
        }
        thread::every(|thread| {
            if !thread.reply.is_null() {
                let kept = &mut *(thread.reply as *mut Reply);
                kept.headers.release();
                kept.body.release();
                free(thread.reply);
                thread.reply = core::ptr::null_mut();
            }
        });
    }
}
