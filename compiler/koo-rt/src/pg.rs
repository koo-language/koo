//! A client for PostgreSQL, behind `std/sql` (Section 10.3).
//!
//! `koo_sql_open` takes the address of a database and keeps it. A query checks a
//! connection out of the database's pool, making one if all are in use and there may be
//! more, sends the statement with its parameters apart from its text, reads everything
//! the server answers into what the calling thread keeps of its last query, and gives
//! the connection back. The program then asks for the rows, a value at a time.
//!
//! A connection belongs to the process that made it. A process that was forked from
//! one with connections, as a worker of a server is, has none: it makes its own.
//!
//! What is here of the protocol: the start-up message, the three ways PostgreSQL asks
//! for a password (in the clear, as an MD5 digest, and by SCRAM-SHA-256), the request
//! for encryption, which `tls.rs` then provides, and the extended query messages, with
//! every parameter and every value as text.

use super::digest::{base64_decode, base64_encode, hex, hmac_sha256, md5, pbkdf2_sha256, sha256};
use super::thread::{Gate, me};
use super::*;
use core::fmt::Write;
use core::sync::atomic::Ordering::{Acquire, Release};
use core::sync::atomic::{AtomicI32, AtomicU32};

unsafe extern "C" {
    fn socket(domain: i32, kind: i32, protocol: i32) -> i32;
    fn connect(fd: i32, address: *const u8, len: u32) -> i32;
    fn close(fd: i32) -> i32;
    fn getpid() -> i32;
    fn setsockopt(fd: i32, level: i32, name: i32, value: *const i32, len: u32) -> i32;
    fn sendto(fd: i32, data: *const u8, len: usize, flags: i32, address: *const u8, address_len: u32) -> isize;
    fn recv(fd: i32, data: *mut u8, len: usize, flags: i32) -> isize;
    fn recvfrom(fd: i32, data: *mut u8, len: usize, flags: i32, from: *mut u8, from_len: *mut u32) -> isize;
    fn poll(fds: *mut PollFd, count: usize, milliseconds: i32) -> i32;
    fn getentropy(data: *mut u8, len: usize) -> i32;
    fn inet_pton(family: i32, text: *const u8, address: *mut u8) -> i32;
    fn fcntl(fd: i32, command: i32, ...) -> i32;
    fn getsockopt(fd: i32, level: i32, name: i32, value: *mut i32, len: *mut u32) -> i32;
    fn getpeername(fd: i32, address: *mut u8, len: *mut u32) -> i32;
    fn pipe(ends: *mut i32) -> i32;
    fn read(fd: i32, data: *mut u8, len: usize) -> isize;
    fn write(fd: i32, data: *const u8, len: usize) -> isize;
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const AF_INET: i32 = 2;
const AF_INET6: i32 = if cfg!(target_os = "macos") { 30 } else { 10 };

/// An address of the Internet, of the old kind or the new.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Ip {
    V4([u8; 4]),
    V6([u8; 16]),
}
const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;
const IPPROTO_TCP: i32 = 6;
const TCP_NODELAY: i32 = 1;
const EINTR: i32 = 4;

/// The most databases a program has open, and the most connections to one.
const MOST_DBS: usize = 16;
const MOST_CONNS: usize = 64;

// What kind of failure the last one was, as `std/sql` numbers them.
const OTHER: i64 = 0;
const CONNECTION: i64 = 1;
const CONSTRAINT: i64 = 2;
const SYNTAX: i64 = 3;
const SERIALIZATION: i64 = 4;
const TIMEOUT: i64 = 5;
/// What a statement fails with that a handler began after its request was given up.
const GIVEN_UP: &[u8] = b"the request this was for was given up: its client has left, or its time is over";

/// Whether an error's code says that the server is closing the connection, or will not
/// open one: the classes 08 and 28, a database that is not there, and the shutdowns.
fn gone(code: &[u8]) -> bool {
    matches!(code.get(..2), Some(b"08") | Some(b"28") | Some(b"3D")) || code.starts_with(b"57P")
}

/// Bytes that grow.
pub(crate) struct Buf {
    pub(crate) ptr: *mut u8,
    pub(crate) len: usize,
    pub(crate) cap: usize,
}

impl Buf {
    pub(crate) const fn new() -> Buf {
        Buf {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }

    pub(crate) unsafe fn room(&mut self, more: usize) {
        unsafe {
            if self.len + more > self.cap {
                let cap = (self.len + more).max(self.cap * 2).max(256);
                self.ptr = realloc(self.ptr, cap);
                if self.ptr.is_null() {
                    fatal(b"koo: out of memory\n");
                }
                self.cap = cap;
            }
        }
    }

    pub(crate) unsafe fn push(&mut self, bytes: &[u8]) {
        unsafe {
            self.room(bytes.len());
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.add(self.len), bytes.len());
        }
        self.len += bytes.len();
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        if self.ptr.is_null() {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
        }
    }

    pub(crate) unsafe fn release(&mut self) {
        unsafe { free(self.ptr) };
        *self = Buf::new();
    }
}

/// A message to the server: its kind, its length, which is filled in when it ends, and
/// what it says.
struct Message<'a> {
    out: &'a mut Buf,
    start: usize,
}

impl Message<'_> {
    unsafe fn begin(out: &mut Buf, kind: u8) -> Message<'_> {
        unsafe {
            if kind != 0 {
                out.push(&[kind]);
            }
            let start = out.len;
            out.push(&[0; 4]);
            Message { out, start }
        }
    }

    unsafe fn bytes(&mut self, bytes: &[u8]) {
        unsafe { self.out.push(bytes) };
    }

    unsafe fn text(&mut self, text: &[u8]) {
        unsafe {
            self.out.push(text);
            self.out.push(&[0]);
        }
    }

    unsafe fn int32(&mut self, value: i32) {
        unsafe { self.out.push(&value.to_be_bytes()) };
    }

    unsafe fn int16(&mut self, value: i16) {
        unsafe { self.out.push(&value.to_be_bytes()) };
    }

    unsafe fn end(self) {
        let length = (self.out.len - self.start) as u32;
        unsafe {
            core::ptr::copy_nonoverlapping(length.to_be_bytes().as_ptr(), self.out.ptr.add(self.start), 4);
        }
    }
}

struct Conn {
    fd: i32,
    /// Whether a thread has it, or is making it.
    busy: bool,
    /// The transaction that is open on it, counted from one for each connection, or
    /// zero; and the thread that began it, which alone may use it.
    transaction: u32,
    began: usize,
    serial: u32,
    /// What has been read from the server and not yet looked at.
    input: Buf,
    at: usize,
    /// The encryption of the connection, if it has one (`tls.rs`).
    tls: *mut tls::Tls,
    /// While the connection is being set up: when the server must have answered, on
    /// the clock that only goes forward. Zero once it is ready.
    until: i64,
    /// What the server said of itself when the connection was made: the number of the
    /// process that serves it there and a key, by which the server is told to stop a
    /// statement (`stop_statement`). And whether it said so.
    backend: [u8; 8],
    known: bool,
    /// Whether a statement is with the server and its answer has not all come.
    asked: bool,
}

struct Db {
    used: bool,
    host: Buf,
    port: u16,
    user: Buf,
    password: Buf,
    database: Buf,
    /// Settings for each session, as `options` in the address gives them.
    options: Buf,
    /// Whether a connection is encrypted, as `sslmode` in the address says: not at all
    /// (0), if the server will (1), always (2), always and with a certificate that an
    /// authority vouches for (3), and that names the host as well (4).
    secure: u8,
    /// A file of the authorities to trust, in place of the system's.
    authorities: Buf,
    limit: usize,
    /// How long, in milliseconds, a statement waits for a connection when all are in use.
    patience: i64,
    /// Those that wait for one, who have it in the order they came.
    turns: thread::Turns,
    conns: [Conn; MOST_CONNS],
}

const NO_CONN: Conn = Conn {
    fd: -1,
    busy: false,
    transaction: 0,
    began: 0,
    serial: 0,
    input: Buf::new(),
    at: 0,
    tls: core::ptr::null_mut(),
    until: 0,
    backend: [0; 8],
    known: false,
    asked: false,
};

const NO_DB: Db = Db {
    used: false,
    host: Buf::new(),
    port: 5432,
    user: Buf::new(),
    password: Buf::new(),
    database: Buf::new(),
    options: Buf::new(),
    secure: 1,
    authorities: Buf::new(),
    limit: 0,
    patience: 30_000,
    turns: thread::Turns::new(),
    conns: [NO_CONN; MOST_CONNS],
};

static mut DBS: [Db; MOST_DBS] = [NO_DB; MOST_DBS];
/// Held to choose a connection; where a thread sleeps that waits for one.
static POOL: Gate = Gate::new();
static OPENED: AtomicU32 = AtomicU32::new(0);
/// The process the connections belong to.
static OWNER: AtomicI32 = AtomicI32::new(0);

/// What a thread keeps of its last query: the rows, or why there are none.
struct Outcome {
    error: Buf,
    code: [u8; 5],
    kind: i64,
    /// The names of the columns, each ended by a zero.
    names: Buf,
    cols: usize,
    rows: usize,
    /// For each value, where it is in `data` and how long, four bytes each; a length of
    /// -1 is NULL.
    cells: Buf,
    data: Buf,
    affected: i64,
    /// Whether the server answered a statement with the word that it rolled back.
    rolled_back: bool,
}

unsafe fn outcome<'a>() -> &'a mut Outcome {
    unsafe {
        let thread = me();
        if thread.sql.is_null() {
            thread.sql = calloc(1, core::mem::size_of::<Outcome>());
            if thread.sql.is_null() {
                fatal(b"koo: out of memory\n");
            }
        }
        &mut *(thread.sql as *mut Outcome)
    }
}

unsafe fn failed(kind: i64, parts: &[&[u8]]) {
    unsafe {
        let out = outcome();
        out.kind = kind;
        out.code = *b"     ";
        out.error.len = 0;
        for part in parts {
            out.error.push(part);
        }
    }
}

unsafe fn system_error() -> &'static [u8] {
    unsafe { c_text(strerror(*errno_location())) }
}

// ---- finding the server -----------------------------------------------------------------

/// `a.b.c.d` as an address.
fn dotted(name: &[u8]) -> Option<[u8; 4]> {
    let mut address = [0u8; 4];
    let mut parts = name.split(|byte| *byte == b'.');
    for part in &mut address {
        let digits = parts.next()?;
        if digits.is_empty() || digits.len() > 3 || !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let value = digits.iter().fold(0u32, |sum, digit| sum * 10 + (digit - b'0') as u32);
        *part = u8::try_from(value).ok()?;
    }
    parts.next().is_none().then_some(address)
}

/// A host that is written as an address and not as a name: `a.b.c.d`, or an address of
/// the new kind, which an address of a server puts in brackets.
unsafe fn literal(name: &[u8]) -> Option<Ip> {
    unsafe {
        if let Some(address) = dotted(name) {
            return Some(Ip::V4(address));
        }
        let name = match name {
            [b'[', inside @ .., b']'] => inside,
            name => name,
        };
        let mut text = [0u8; 64];
        if !name.contains(&b':') || name.len() >= text.len() || name.contains(&0) {
            return None;
        }
        text[..name.len()].copy_from_slice(name);
        let mut address = [0u8; 16];
        (inet_pton(AF_INET6, text.as_ptr(), address.as_mut_ptr()) == 1).then_some(Ip::V6(address))
    }
}

/// What a file of the system's settings holds, or nothing if it cannot be read. Of one
/// longer than 4 MB, the whole lines of the first 4 MB: half a line would be read as a
/// line that says something else.
unsafe fn file(path: &core::ffi::CStr) -> Buf {
    unsafe {
        const MOST: usize = 4 << 20;
        let mut out = Buf::new();
        let file = fopen(path.as_ptr() as *const u8, c"r".as_ptr() as *const u8);
        if file.is_null() {
            return out;
        }
        loop {
            out.room(65536);
            let got = fread(out.ptr.add(out.len), 1, 65536, file);
            out.len += got;
            if got < 65536 {
                break;
            }
            if out.len >= MOST {
                while out.len > 0 && *out.ptr.add(out.len - 1) != b'\n' {
                    out.len -= 1;
                }
                break;
            }
        }
        fclose(file);
        out
    }
}

/// The words of a line of a configuration file, without what follows a `#`.
fn words(line: &[u8]) -> impl Iterator<Item = &[u8]> {
    let line = line.split(|byte| *byte == b'#').next().unwrap_or(&[]);
    line.split(|byte| byte.is_ascii_whitespace())
        .filter(|word| !word.is_empty())
}

/// Asks the name servers for the address of a name, of the old kind (`kind` 1) or the
/// new (28), all of them at once on one socket, and takes the first answer that is one:
/// an address, or the word that the name has none. An answer is one only if it comes
/// from a server that was asked, says that it is an answer, and repeats the question;
/// anything else on the socket is not listened to. A server that refuses, fails, or
/// does not understand has not answered, and the others are waited for; one that is
/// silent holds nothing up. An answer too long for the one datagram is asked for again
/// over a connection, and a server that did not understand the question is asked again
/// without the word about longer answers, which is what an old one stumbles on.
///
/// Returns nothing if no server answered by `until` (0 for no limit beyond the two
/// seconds, twice, that a question is given), and otherwise the address, or that there
/// is none.
unsafe fn ask_name_servers(servers: &[Ip], name: &[u8], kind: u8, until: i64) -> Option<Option<Ip>> {
    unsafe {
        // The question: the name as labels, each with its length before it, and after
        // it the word that an answer may be as long as 1232 bytes.
        let mut query = [0u8; 320];
        let mut id = [0u8; 2];
        getentropy(id.as_mut_ptr(), 2);
        query[..2].copy_from_slice(&id);
        query[2] = 1; // the server may ask others
        query[5] = 1; // one question
        let mut at = 12;
        for label in name.split(|byte| *byte == b'.').filter(|label| !label.is_empty()) {
            // A name that cannot be asked for is a name with no address.
            if label.len() > 63 || at + label.len() + 17 > query.len() {
                return Some(None);
            }
            query[at] = label.len() as u8;
            query[at + 1..at + 1 + label.len()].copy_from_slice(label);
            at += 1 + label.len();
        }
        // The end of the name, then: an address, of the Internet.
        query[at + 2] = kind;
        query[at + 4] = 1;
        let question = at + 5;
        // No name, the kind 41, and the length this end can take.
        query[question + 2] = 41;
        query[question + 3..question + 5].copy_from_slice(&1232u16.to_be_bytes());
        let late = || until != 0 && koo_now_nanos() >= until;
        // One socket for them all: where a server is at an address of the new kind, the
        // socket is of that kind, and reaches the others at their addresses written so.
        let mut fd = -1;
        if servers.iter().any(|server| matches!(server, Ip::V6(_))) {
            fd = socket(AF_INET6, SOCK_DGRAM, 0);
            if fd >= 0 {
                let both = 0i32;
                let only_new = if cfg!(target_os = "macos") { 27 } else { 26 };
                setsockopt(fd, 41, only_new, &both, 4);
            }
        }
        let new_kind = fd >= 0;
        if fd < 0 {
            fd = socket(AF_INET, SOCK_DGRAM, 0);
        }
        if fd < 0 {
            return None;
        }
        let reached = |server: &Ip| -> Option<([u8; 28], u32)> {
            match (server, new_kind) {
                (Ip::V4(old), true) => {
                    let mut written = [0u8; 16];
                    (written[10], written[11]) = (0xff, 0xff);
                    written[12..].copy_from_slice(old);
                    Some(socket_address(Ip::V6(written), 53))
                }
                (Ip::V6(_), false) => None,
                (server, _) => Some(socket_address(*server, 53)),
            }
        };
        let most = servers.len().min(32);
        // The servers that have said they will not help, and of those the ones that did
        // not understand the question.
        let (mut unhelpful, mut puzzled) = (0u32, 0u32);
        let mut found = None;
        'asking: for round in 0..2 {
            // The second time without the word about longer answers.
            let plain = round == 1;
            query[11] = u8::from(!plain);
            let length = if plain { question } else { question + 11 };
            let mut asked = 0u32;
            for (which, server) in servers[..most].iter().enumerate() {
                let again = unhelpful & (1 << which) == 0 || puzzled & (1 << which) != 0;
                if let (true, Some((address, address_len))) = (again, reached(server))
                    && sendto(fd, query.as_ptr(), length, 0, address.as_ptr(), address_len) >= 0
                {
                    asked |= 1 << which;
                }
            }
            (unhelpful, puzzled) = (unhelpful & !asked, 0);
            // A handler of a server lets the server go on while the name servers think:
            // for two seconds, or for what is left of the caller's time.
            let patience = koo_now_nanos() + 2_000_000_000;
            let patience = if until != 0 { patience.min(until) } else { patience };
            // (The time is looked at before each datagram as well as while waiting for
            // one: a stream of them that are no answers is not waited through.)
            while asked & !unhelpful != 0 && koo_now_nanos() < patience && ready(fd, false, patience) {
                let mut answer = [0u8; 1500];
                let mut from = [0u8; 28];
                let mut from_len = from.len() as u32;
                let got = recvfrom(
                    fd,
                    answer.as_mut_ptr(),
                    answer.len(),
                    0,
                    from.as_mut_ptr(),
                    &mut from_len,
                );
                // From a server that was asked, an answer, to this question.
                let who = servers[..most].iter().position(|server| {
                    // The port, and the address, which lies further in where it is longer.
                    let part = if new_kind { 8..24 } else { 4..8 };
                    reached(server).is_some_and(|(address, _)| {
                        address[2..4] == from[2..4] && address[part.clone()] == from[part.clone()]
                    })
                });
                let Some(who) = who.filter(|who| asked & (1 << who) != 0) else {
                    continue;
                };
                if got < question as isize
                    || answer[..2] != id
                    || answer[2] & 0x80 == 0
                    || answer[4..6] != [0, 1]
                    || !answer[12..question].eq_ignore_ascii_case(&query[12..question])
                {
                    continue;
                }
                let mut answer = &answer[..got as usize];
                // What did not fit is asked for again of the same server, over a
                // connection, where it fits.
                let mut whole = [0u8; 16384];
                if answer[2] & 2 != 0 {
                    let got = ask_over_a_connection(servers[who], &query[..length], &mut whole, patience);
                    if got >= question && whole[..2] == id {
                        answer = &whole[..got];
                    } else if !matches!(address_in(answer, kind), Ok(Some(_))) {
                        // The whole of it could not be had, and the part holds no
                        // address: that server has not answered.
                        unhelpful |= 1 << who;
                        continue;
                    }
                }
                // 0 is an answer, and 3 the word that there is no such name. The rest is
                // a server that failed, would not say, or did not understand (1 and 4).
                let said = answer[3] & 15;
                match address_in(answer, kind) {
                    Ok(Some(address)) => {
                        found = Some(Some(address));
                        break 'asking;
                    }
                    Ok(None) if said == 0 || said == 3 => {
                        found = Some(None);
                        break 'asking;
                    }
                    _ => {
                        unhelpful |= 1 << who;
                        if said == 1 || said == 4 {
                            puzzled |= 1 << who;
                        }
                    }
                }
            }
            // Nobody is left to ask again: every server that was asked has said that it
            // will not help, and none of them for want of understanding.
            if asked == 0 || late() || (asked & !unhelpful == 0 && puzzled == 0) {
                break;
            }
        }
        close(fd);
        found
    }
}

/// Asks one name server over a connection, as is done for an answer that is too long
/// for a datagram: the question with its length before it, and the answer likewise.
/// Returns how long the answer is, or 0.
unsafe fn ask_over_a_connection(server: Ip, query: &[u8], answer: &mut [u8], until: i64) -> usize {
    unsafe {
        let fd = socket(
            if matches!(server, Ip::V6(_)) { AF_INET6 } else { AF_INET },
            SOCK_STREAM,
            0,
        );
        if fd < 0 {
            return 0;
        }
        let (target, target_len) = socket_address(server, 53);
        let left = ((until - koo_now_nanos()) / 1_000_000).max(1);
        let mut got = 0;
        let mut framed = [0u8; 322];
        framed[..2].copy_from_slice(&(query.len() as u16).to_be_bytes());
        framed[2..2 + query.len()].copy_from_slice(query);
        if connect_within(fd, target.as_ptr(), target_len, left) && send_within(fd, &framed[..2 + query.len()], until) {
            // Two bytes of length, and then so many of the answer.
            let mut have = 0;
            let mut wanted = 2;
            let mut size = [0u8; 2];
            while have < wanted && ready(fd, false, until) {
                let n = if have < 2 {
                    recv(fd, size.as_mut_ptr().add(have), 2 - have, 0)
                } else {
                    recv(fd, answer.as_mut_ptr().add(have - 2), wanted - have, 0)
                };
                if n <= 0 {
                    break;
                }
                have += n as usize;
                if have == 2 {
                    // One that is longer than there is room for is not had at all.
                    if u16::from_be_bytes(size) as usize > answer.len() {
                        break;
                    }
                    wanted = 2 + u16::from_be_bytes(size) as usize;
                }
            }
            if have >= 2 && have == wanted {
                got = have - 2;
            }
        }
        close(fd);
        got
    }
}

/// The first address in a name server's answer.
fn address_in(answer: &[u8], wanted: u8) -> Result<Option<Ip>, ()> {
    first_address(answer, wanted).ok_or(())
}

/// As `address_in`, with nothing for an answer that does not hold what it says it does.
fn first_address(answer: &[u8], wanted: u8) -> Option<Option<Ip>> {
    // A name is labels up to an empty one, or up to a pointer to the rest of it.
    fn after_name(answer: &[u8], mut at: usize) -> Option<usize> {
        loop {
            let length = *answer.get(at)? as usize;
            if length == 0 {
                return Some(at + 1);
            }
            if length >= 192 {
                return Some(at + 2);
            }
            at += 1 + length;
        }
    }
    let questions = u16::from_be_bytes([answer[4], answer[5]]);
    let answers = u16::from_be_bytes([answer[6], answer[7]]);
    let mut at = 12;
    for _ in 0..questions {
        at = after_name(answer, at)? + 4;
    }
    for _ in 0..answers {
        at = after_name(answer, at)?;
        let head = answer.get(at..at + 10)?;
        let kind = u16::from_be_bytes([head[0], head[1]]);
        let length = u16::from_be_bytes([head[8], head[9]]) as usize;
        at += 10;
        // Of the Internet, and of the kind that was asked for.
        let internet = head[2..4] == [0, 1];
        if internet && kind == wanted as u16 && kind == 1 && length == 4 {
            return Some(Some(Ip::V4(answer.get(at..at + 4)?.try_into().ok()?)));
        }
        if internet && kind == wanted as u16 && kind == 28 && length == 16 {
            return Some(Some(Ip::V6(answer.get(at..at + 16)?.try_into().ok()?)));
        }
        at += length;
        // What is said to be there is there.
        if at > answer.len() {
            return None;
        }
    }
    Some(None)
}

/// On macOS the system is asked for the address of a host: what it knows of names is
/// not all in the two files that `resolve` reads (a VPN has its own name servers for its
/// own names, and a machine on the same network is found without any), and its resolver
/// is part of every program there in any case.
#[cfg(target_os = "macos")]
unsafe fn by_the_system(host: &[u8]) -> Option<Ip> {
    unsafe {
        #[repr(C)]
        struct Found {
            flags: i32,
            family: i32,
            kind: i32,
            protocol: i32,
            address_len: u32,
            name: *mut u8,
            address: *const u8,
            next: *mut Found,
        }
        unsafe extern "C" {
            fn getaddrinfo(host: *const u8, service: *const u8, hints: *const Found, found: *mut *mut Found) -> i32;
            fn freeaddrinfo(found: *mut Found);
        }
        let mut name = [0u8; 256];
        if host.len() < name.len() && !host.contains(&0) {
            name[..host.len()].copy_from_slice(host);
            // Either kind of address, of those the machine has a way to reach (0x400).
            let hints = Found {
                flags: 0x400,
                family: 0,
                kind: SOCK_STREAM,
                protocol: 0,
                address_len: 0,
                name: core::ptr::null_mut(),
                address: core::ptr::null(),
                next: core::ptr::null_mut(),
            };
            let mut found: *mut Found = core::ptr::null_mut();
            if getaddrinfo(name.as_ptr(), core::ptr::null(), &hints, &mut found) == 0 && !found.is_null() {
                // An address of the old kind if the host has one, since more of the
                // world reaches those; of the new kind otherwise.
                let (mut old, mut new) = (None, None);
                let mut each = found;
                while !each.is_null() {
                    let at = (*each).address;
                    if (*each).family == AF_INET && (*each).address_len >= 8 && old.is_none() {
                        // The length, the family, the port, and then the four bytes.
                        old = Some(Ip::V4(*(at.add(4) as *const [u8; 4])));
                    }
                    if (*each).family == AF_INET6 && (*each).address_len >= 24 && new.is_none() {
                        // The same, a number of the flow, and then the sixteen.
                        new = Some(Ip::V6(*(at.add(8) as *const [u8; 16])));
                    }
                    each = (*each).next;
                }
                freeaddrinfo(found);
                return old.or(new);
            }
        }
        None
    }
}

#[cfg(not(target_os = "macos"))]
unsafe fn by_the_system(_host: &[u8]) -> Option<Ip> {
    None
}

/// The address of a host: as it is written, from the machine's table of hosts, or from
/// the name servers the machine is set up with, trying the name inside each of the
/// domains it searches, as the system's own resolver does. A host that has an address
/// of the old kind is reached at that one, and one that has only the new kind at that.
unsafe fn resolve(host: &[u8], until: i64, silent: &mut bool) -> Option<Ip> {
    unsafe {
        *silent = false;
        if let Some(address) = literal(host) {
            return Some(address);
        }
        if host.eq_ignore_ascii_case(b"localhost") {
            return Some(Ip::V4([127, 0, 0, 1]));
        }
        if cfg!(target_os = "macos") {
            return by_the_system(host);
        }
        let mut hosts = file(c"/etc/hosts");
        let (mut old, mut new) = (None, None);
        for line in hosts.bytes().split(|byte| *byte == b'\n') {
            let mut parts = words(line);
            let Some(address) = parts.next().and_then(|word| literal(word)) else {
                continue;
            };
            if parts.any(|name| name.eq_ignore_ascii_case(host)) {
                match address {
                    Ip::V4(_) if old.is_none() => old = Some(address),
                    Ip::V6(_) if new.is_none() => new = Some(address),
                    _ => {}
                }
            }
        }
        hosts.release();
        if old.or(new).is_some() {
            return old.or(new);
        }
        let mut settings = file(c"/etc/resolv.conf");
        let mut servers = [Ip::V4([0; 4]); 3];
        let mut server_count = 0;
        let mut dots_wanted = 1;
        let mut search: &[u8] = &[];
        for line in settings.bytes().split(|byte| *byte == b'\n') {
            let mut parts = words(line);
            match parts.next() {
                Some(b"nameserver") => {
                    if let Some(address) = parts.next().and_then(|word| literal(word))
                        && server_count < servers.len()
                    {
                        servers[server_count] = address;
                        server_count += 1;
                    }
                }
                Some(b"search") | Some(b"domain") => search = line,
                Some(b"options") => {
                    for option in parts {
                        if let Some(count) = option.strip_prefix(b"ndots:") {
                            dots_wanted = count.iter().fold(0, |sum, digit| sum * 10 + (*digit as usize & 15));
                        }
                    }
                }
                _ => {}
            }
        }
        if server_count == 0 {
            servers[0] = Ip::V4([127, 0, 0, 1]);
            server_count = 1;
        }
        let dots = host.iter().filter(|byte| **byte == b'.').count();
        // A name with few dots is looked for in the searched domains first.
        let alone_first = dots >= dots_wanted || host.ends_with(b".");
        // The servers are asked together, and the first that answers is believed.
        let late = || until != 0 && koo_now_nanos() >= until;
        // Whether any server said anything of any of the names.
        let heard = core::cell::Cell::new(false);
        let ask = |name: &[u8], kind: u8| {
            if late() {
                return None;
            }
            let said = ask_name_servers(&servers[..server_count], name, kind, until);
            heard.set(heard.get() || said.is_some());
            said.flatten()
        };
        let mut answer = None;
        // An address of the old kind is asked for everywhere first, and one of the new
        // kind only for a host that has none.
        for kind in [1u8, 28] {
            if alone_first {
                answer = ask(host, kind);
            }
            if answer.is_none() && !host.ends_with(b".") {
                for domain in words(search).skip(1) {
                    let mut name = [0u8; 256];
                    if host.len() + 1 + domain.len() > name.len() {
                        continue;
                    }
                    name[..host.len()].copy_from_slice(host);
                    name[host.len()] = b'.';
                    name[host.len() + 1..host.len() + 1 + domain.len()].copy_from_slice(domain);
                    answer = ask(&name[..host.len() + 1 + domain.len()], kind);
                    if answer.is_some() || late() {
                        break;
                    }
                }
            }
            if answer.is_none() && !alone_first {
                answer = ask(host, kind);
            }
            if answer.is_some() || late() {
                break;
            }
        }
        settings.release();
        *silent = answer.is_none() && !heard.get();
        answer
    }
}

/// An address and a port as the system wants them, and how much of it there is.
fn socket_address(address: Ip, port: u16) -> ([u8; 28], u32) {
    let mut out = [0u8; 28];
    let (family, len) = match address {
        Ip::V4(_) => (AF_INET, 16),
        Ip::V6(_) => (AF_INET6, 28),
    };
    if cfg!(target_os = "macos") {
        out[0] = len as u8;
        out[1] = family as u8;
    } else {
        out[..2].copy_from_slice(&(family as u16).to_ne_bytes());
    }
    out[2..4].copy_from_slice(&port.to_be_bytes());
    match address {
        Ip::V4(bytes) => out[4..8].copy_from_slice(&bytes),
        // After the port comes a number of the flow, and then the address.
        Ip::V6(bytes) => out[8..24].copy_from_slice(&bytes),
    }
    (out, len)
}

// ---- talking to the server --------------------------------------------------------------

/// Writes to a connection. A server that has gone must be an error here and not a
/// signal that ends the program: on Linux that is said with each write, and on macOS
/// once, when the socket is made.
pub(crate) unsafe fn send_all(fd: i32, mut data: &[u8]) -> bool {
    let quietly = if cfg!(target_os = "linux") { 0x4000 } else { 0 };
    while !data.is_empty() {
        let n = unsafe { sendto(fd, data.as_ptr(), data.len(), quietly, core::ptr::null(), 0) };
        if n > 0 {
            data = &data[n as usize..];
        } else if unsafe { *errno_location() } != EINTR {
            return false;
        }
    }
    true
}

/// Waits until a socket can be read from, or written to, or until the time `until` on
/// the clock that only goes forward (0 for no limit). A handler of a server is left
/// where it is meanwhile. Returns false if the time ran out.
pub(crate) unsafe fn ready(fd: i32, write: bool, until: i64) -> bool {
    unsafe {
        match fiber::wait(fd, write, until) {
            fiber::Waited::Came => true,
            fiber::Waited::TimeUp => false,
            fiber::Waited::NotHere => {
                let mut fds = PollFd {
                    fd,
                    events: if write { 4 } else { 1 },
                    revents: 0,
                };
                loop {
                    let left = if until == 0 {
                        -1
                    } else {
                        // In whole milliseconds, and no fewer than are left.
                        ((until - koo_now_nanos() + 999_999) / 1_000_000).clamp(0, i32::MAX as i64) as i32
                    };
                    let found = poll(&mut fds, 1, left);
                    if found < 0 && *errno_location() == EINTR {
                        continue;
                    }
                    break found > 0;
                }
            }
        }
    }
}

/// Writes to a connection as `send_all` does, but without keeping the thread while the
/// other end has no room: a handler of a server is left where it is until there is
/// room, and the wait ends at `until` (0 for no limit). Returns whether all went out.
pub(crate) unsafe fn send_within(fd: i32, mut data: &[u8], until: i64) -> bool {
    unsafe {
        const F_GETFL: i32 = 3;
        const F_SETFL: i32 = 4;
        let quietly = if cfg!(target_os = "linux") { 0x4000 } else { 0 };
        let non_blocking = if cfg!(target_os = "macos") { 0x4 } else { 0x800 };
        let full = if cfg!(target_os = "macos") { 35 } else { 11 };
        // The socket itself is told not to wait while this goes out: a word to that
        // effect with each write is not heeded everywhere for a write that is long.
        let flags = fcntl(fd, F_GETFL);
        fcntl(fd, F_SETFL, flags | non_blocking);
        let mut whole = true;
        while !data.is_empty() {
            let n = sendto(fd, data.as_ptr(), data.len(), quietly, core::ptr::null(), 0);
            if n > 0 {
                data = &data[n as usize..];
                continue;
            }
            let error = *errno_location();
            if error == EINTR {
                continue;
            }
            // The time is for the whole of it, however much has gone out meanwhile.
            if error != full || (until != 0 && koo_now_nanos() >= until) || !ready(fd, true, until) {
                whole = false;
                break;
            }
        }
        fcntl(fd, F_SETFL, flags);
        whole
    }
}

/// Writes to a connection, through its encryption if it has one.
unsafe fn send(conn: &Conn, data: &[u8]) -> bool {
    unsafe {
        if conn.tls.is_null() {
            send_within(conn.fd, data, conn.until)
        } else {
            tls::write_all(conn.tls, data)
        }
    }
}

/// Ends a connection: its encryption, with a word to the server if `polite`, and its
/// socket. A process that was forked is not polite about the connections of its
/// parent, which are still the parent's.
unsafe fn hang_up(conn: &mut Conn, polite: bool) {
    unsafe {
        // A connection that is let go of in the middle of a statement, because whoever
        // sent it will not wait for the answer: the server is told to stop the
        // statement, which it would otherwise go on with to its end, for nobody.
        if conn.asked && conn.known && conn.fd >= 0 {
            stop_statement(conn);
        }
        conn.asked = false;
        if !conn.tls.is_null() {
            tls::close(conn.tls, polite);
            conn.tls = core::ptr::null_mut();
        }
        if conn.fd >= 0 {
            close(conn.fd);
            conn.fd = -1;
        }
    }
}

// ---- telling the server to stop a statement -----------------------------------------------
//
// PostgreSQL stops a statement when it is told to on another connection: one that says
// only that, with the number and the key the server gave the first connection, and is
// then closed. Making that connection may wait, and where a statement is given up there
// must be no waiting (a handler of a server whose client has left is let wait for
// nothing), so a thread of its own does it: it sleeps until it is given a statement to
// stop, and nothing waits for it.

struct Stop {
    address: [u8; 28],
    address_len: u32,
    backend: [u8; 8],
}

static STOPPING: thread::Lock = thread::Lock::new();
static mut STOPS: [Stop; 64] = [const {
    Stop {
        address: [0; 28],
        address_len: 0,
        backend: [0; 8],
    }
}; 64];
static mut STOP_COUNT: usize = 0;
/// The pipe by which the thread is woken, and the process whose thread it is.
static mut STOP_PIPE: [i32; 2] = [-1; 2];
static STOPPER: AtomicI32 = AtomicI32::new(0);

/// Hands the statement that is with the server on a connection to the thread that
/// tells servers to stop. If there is no thread to be had, or it has sixty-four
/// statements to stop already, the statement runs to its end, as it did before.
#[allow(static_mut_refs)]
unsafe fn stop_statement(conn: &Conn) {
    unsafe {
        let mut stop = Stop {
            address: [0; 28],
            address_len: 28,
            backend: conn.backend,
        };
        if getpeername(conn.fd, stop.address.as_mut_ptr(), &mut stop.address_len) != 0 || stop.address_len > 28 {
            return;
        }
        STOPPING.lock();
        let pid = getpid();
        if STOPPER.load(Acquire) != pid {
            // The first in this process: a worker of a server has none of the threads
            // of the process that started it, nor what that one had still to do.
            let mut ends = [-1i32; 2];
            if pipe(ends.as_mut_ptr()) != 0 {
                STOPPING.unlock();
                return;
            }
            STOP_PIPE = ends;
            STOP_COUNT = 0;
            if !thread::start(stopper) {
                close(ends[0]);
                close(ends[1]);
                STOP_PIPE = [-1; 2];
                STOPPING.unlock();
                return;
            }
            STOPPER.store(pid, Release);
        }
        if STOP_COUNT < STOPS.len() {
            STOPS[STOP_COUNT] = stop;
            STOP_COUNT += 1;
            let _ = write(STOP_PIPE[1], [1u8].as_ptr(), 1);
        }
        STOPPING.unlock();
    }
}

/// The thread: for each statement it is given, a connection to the server that says
/// to stop it and nothing else. Two seconds for each, and no word of how it went: the
/// server answers nothing, and a statement that has ended meanwhile is none the worse.
#[allow(static_mut_refs)]
extern "C" fn stopper(_: *mut u8) -> *mut u8 {
    unsafe {
        loop {
            let mut woken = [0u8; 16];
            if read(STOP_PIPE[0], woken.as_mut_ptr(), woken.len()) <= 0 && *errno_location() != EINTR {
                return core::ptr::null_mut();
            }
            loop {
                STOPPING.lock();
                if STOP_COUNT == 0 {
                    STOPPING.unlock();
                    break;
                }
                STOP_COUNT -= 1;
                let stop = core::ptr::read(&STOPS[STOP_COUNT]);
                STOPPING.unlock();
                // The kind of address is its first two bytes, or on macOS its second.
                let family = if cfg!(target_os = "macos") {
                    stop.address[1] as i32
                } else {
                    u16::from_ne_bytes([stop.address[0], stop.address[1]]) as i32
                };
                let fd = socket(family, SOCK_STREAM, 0);
                if fd < 0 {
                    continue;
                }
                // Sixteen bytes: their number, the word for this, and the two numbers.
                let mut said = [0u8; 16];
                said[..8].copy_from_slice(&[0, 0, 0, 16, 0x04, 0xd2, 0x16, 0x2e]);
                said[8..].copy_from_slice(&stop.backend);
                let until = koo_now_nanos() + 2_000_000_000;
                if connect_within(fd, stop.address.as_ptr(), stop.address_len, 2000) {
                    send_within(fd, &said, until);
                }
                close(fd);
            }
        }
    }
}

/// Reads until `conn.input` holds `need` bytes past `conn.at`.
unsafe fn fill(conn: &mut Conn, need: usize) -> bool {
    unsafe {
        if conn.at > 0 && conn.at == conn.input.len {
            conn.at = 0;
            conn.input.len = 0;
        }
        while conn.input.len - conn.at < need {
            // A server that goes on talking while a connection is set up, without
            // coming to the point, has no more time than one that says nothing.
            if conn.until != 0 && koo_now_nanos() >= conn.until {
                return false;
            }
            conn.input.room(need.max(8192));
            let room =
                core::slice::from_raw_parts_mut(conn.input.ptr.add(conn.input.len), conn.input.cap - conn.input.len);
            // The answer of a database is what a handler of a server waits for most:
            // the server does other work until there is something of it to read.
            if (conn.tls.is_null() || tls::pending(conn.tls) == 0) && !ready(conn.fd, false, conn.until) {
                return false;
            }
            let n = if conn.tls.is_null() {
                read(conn.fd, room.as_mut_ptr(), room.len())
            } else {
                match tls::read(conn.tls, room) {
                    // An error of the encryption is not one to try again after.
                    n if n < 0 => return false,
                    n => n,
                }
            };
            if n > 0 {
                conn.input.len += n as usize;
            } else if n == 0 || *errno_location() != EINTR {
                return false;
            }
        }
        true
    }
}

/// The next message from the server: its kind, and what it says, which stays where it
/// is until the next one is read.
unsafe fn message<'a>(conn: &mut Conn) -> Option<(u8, &'a [u8])> {
    unsafe {
        if !fill(conn, 5) {
            return None;
        }
        let head = core::slice::from_raw_parts(conn.input.ptr.add(conn.at), 5);
        let kind = head[0];
        let length = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        // The server sends nothing longer than a gigabyte.
        if !(4..=0x4000_0000).contains(&length) || !fill(conn, 1 + length) {
            return None;
        }
        let body = core::slice::from_raw_parts(conn.input.ptr.add(conn.at + 5), length - 4);
        conn.at += 1 + length;
        Some((kind, body))
    }
}

/// What an error from the server says: its code, and its message.
fn error_fields(body: &[u8]) -> (&[u8], &[u8]) {
    let (mut code, mut text): (&[u8], &[u8]) = (b"", b"the server reported an error");
    for field in body.split(|byte| *byte == 0) {
        match field.split_first() {
            Some((b'C', value)) => code = value,
            Some((b'M', value)) => text = value,
            _ => {}
        }
    }
    (code, text)
}

unsafe fn server_error(body: &[u8]) {
    unsafe {
        let (code, text) = error_fields(body);
        let kind = match code.get(..2) {
            _ if gone(code) => CONNECTION,
            // A statement that ran out of the time it was given, or waited too long
            // for a lock.
            _ if code == b"57014" || code == b"55P03" => TIMEOUT,
            Some(b"23") => CONSTRAINT,
            Some(b"42") => SYNTAX,
            Some(b"40") => SERIALIZATION,
            _ => OTHER,
        };
        failed(kind, &[text]);
        let out = outcome();
        if code.len() == 5 {
            out.code.copy_from_slice(code);
        }
    }
}

/// Proves to the server who is connecting, by whichever of the three ways it asks for.
/// Returns once the server says it is satisfied.
unsafe fn authenticate(conn: &mut Conn, db: &Db) -> bool {
    unsafe {
        let (user, password) = (db.user.bytes(), db.password.bytes());
        let mut nonce = [0u8; 24];
        let mut first_bare = Buf::new();
        let mut server_proof = [0u8; 32];
        let mut out = Buf::new();
        let done = loop {
            let Some((kind, body)) = message(conn) else {
                failed(CONNECTION, &[unready(conn)]);
                break false;
            };
            if kind == b'E' {
                server_error(body);
                outcome().kind = CONNECTION;
                break false;
            }
            if kind != b'R' || body.len() < 4 {
                continue;
            }
            out.len = 0;
            match u32::from_be_bytes([body[0], body[1], body[2], body[3]]) {
                0 => break true,
                3 => {
                    let mut reply = Message::begin(&mut out, b'p');
                    reply.text(password);
                    reply.end();
                }
                5 if body.len() >= 8 => {
                    // "md5", then the digest of the salt after the digest of the
                    // password and the user's name, each digest in hexadecimal.
                    let mut inner = [0u8; 32];
                    hex(&md5(&[password, user]), &mut inner);
                    let mut outer = [0u8; 35];
                    outer[..3].copy_from_slice(b"md5");
                    hex(&md5(&[&inner, &body[4..8]]), &mut outer[3..]);
                    let mut reply = Message::begin(&mut out, b'p');
                    reply.text(&outer);
                    reply.end();
                }
                10 => {
                    if !body[4..].split(|byte| *byte == 0).any(|way| way == b"SCRAM-SHA-256") {
                        failed(
                            CONNECTION,
                            &[b"the server asks for a proof of identity that Koo does not know"],
                        );
                        break false;
                    }
                    let mut random = [0u8; 18];
                    getentropy(random.as_mut_ptr(), random.len());
                    base64_encode(&random, &mut nonce);
                    first_bare.len = 0;
                    first_bare.push(b"n=,r=");
                    first_bare.push(&nonce);
                    let mut reply = Message::begin(&mut out, b'p');
                    reply.text(b"SCRAM-SHA-256");
                    reply.int32(3 + first_bare.len as i32);
                    reply.bytes(b"n,,");
                    reply.bytes(first_bare.bytes());
                    reply.end();
                }
                11 => {
                    // The server's nonce, which begins with ours, a salt, and a count
                    // of rounds: `r=...,s=...,i=...`.
                    let server_first = &body[4..];
                    let field = |name: u8| {
                        server_first
                            .split(|byte| *byte == b',')
                            .find_map(|field| field.strip_prefix(&[name, b'='][..]))
                    };
                    let (Some(joined), Some(salt), Some(rounds)) = (field(b'r'), field(b's'), field(b'i')) else {
                        failed(CONNECTION, &[b"the server's answer to the password was not understood"]);
                        break false;
                    };
                    let mut salt_bytes = [0u8; 64];
                    let salt_len = base64_decode(salt, &mut salt_bytes);
                    let rounds = rounds.iter().fold(0u32, |sum, digit| {
                        sum.saturating_mul(10).saturating_add((*digit as u32) & 15)
                    });
                    let Some(salt_len) = salt_len.filter(|_| joined.starts_with(&nonce) && rounds > 0) else {
                        failed(CONNECTION, &[b"the server's answer to the password was not understood"]);
                        break false;
                    };
                    // A server names the work to be done on the password, and may name
                    // more than anyone would wait for: PostgreSQL asks for 4,096 rounds.
                    if rounds > 1_000_000 {
                        failed(
                            CONNECTION,
                            &[b"the server asks for more than a million rounds of work on the password, which is refused"],
                        );
                        break false;
                    }
                    let salted = pbkdf2_sha256(password, &salt_bytes[..salt_len], rounds);
                    let client_key = hmac_sha256(&salted, &[b"Client Key"]);
                    let stored_key = sha256(&client_key);
                    let message_parts: [&[u8]; 6] =
                        [first_bare.bytes(), b",", server_first, b",", b"c=biws,r=", joined];
                    let signature = hmac_sha256(&stored_key, &message_parts);
                    let mut proof = [0u8; 32];
                    for (byte, (key, sign)) in proof.iter_mut().zip(client_key.iter().zip(signature)) {
                        *byte = key ^ sign;
                    }
                    let server_key = hmac_sha256(&salted, &[b"Server Key"]);
                    server_proof = hmac_sha256(&server_key, &message_parts);
                    let mut proof_text = [0u8; 44];
                    base64_encode(&proof, &mut proof_text);
                    let mut reply = Message::begin(&mut out, b'p');
                    reply.bytes(b"c=biws,r=");
                    reply.bytes(joined);
                    reply.bytes(b",p=");
                    reply.bytes(&proof_text);
                    reply.end();
                }
                12 => {
                    // The server proves in turn that it knows the password.
                    let mut expected = [0u8; 46];
                    expected[..2].copy_from_slice(b"v=");
                    base64_encode(&server_proof, &mut expected[2..]);
                    if body[4..] != expected {
                        failed(CONNECTION, &[b"the server did not prove that it knows the password"]);
                        break false;
                    }
                }
                _ => {
                    failed(
                        CONNECTION,
                        &[b"the server asks for a proof of identity that Koo does not know"],
                    );
                    break false;
                }
            }
            if out.len > 0 && !send(conn, out.bytes()) {
                failed(CONNECTION, &[b"the connection was lost while it was being set up"]);
                break false;
            }
        };
        out.release();
        first_bare.release();
        done
    }
}

/// Connects a socket, giving the other side so many milliseconds to answer and not
/// the minutes the system would wait. The connection is begun without waiting; then a
/// handler of a server lets the server do other work until it is made, and anything
/// else waits for it here. Returns whether it was made; if not, the error is where the
/// system leaves errors, and is `SLOW` if the time ran out.
pub(crate) unsafe fn connect_within(fd: i32, target: *const u8, target_len: u32, milliseconds: i64) -> bool {
    unsafe {
        const F_GETFL: i32 = 3;
        const F_SETFL: i32 = 4;
        let non_blocking = if cfg!(target_os = "macos") { 0x4 } else { 0x800 };
        let in_progress = if cfg!(target_os = "macos") { 36 } else { 115 };
        let (level, error_of) = if cfg!(target_os = "macos") {
            (0xffff, 0x1007)
        } else {
            (1, 4)
        };
        let flags = fcntl(fd, F_GETFL);
        fcntl(fd, F_SETFL, flags | non_blocking);
        let mut made = connect(fd, target, target_len) == 0;
        let mut failed_with = *errno_location();
        if !made && failed_with == in_progress {
            let until = koo_now_nanos() + milliseconds * 1_000_000;
            let came = match fiber::wait(fd, true, until) {
                fiber::Waited::Came => true,
                fiber::Waited::TimeUp => false,
                fiber::Waited::NotHere => {
                    // 4 asks whether the socket can be written to, which is when the
                    // connection is made or refused.
                    let mut fds = PollFd {
                        fd,
                        events: 4,
                        revents: 0,
                    };
                    let mut left = milliseconds;
                    loop {
                        let found = poll(&mut fds, 1, left.clamp(0, i32::MAX as i64) as i32);
                        if found < 0 && *errno_location() == EINTR {
                            left = (until - koo_now_nanos()) / 1_000_000;
                            if left > 0 {
                                continue;
                            }
                        }
                        break found > 0;
                    }
                }
            };
            if came {
                let (mut error, mut len) = (0i32, 4u32);
                getsockopt(fd, level, error_of, &mut error, &mut len);
                made = error == 0;
                failed_with = error;
            } else {
                failed_with = SLOW;
            }
        }
        fcntl(fd, F_SETFL, flags);
        *errno_location() = failed_with;
        made
    }
}

/// What `connect_within` leaves in place of an error when the time ran out.
const SLOW: i32 = -2;

/// What `dial` returns for a server that did not answer in its time.
pub(crate) const UNANSWERED: i32 = -2;

/// Connects to a host, giving it so many milliseconds to answer. Returns the socket; or
/// -1, or `UNANSWERED` if the time ran out, with the reason written into `why`.
pub(crate) unsafe fn dial(host: &[u8], port: u16, milliseconds: i64, why: &mut Buffer) -> i32 {
    unsafe {
        let milliseconds = milliseconds.max(1);
        let given = milliseconds;
        let until = koo_now_nanos() + milliseconds * 1_000_000;
        let mut silent = false;
        let Some(address) = resolve(host, until, &mut silent) else {
            // The time may be what ran out, with the name servers silent.
            if koo_now_nanos() >= until {
                why.push(b"the address of the host `");
                why.push(host);
                let _ = write!(why, "` was not found in {given} ms");
                return UNANSWERED;
            }
            // Or they were silent, or of no help, for as long as they are given.
            if silent {
                why.push(b"the name servers did not say what the address of the host `");
                why.push(host);
                why.push(b"` is");
                return -1;
            }
            why.push(b"there is no address for the host `");
            why.push(host);
            why.push(b"`");
            return -1;
        };
        // What is left of the time is for the connection itself.
        let milliseconds = (until - koo_now_nanos()) / 1_000_000;
        let family = if matches!(address, Ip::V6(_)) {
            AF_INET6
        } else {
            AF_INET
        };
        let fd = socket(family, SOCK_STREAM, 0);
        let (target, target_len) = socket_address(address, port);
        let milliseconds = milliseconds.max(1);
        let connected = fd >= 0 && connect_within(fd, target.as_ptr(), target_len, milliseconds);
        if !connected {
            let slow = fd >= 0 && *errno_location() == SLOW;
            why.push(b"cannot connect to ");
            why.push(host);
            let _ = write!(why, ":{port}: ");
            if slow && given % 1000 == 0 {
                let _ = write!(why, "the server did not answer in {} seconds", given / 1000);
            } else if slow {
                let _ = write!(why, "the server did not answer in {given} ms");
            } else {
                why.push(system_error());
            }
            if fd >= 0 {
                close(fd);
            }
            return if slow { UNANSWERED } else { -1 };
        }
        let on = 1i32;
        setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &on, 4);
        // A server that has vanished without a word, as when a machine loses power or
        // a firewall forgets a connection, is found out: the system asks after it when
        // nothing has passed for thirty seconds, every ten seconds, and gives the
        // connection up after three askings unanswered; and what was sent and is not
        // acknowledged in thirty seconds ends it too.
        let (idle, between, askings, unanswered) = (30i32, 10i32, 3i32, 30i32);
        if cfg!(target_os = "macos") {
            // No signal for a write to a server that has gone.
            setsockopt(fd, 0xffff, 0x1022, &on, 4);
            setsockopt(fd, 0xffff, 0x0008, &on, 4);
            setsockopt(fd, IPPROTO_TCP, 0x10, &idle, 4);
            setsockopt(fd, IPPROTO_TCP, 0x101, &between, 4);
            setsockopt(fd, IPPROTO_TCP, 0x102, &askings, 4);
            setsockopt(fd, IPPROTO_TCP, 0x80, &unanswered, 4);
        } else {
            let milliseconds = unanswered * 1000;
            setsockopt(fd, 1, 9, &on, 4);
            setsockopt(fd, IPPROTO_TCP, 4, &idle, 4);
            setsockopt(fd, IPPROTO_TCP, 5, &between, 4);
            setsockopt(fd, IPPROTO_TCP, 6, &askings, 4);
            setsockopt(fd, IPPROTO_TCP, 18, &milliseconds, 4);
        }
        fd
    }
}

/// Why a connection that was being set up is not ready: the server went, or the time
/// ran out with the server there and silent.
unsafe fn unready(conn: &Conn) -> &'static [u8] {
    if conn.until != 0 && unsafe { koo_now_nanos() } >= conn.until {
        b"the server took the connection and then did not answer in ten seconds"
    } else {
        b"the server closed the connection while it was being set up"
    }
}

/// Connects to a database and waits until it is ready for a query.
unsafe fn establish(conn: &mut Conn, db: &Db) -> bool {
    unsafe {
        let host = db.host.bytes();
        let mut why = Buffer::new();
        let fd = dial(host, db.port, 10_000, &mut why);
        if fd < 0 {
            failed(CONNECTION, &[&why.data[..why.len]]);
            return false;
        }
        conn.fd = fd;
        conn.at = 0;
        conn.input.len = 0;
        // A server that takes the connection and then says nothing is given as long to
        // answer as one is given to take it, and no longer.
        conn.until = koo_now_nanos() + 10_000_000_000;
        // Encryption is asked for before anything else is said: the server answers
        // with one letter, `S` if it will, and then TLS begins on the same socket.
        let wanted = match db.secure {
            0 => false,
            1 => tls::available(),
            _ => true,
        };
        if wanted {
            if !tls::available() {
                failed(CONNECTION, &[tls::missing()]);
                hang_up(conn, false);
                return false;
            }
            let mut answer = [0u8; 1];
            let asked = send_all(fd, &[0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f]);
            if !asked || !ready(fd, false, conn.until) || read(fd, answer.as_mut_ptr(), 1) != 1 {
                failed(CONNECTION, &[unready(conn)]);
                hang_up(conn, false);
                return false;
            }
            if answer[0] == b'S' {
                let trust = match db.secure {
                    3 => tls::Trust::Authority,
                    4 => tls::Trust::Host,
                    _ => tls::Trust::Any,
                };
                let mut why = [0u8; 300];
                match tls::connect(fd, host, trust, db.authorities.bytes(), &mut why) {
                    Ok(made) => conn.tls = made,
                    Err(len) => {
                        failed(CONNECTION, &[&why[..len]]);
                        hang_up(conn, false);
                        return false;
                    }
                }
            } else if db.secure >= 2 {
                failed(
                    CONNECTION,
                    &[b"the server does not encrypt its connections, and the address asks for it with `sslmode`"],
                );
                hang_up(conn, false);
                return false;
            }
        }
        let mut out = Buf::new();
        let mut start = Message::begin(&mut out, 0);
        start.int32(196608); // version 3.0
        start.text(b"user");
        start.text(db.user.bytes());
        start.text(b"database");
        start.text(db.database.bytes());
        start.text(b"client_encoding");
        start.text(b"UTF8");
        start.text(b"application_name");
        start.text(b"koo");
        // Dates and times are read in the one form that `sql.Row` understands.
        start.text(b"DateStyle");
        start.text(b"ISO");
        if db.options.len > 0 {
            start.text(b"options");
            start.text(db.options.bytes());
        }
        start.bytes(&[0]);
        start.end();
        let sent = send(conn, out.bytes());
        out.release();
        if !sent {
            failed(CONNECTION, &[b"the connection was lost while it was being set up"]);
        }
        let mut ready = sent && authenticate(conn, db);
        // What the server says of itself, up to the word that it is ready.
        conn.known = false;
        conn.asked = false;
        while ready {
            match message(conn) {
                Some((b'Z', _)) => break,
                Some((b'K', body)) if body.len() == 8 => {
                    let mut backend = [0u8; 8];
                    backend.copy_from_slice(body);
                    conn.backend = backend;
                    conn.known = true;
                }
                Some((b'E', body)) => {
                    server_error(body);
                    outcome().kind = CONNECTION;
                    ready = false;
                }
                Some(_) => {}
                None => {
                    failed(CONNECTION, &[unready(conn)]);
                    ready = false;
                }
            }
        }
        if !ready {
            hang_up(conn, false);
        }
        conn.until = 0;
        ready
    }
}

// ---- the pool ---------------------------------------------------------------------------

/// Makes the pool usable, and, in a process that was forked, forgets the connections of
/// its parent: they are the parent's to use and to close.
#[allow(static_mut_refs)]
unsafe fn adopt() {
    unsafe {
        let pid = getpid();
        if OPENED.load(Acquire) == 0 || OWNER.load(Acquire) != pid {
            POOL.open();
            for db in DBS.iter_mut() {
                for conn in db.conns.iter_mut() {
                    // (A statement that was with the server when this process was
                    // forked is the other process's, and is not this one's to stop.)
                    conn.asked = false;
                    hang_up(conn, false);
                    conn.busy = false;
                    conn.transaction = 0;
                }
            }
            OWNER.store(pid, Release);
            OPENED.store(1, Release);
        }
    }
}

/// In a process just forked: the connections to databases are the other process's, and
/// are closed here at once and not when this process first sends a statement, which it
/// may never do.
pub unsafe fn after_fork() {
    unsafe {
        if OPENED.load(Acquire) != 0 {
            adopt();
        }
    }
}

/// A connection for the calling thread: one that is idle, or a new one if there may be
/// more, or the first to be given back. Nothing if one could not be made.
#[allow(static_mut_refs)]
unsafe fn check_out(db: &mut Db) -> Option<&mut Conn> {
    unsafe {
        adopt();
        POOL.lock();
        let until = koo_now_nanos().saturating_add(db.patience.saturating_mul(1_000_000));
        // The number of this one among those that wait, once it has had to.
        let mut number = 0u64;
        let slot = loop {
            // A connection that is free is for whoever has waited longest for one.
            let turn = db.turns.mine(number, koo_now_nanos());
            if turn && let Some(index) = db.conns.iter().position(|conn| conn.fd >= 0 && !conn.busy) {
                db.conns[index].busy = true;
                db.turns.done(number);
                POOL.unlock();
                return Some(&mut db.conns[index]);
            }
            let open = db.conns.iter().filter(|conn| conn.fd >= 0 || conn.busy).count();
            if turn
                && open < db.limit
                && let Some(index) = db.conns.iter().position(|conn| conn.fd < 0 && !conn.busy)
            {
                db.conns[index].busy = true;
                db.turns.done(number);
                break index;
            }
            // Every connection is in use. If each is in a transaction that this very
            // thread began, none will ever come back: the thread waits for itself.
            let mine = me() as *mut thread::Thread as usize;
            let all_mine = db.conns.iter().any(|conn| conn.busy)
                && db
                    .conns
                    .iter()
                    .filter(|conn| conn.busy)
                    .all(|conn| conn.transaction != 0 && conn.began == mine);
            if all_mine {
                db.turns.done(number);
                POOL.unlock();
                failed(
                    OTHER,
                    &[b"every connection of the pool is in a transaction that this statement is inside of, so it would wait for ever: inside `db.transaction(tx -> ...)` a statement is sent with `tx`, not with `db`"],
                );
                return None;
            }
            let left = (until - koo_now_nanos()) / 1_000_000;
            if left <= 0 {
                db.turns.done(number);
                POOL.unlock();
                let mut text = Buffer::new();
                let _ = write!(
                    text,
                    "every connection to the database was in use for {} ms, and the pool has {}",
                    db.patience, db.limit
                );
                failed(TIMEOUT, &[&text.data[..text.len]]);
                return None;
            }
            // (One whose request was given up does not wait at all, and must not be
            // let ask again and again without waiting.)
            if crate::http::given_up() {
                db.turns.done(number);
                POOL.unlock();
                failed(TIMEOUT, &[GIVEN_UP]);
                return None;
            }
            db.turns.wait(&mut number, until);
            // A handler of a server that waits for a connection lets the others go on,
            // one of which has it, and is woken when one is given back (and looks
            // again before long whether or not, since a connection is also freed by
            // being closed); anything else sleeps until one is given back.
            POOL.unlock();
            let paused = crate::http::await_turn(2_000_000) || fiber::pause(1_000_000);
            POOL.lock();
            if !paused {
                POOL.wait_for(left);
            }
        };
        POOL.unlock();
        let ready = {
            let (conns, rest): (*mut Conn, *const Db) = (db.conns.as_mut_ptr(), db);
            establish(&mut *conns.add(slot), &*rest)
        };
        if !ready {
            give_back(&mut db.conns[slot], true);
            return None;
        }
        Some(&mut db.conns[slot])
    }
}

/// Gives a connection back to the pool, or closes it if it is no longer good.
unsafe fn give_back(conn: &mut Conn, broken: bool) {
    unsafe {
        POOL.lock();
        if broken {
            hang_up(conn, false);
        }
        conn.busy = false;
        // Every pool's waiters sleep at the one gate: all are woken, and those whose
        // pool this is not sleep again.
        POOL.wake_all();
        POOL.unlock();
        crate::http::give_turns();
    }
}

// ---- what the program asks for ----------------------------------------------------------

/// `%41` as `A`, in a part of an address.
unsafe fn unescaped(part: &[u8], out: &mut Buf) {
    unsafe {
        out.len = 0;
        let mut at = 0;
        while at < part.len() {
            let digit = |byte: u8| (byte as char).to_digit(16);
            if part[at] == b'%'
                && at + 2 < part.len()
                && let (Some(high), Some(low)) = (digit(part[at + 1]), digit(part[at + 2]))
            {
                out.push(&[(high * 16 + low) as u8]);
                at += 3;
            } else {
                out.push(&[part[at]]);
                at += 1;
            }
        }
    }
}

/// The parameters of an address that PostgreSQL's own client knows (its documentation,
/// "Parameter Key Words"). Of these the client here acts on `sslmode`, `sslrootcert`
/// and `options`; the others are let pass, as addresses handed out by those who host
/// databases carry them, and any other name is refused.
const PARAMETERS: [&[u8]; 52] = [
    b"application_name",
    b"channel_binding",
    b"client_encoding",
    b"connect_timeout",
    b"dbname",
    b"fallback_application_name",
    b"gssdelegation",
    b"gssencmode",
    b"gsslib",
    b"host",
    b"hostaddr",
    b"keepalives",
    b"keepalives_count",
    b"keepalives_idle",
    b"keepalives_interval",
    b"krbsrvname",
    b"load_balance_hosts",
    b"max_protocol_version",
    b"min_protocol_version",
    b"oauth_ca_file",
    b"oauth_client_id",
    b"oauth_client_secret",
    b"oauth_issuer",
    b"oauth_scope",
    b"options",
    b"passfile",
    b"password",
    b"port",
    b"replication",
    b"require_auth",
    b"requirepeer",
    b"requiressl",
    b"scram_client_key",
    b"scram_server_key",
    b"service",
    b"ssl_max_protocol_version",
    b"ssl_min_protocol_version",
    b"sslcert",
    b"sslcertmode",
    b"sslcompression",
    b"sslcrl",
    b"sslcrldir",
    b"sslkey",
    b"sslkeylogfile",
    b"sslmode",
    b"sslnegotiation",
    b"sslpassword",
    b"sslrootcert",
    b"sslsni",
    b"target_session_attrs",
    b"tcp_user_timeout",
    b"user",
];

/// How many letters must be changed, put in or taken out to make one name of another:
/// enough to tell a misspelling from a name that is nothing like any other.
fn distance(a: &[u8], b: &[u8]) -> usize {
    const MOST: usize = 40;
    if a.len() > MOST || b.len() > MOST {
        return MOST;
    }
    let mut row = [0usize; MOST + 1];
    for (at, cell) in row.iter_mut().enumerate().take(b.len() + 1) {
        *cell = at;
    }
    for (i, x) in a.iter().enumerate() {
        let mut before = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let kept = row[j + 1];
            row[j + 1] = (before + usize::from(x != y)).min(row[j] + 1).min(kept + 1);
            before = kept;
        }
    }
    row[b.len()]
}

/// Opens a database: `postgres://user:password@host:port/database`. Returns its number,
/// or -1 if the address is not understood or the database cannot be reached; a thread
/// then finds why in `koo_sql_error`.
#[unsafe(no_mangle)]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn koo_sql_open(url: *const Header, limit: i64, patience: i64) -> i64 {
    unsafe {
        let url = bytes(url);
        let Some(rest) = url
            .strip_prefix(b"postgres://")
            .or_else(|| url.strip_prefix(b"postgresql://"))
        else {
            failed(
                CONNECTION,
                &[b"the address of a database begins with `postgres://`: `postgres://user:password@host:5432/name`"],
            );
            return -1;
        };
        let (rest, options) = match rest.iter().position(|byte| *byte == b'?') {
            Some(at) => (&rest[..at], &rest[at + 1..]),
            None => (rest, &b""[..]),
        };
        // A parameter that PostgreSQL's own client does not know is a mistake in the
        // address, and most often a misspelling: `sslmod=verify-full` would otherwise
        // ask for nothing, and the connection be made as if it were not there.
        for option in options.split(|byte| *byte == b'&').filter(|option| !option.is_empty()) {
            let name = option.split(|byte| *byte == b'=').next().unwrap_or(option);
            if !PARAMETERS.contains(&name) {
                let nearest = PARAMETERS
                    .iter()
                    .map(|known| (distance(name, known), *known))
                    .min_by_key(|(far, _)| *far)
                    .filter(|(far, _)| *far <= 3);
                match nearest {
                    Some((_, known)) => failed(
                        CONNECTION,
                        &[
                            b"`",
                            name,
                            b"` is not a parameter of a database's address; did you mean `",
                            known,
                            b"`?",
                        ],
                    ),
                    None => failed(
                        CONNECTION,
                        &[b"`", name, b"` is not a parameter of a database's address"],
                    ),
                };
                return -1;
            }
        }
        // Whether connections are encrypted, by the names PostgreSQL gives the ways.
        let setting = |name: &[u8]| {
            options
                .split(|byte| *byte == b'&')
                .find_map(|option| option.strip_prefix(name)?.strip_prefix(b"="))
        };
        // What the address leaves out is what the environment says of it, by the names
        // PostgreSQL's own client knows: `PGHOST`, `PGPORT`, `PGUSER`, `PGPASSWORD`,
        // `PGDATABASE` and `PGSSLMODE`. A place that hands a program its database in
        // pieces, and not as one address, is so heard; and a password that comes so is
        // taken as it is, with nothing in it that an address would have to write
        // another way.
        let said = |name: &core::ffi::CStr| -> &'static [u8] {
            let value = crate::getenv(name.as_ptr() as *const u8);
            if value.is_null() { b"" } else { c_text(value) }
        };
        let authorities = setting(b"sslrootcert").unwrap_or(b"");
        let mode = match setting(b"sslmode") {
            None => Some(said(c"PGSSLMODE")).filter(|mode| !mode.is_empty()),
            mode => mode,
        };
        let secure = match mode {
            None | Some(b"prefer") => 1,
            Some(b"disable") => 0,
            // With authorities given, to encrypt is also to ask who is there.
            Some(b"require") if authorities.is_empty() => 2,
            Some(b"require") | Some(b"verify-ca") => 3,
            Some(b"verify-full") => 4,
            Some(other) => {
                failed(
                    CONNECTION,
                    &[
                        b"`",
                        other,
                        b"` is not an `sslmode`: it is `disable`, `prefer`, `require`, `verify-ca` or `verify-full`",
                    ],
                );
                return -1;
            }
        };
        let (who, place) = match rest.iter().rposition(|byte| *byte == b'@') {
            Some(at) => (&rest[..at], &rest[at + 1..]),
            None => (&b""[..], rest),
        };
        let (user, password) = match who.iter().position(|byte| *byte == b':') {
            Some(at) => (&who[..at], &who[at + 1..]),
            None => (who, &b""[..]),
        };
        let (place, database) = match place.iter().position(|byte| *byte == b'/') {
            Some(at) => (&place[..at], &place[at + 1..]),
            None => (place, &b""[..]),
        };
        // An address of the new kind is in brackets, and has colons of its own.
        let bracket = place.iter().rposition(|byte| *byte == b']').unwrap_or(0);
        let (host, port) = match place[bracket..].iter().rposition(|byte| *byte == b':') {
            Some(at) => (&place[..bracket + at], &place[bracket + at + 1..]),
            None if !said(c"PGPORT").is_empty() => (place, said(c"PGPORT")),
            None => (place, &b"5432"[..]),
        };
        let port_number = port
            .iter()
            .try_fold(0u32, |sum, digit| {
                digit.is_ascii_digit().then(|| sum * 10 + (*digit - b'0') as u32)
            })
            .filter(|number| !port.is_empty() && port.len() <= 5 && *number > 0 && *number < 65536);
        let Some(port_number) = port_number else {
            failed(CONNECTION, &[b"`", port, b"` is not a port"]);
            return -1;
        };
        adopt();
        POOL.lock();
        let free = DBS.iter().position(|db| !db.used);
        if let Some(index) = free {
            DBS[index].used = true;
        }
        POOL.unlock();
        let Some(index) = free else {
            failed(OTHER, &[b"a program has at most sixteen databases open"]);
            return -1;
        };
        let db = &mut DBS[index];
        // Each part from the address, where `%` and two digits stand for a byte; or
        // from the environment, as it is; or what it is when nobody says.
        let part = |written: &[u8], named: &core::ffi::CStr, otherwise: &[u8], out: &mut Buf| {
            if !written.is_empty() {
                unescaped(written, out);
            } else {
                out.len = 0;
                out.push(if said(named).is_empty() { otherwise } else { said(named) });
            }
        };
        part(user, c"PGUSER", b"postgres", &mut db.user);
        part(password, c"PGPASSWORD", b"", &mut db.password);
        part(host, c"PGHOST", b"localhost", &mut db.host);
        let user_again = core::slice::from_raw_parts(db.user.ptr, db.user.len);
        part(database, c"PGDATABASE", user_again, &mut db.database);
        let settings = options
            .split(|byte| *byte == b'&')
            .find_map(|option| option.strip_prefix(b"options="))
            .unwrap_or(b"");
        unescaped(settings, &mut db.options);
        unescaped(authorities, &mut db.authorities);
        // A NUL ends a word of what is said to the server when the connection is set
        // up: one inside a part of the address would begin another word of its own.
        let whole = [&db.user, &db.password, &db.host, &db.database, &db.options]
            .iter()
            .all(|part| !part.bytes().contains(&0));
        if !whole {
            db.used = false;
            failed(CONNECTION, &[b"the address of the database has a NUL in it (`%00`)"]);
            return -1;
        }
        db.secure = secure;
        db.patience = patience.max(1);
        db.port = port_number as u16;
        db.limit = (limit.max(1) as usize).min(MOST_CONNS);
        // One connection now, so that an address that leads nowhere is known at once.
        match check_out(db) {
            Some(conn) => {
                give_back(conn, false);
                index as i64
            }
            None => {
                db.used = false;
                -1
            }
        }
    }
}

/// Sends a statement and reads the answer. Returns whether the server did what was
/// asked; when it sends an error it goes on to say when it is ready again, and all of
/// that is read, so that the connection is good for the next statement.
unsafe fn exchange(conn: &mut Conn, request: &[u8], out: &mut Outcome) -> Result<bool, bool> {
    unsafe {
        if !send(conn, request) {
            return Err(true);
        }
        conn.asked = true;
        let mut ok = true;
        // The statement is with the server. If the connection ends now, the server may
        // have done the statement or not, and it is not sent again; unless all the
        // server said was that it is shutting down, which it says in place of doing
        // anything. (A statement that could not be sent whole, above, was not done: it
        // is the last message of it that makes the server keep what it did.)
        let (mut goodbye, mut more) = (false, false);
        let mut refused_copy = false;
        loop {
            let Some((kind, body)) = message(conn) else {
                return Err(goodbye && !more);
            };
            let farewell = kind == b'E' && {
                let code = error_fields(body).0;
                code.starts_with(b"08") || code.starts_with(b"57P")
            };
            goodbye |= farewell;
            more |= !farewell;
            match kind {
                b'T' if body.len() >= 2 => {
                    out.cols = u16::from_be_bytes([body[0], body[1]]) as usize;
                    let mut rest = &body[2..];
                    for _ in 0..out.cols {
                        let end = rest.iter().position(|byte| *byte == 0).unwrap_or(rest.len());
                        out.names.push(&rest[..end]);
                        out.names.push(&[0]);
                        rest = rest.get(end + 19..).unwrap_or(&[]);
                    }
                }
                b'D' if body.len() >= 2 => {
                    let count = u16::from_be_bytes([body[0], body[1]]) as usize;
                    let mut rest = &body[2..];
                    for _ in 0..count {
                        let Some(head) = rest.get(..4) else { break };
                        let length = i32::from_be_bytes([head[0], head[1], head[2], head[3]]);
                        rest = &rest[4..];
                        // A value is as long as what arrived of it, whatever was said.
                        let value = match length {
                            1.. => rest.get(..length as usize).unwrap_or(rest),
                            _ => &[],
                        };
                        let length = if length < 0 { -1 } else { value.len() as i32 };
                        out.cells.push(&(out.data.len as u32).to_ne_bytes());
                        out.cells.push(&length.to_ne_bytes());
                        out.data.push(value);
                        rest = &rest[value.len()..];
                    }
                    out.rows += 1;
                }
                b'C' => {
                    // `INSERT 0 3`, `UPDATE 3`, `SELECT 3`: the last number is the count.
                    let tag = body.split(|byte| *byte == 0).next().unwrap_or(&[]);
                    out.rolled_back = tag == b"ROLLBACK";
                    let last = tag.rsplit(|byte| *byte == b' ').next().unwrap_or(&[]);
                    if !last.is_empty() && last.iter().all(u8::is_ascii_digit) {
                        out.affected = last.iter().fold(0i64, |sum, digit| {
                            sum.saturating_mul(10).saturating_add((*digit - b'0') as i64)
                        });
                    }
                }
                b'E' => {
                    server_error(body);
                    // What the server calls a statement that was called off was a
                    // statement this program could not go through with.
                    if refused_copy {
                        outcome().kind = OTHER;
                    }
                    ok = false;
                }
                // `copy ... from stdin`: the server waits for the rows, which a program
                // has no way to send. It is told that none will come, and answers that
                // the statement failed.
                b'G' | b'W' => {
                    let mut refusal = Buf::new();
                    let mut fail = Message::begin(&mut refusal, b'f');
                    fail.text(b"`copy ... from stdin` is not something a Koo program can do: insert the rows");
                    fail.end();
                    // The end of the statement was said before the server began to wait
                    // for rows, and was not heard then: it is said again.
                    Message::begin(&mut refusal, b'S').end();
                    let told = send(conn, refusal.bytes());
                    refusal.release();
                    refused_copy = true;
                    if !told {
                        return Err(false);
                    }
                }
                b'Z' => {
                    conn.asked = false;
                    return Ok(ok);
                }
                _ => {}
            }
        }
    }
}

/// A statement as the messages that send it: the statement, its parameters, a request
/// for the names of the columns, the order to run it, and the word that this is all.
unsafe fn request(text: &[u8], params: *const Header, nulls: &[u8]) -> Buf {
    unsafe {
        let count = if params.is_null() { 0 } else { (*params).len as usize };
        let values = (params as *const u8).wrapping_add(HEADER) as *const *const Header;
        let mut request = Buf::new();
        let mut parse = Message::begin(&mut request, b'P');
        parse.text(b"");
        parse.text(text);
        parse.int16(0);
        parse.end();
        let mut bind = Message::begin(&mut request, b'B');
        bind.text(b"");
        bind.text(b"");
        bind.int16(0);
        bind.int16(count as i16);
        for index in 0..count {
            if nulls.get(index) == Some(&b't') {
                bind.int32(-1);
            } else {
                let value = bytes(*values.add(index));
                bind.int32(value.len() as i32);
                bind.bytes(value);
            }
        }
        bind.int16(0);
        bind.end();
        let mut describe = Message::begin(&mut request, b'D');
        describe.bytes(b"P");
        describe.text(b"");
        describe.end();
        let mut execute = Message::begin(&mut request, b'E');
        execute.text(b"");
        execute.int32(0);
        execute.end();
        Message::begin(&mut request, b'S').end();
        request
    }
}

/// Forgets what the thread kept of its last statement.
unsafe fn fresh<'a>() -> &'a mut Outcome {
    unsafe {
        let out = outcome();
        out.kind = OTHER;
        out.error.len = 0;
        out.code = *b"     ";
        (out.cols, out.rows, out.affected) = (0, 0, 0);
        (out.names.len, out.cells.len, out.data.len) = (0, 0, 0);
        out.rolled_back = false;
        out
    }
}

#[allow(static_mut_refs)]
unsafe fn database<'a>(db: i64) -> Option<&'a mut Db> {
    unsafe {
        if db < 0 || db as usize >= MOST_DBS || !DBS[db as usize].used {
            failed(CONNECTION, &[b"this database is not open"]);
            return None;
        }
        Some(&mut DBS[db as usize])
    }
}

/// Runs a statement on any connection of the pool, trying once more on another if the
/// first turns out to have been closed while it was idle. With `keep`, a connection on
/// which the statement ran is not given back, and its place in the pool is returned.
unsafe fn run_on_any(db: &mut Db, request: &[u8], keep: bool) -> Option<usize> {
    unsafe {
        let out = outcome();
        let first = db.conns.as_ptr();
        let mut sent_again = false;
        // At most every connection of the pool can turn out to be dead, and one more.
        for _ in 0..MOST_CONNS + 2 {
            let conn = check_out(db)?;
            // A connection that has been idle has nothing to say. If it has, the server
            // has closed it, or is about to: it is not used, and another is taken.
            if has_something(conn.fd) {
                give_back(conn, true);
                continue;
            }
            let index = (conn as *const Conn).offset_from(first) as usize;
            match exchange(conn, request, out) {
                Ok(ok) => {
                    if !(ok && keep) {
                        give_back(conn, false);
                    }
                    return ok.then_some(index);
                }
                Err(untouched) => {
                    give_back(conn, true);
                    if untouched && !sent_again {
                        sent_again = true;
                        fresh();
                        continue;
                    }
                    // The statement went out whole and no answer came back: it must
                    // not be sent again, since the database may have done it.
                    failed(
                        CONNECTION,
                        &[if untouched {
                            b"the connection to the database was lost" as &[u8]
                        } else {
                            b"the connection to the database was lost before it answered: the statement may have been done"
                        }],
                    );
                    return None;
                }
            }
        }
        failed(CONNECTION, &[b"the connection to the database was lost"]);
        None
    }
}

/// Whether a socket has something to read, or has been closed, without waiting.
pub(crate) unsafe fn has_something(fd: i32) -> bool {
    unsafe {
        let mut fds = PollFd {
            fd,
            events: 1,
            revents: 0,
        };
        poll(&mut fds, 1, 0) > 0
    }
}

/// The connection of a transaction, if the transaction is still open and the calling
/// thread is the one that began it.
unsafe fn in_transaction(db: &mut Db, transaction: i64) -> Option<&mut Conn> {
    unsafe {
        let (index, serial) = ((transaction & 0xff) as usize, (transaction >> 8) as u32);
        let conn = db
            .conns
            .get_mut(index)
            .filter(|conn| conn.transaction == serial && serial != 0);
        let Some(conn) = conn else {
            failed(
                OTHER,
                &[b"this transaction is over: it is used only inside the function given to `transaction`"],
            );
            return None;
        };
        if conn.began != me() as *mut thread::Thread as usize {
            failed(
                OTHER,
                &[b"a transaction is used only by the thread that began it: an actor that is told of one runs elsewhere"],
            );
            return None;
        }
        if conn.fd < 0 {
            failed(
                CONNECTION,
                &[b"the connection to the database was lost, and the transaction with it"],
            );
            return None;
        }
        Some(conn)
    }
}

/// Runs a statement with its parameters, each as text, or as NULL where `nulls` has
/// `t`; inside a transaction if one is given, and otherwise on any connection. Returns
/// whether it ran; the rows, or the error, are kept for the calling thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_run(
    db: i64,
    transaction: i64,
    text: *const Header,
    params: *const Header,
    nulls: *const Header,
) -> bool {
    unsafe {
        koo_flush();
        let out = fresh();
        // A handler whose request was given up begins nothing more.
        if crate::http::cut_short() {
            failed(TIMEOUT, &[GIVEN_UP]);
            return false;
        }
        let Some(db) = database(db) else { return false };
        if !params.is_null() && (*params).len > 65535 {
            failed(OTHER, &[b"a statement has at most 65,535 parameters"]);
            return false;
        }
        if transaction == 0 {
            let mut request = request(bytes(text), params, bytes(nulls));
            let ran = run_on_any(db, request.bytes(), false).is_some();
            request.release();
            return ran;
        }
        let Some(conn) = in_transaction(db, transaction) else {
            return false;
        };
        let mut request = request(bytes(text), params, bytes(nulls));
        let ran = exchange(conn, request.bytes(), out);
        request.release();
        ran.unwrap_or_else(|_| {
            // The transaction is lost with its connection; `koo_sql_end` gives the
            // place in the pool back.
            hang_up(conn, false);
            failed(
                CONNECTION,
                &[b"the connection to the database was lost, and the transaction with it"],
            );
            false
        })
    }
}

/// Begins a transaction, at one of the three levels at which PostgreSQL keeps
/// transactions apart, and returns its number, or zero if it could not begin. The
/// connection is the transaction's until `koo_sql_end`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_begin(db: i64, level: i64) -> i64 {
    unsafe {
        koo_flush();
        fresh();
        if crate::http::cut_short() {
            failed(TIMEOUT, &[GIVEN_UP]);
            return 0;
        }
        let Some(db) = database(db) else { return 0 };
        let text: &[u8] = match level {
            1 => b"begin isolation level repeatable read",
            2 => b"begin isolation level serializable",
            _ => b"begin",
        };
        let mut request = request(text, core::ptr::null(), b"");
        let kept = run_on_any(db, request.bytes(), true);
        request.release();
        let Some(index) = kept else { return 0 };
        let conn = &mut db.conns[index];
        conn.serial = if conn.serial >= 0x3fff_ffff { 1 } else { conn.serial + 1 };
        conn.transaction = conn.serial;
        conn.began = me() as *mut thread::Thread as usize;
        ((conn.serial as i64) << 8) | index as i64
    }
}

/// Ends a transaction, keeping what it did or undoing it, and gives its connection
/// back. Returns whether the database did so; a transaction in which a statement
/// failed cannot be kept, and to try is an error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_end(db: i64, transaction: i64, keep: bool) -> bool {
    unsafe {
        koo_flush();
        let out = fresh();
        let Some(db) = database(db) else { return false };
        let Some(conn) = in_transaction(db, transaction) else {
            // Lost with its connection: only the place in the pool is left to give back.
            let index = (transaction & 0xff) as usize;
            if let Some(conn) = db.conns.get_mut(index)
                && conn.transaction == (transaction >> 8) as u32
                && conn.transaction != 0
                && conn.began == me() as *mut thread::Thread as usize
            {
                conn.transaction = 0;
                give_back(conn, true);
            }
            return false;
        };
        // A transaction of a handler whose request was given up is undone, whichever
        // was asked: by closing its connection, which the database takes for that, and
        // not by asking, whose answer the handler would not be let wait for. So what
        // was done for a client that left is not half kept and half unknown.
        if crate::http::given_up() {
            conn.transaction = 0;
            give_back(conn, true);
            failed(TIMEOUT, &[GIVEN_UP]);
            return false;
        }
        let mut request = request(if keep { b"commit" } else { b"rollback" }, core::ptr::null(), b"");
        let ended = exchange(conn, request.bytes(), out);
        request.release();
        conn.transaction = 0;
        give_back(conn, ended.is_err());
        match ended {
            Ok(true) if keep && out.rolled_back => {
                failed(
                    OTHER,
                    &[b"the transaction was undone, not kept: a statement in it had failed, and the database does nothing more in a transaction after that"],
                );
                false
            }
            Ok(ok) => ok,
            Err(_) => {
                failed(
                    CONNECTION,
                    &[b"the connection to the database was lost, and the transaction with it"],
                );
                false
            }
        }
    }
}

/// When a thread recovers from a panic: the transactions it began are over, and their
/// connections are closed, which is how the server learns to undo them.
#[allow(static_mut_refs)]
pub unsafe fn abandon() {
    unsafe {
        if OPENED.load(Acquire) == 0 || OWNER.load(Acquire) != getpid() {
            return;
        }
        let thread = me() as *mut thread::Thread as usize;
        for db in DBS.iter_mut().filter(|db| db.used) {
            for conn in db.conns.iter_mut() {
                if conn.transaction != 0 && conn.began == thread {
                    conn.transaction = 0;
                    give_back(conn, true);
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_rows() -> i64 {
    unsafe { outcome().rows as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_cols() -> i64 {
    unsafe { outcome().cols as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_affected() -> i64 {
    unsafe { outcome().affected }
}

/// The name of a column of the last query.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_name(col: i64) -> *mut Header {
    unsafe {
        let name = outcome()
            .names
            .bytes()
            .split(|byte| *byte == 0)
            .nth(col.max(0) as usize)
            .unwrap_or(&[]);
        str_from_outside(name)
    }
}

unsafe fn cell(row: i64, col: i64) -> Option<&'static [u8]> {
    unsafe {
        let out = outcome();
        if row < 0 || col < 0 || row as usize >= out.rows || col as usize >= out.cols {
            return None;
        }
        let at = (row as usize * out.cols + col as usize) * 8;
        let entry = out.cells.bytes().get(at..at + 8)?;
        let offset = u32::from_ne_bytes([entry[0], entry[1], entry[2], entry[3]]) as usize;
        let length = i32::from_ne_bytes([entry[4], entry[5], entry[6], entry[7]]);
        if length < 0 {
            return None;
        }
        let data = out.data.bytes().get(offset..offset + length as usize)?;
        Some(core::slice::from_raw_parts(data.as_ptr(), data.len()))
    }
}

/// A value of the last query, as text; the empty text for NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_value(row: i64, col: i64) -> *mut Header {
    unsafe { str_from_outside(cell(row, col).unwrap_or(&[])) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_null(row: i64, col: i64) -> bool {
    unsafe { cell(row, col).is_none() }
}

/// Why the last query, or the last attempt to open a database, failed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_error() -> *mut Header {
    unsafe { str_from_outside(outcome().error.bytes()) }
}

/// The five characters by which the server names the error, or the empty text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_code() -> *mut Header {
    unsafe {
        let code = outcome().code;
        new_str(if code == *b"     " { b"" } else { &code })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sql_kind() -> i64 {
    unsafe { outcome().kind }
}

/// When `main` returns: the connections are closed, each with a word of farewell.
#[allow(static_mut_refs)]
pub unsafe fn at_exit() {
    unsafe {
        if OPENED.load(Acquire) == 0 || OWNER.load(Acquire) != getpid() {
            return;
        }
        for db in DBS.iter_mut() {
            for conn in db.conns.iter_mut() {
                if conn.fd >= 0 {
                    let _ = send(conn, &[b'X', 0, 0, 0, 4]);
                    hang_up(conn, true);
                }
                conn.input.release();
            }
            db.host.release();
            db.user.release();
            db.password.release();
            db.database.release();
            db.options.release();
            db.authorities.release();
            db.used = false;
        }
        thread::every(|thread| {
            if !thread.sql.is_null() {
                let out = &mut *(thread.sql as *mut Outcome);
                out.error.release();
                out.names.release();
                out.cells.release();
                out.data.release();
                free(thread.sql);
                thread.sql = core::ptr::null_mut();
            }
        });
    }
}
