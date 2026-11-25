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
    fn poll(fds: *mut PollFd, count: usize, milliseconds: i32) -> i32;
    fn getentropy(data: *mut u8, len: usize) -> i32;
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const AF_INET: i32 = 2;
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

/// What a file holds, or nothing if it cannot be read. Only the first 64 KB.
unsafe fn file(path: &core::ffi::CStr) -> Buf {
    unsafe {
        let mut out = Buf::new();
        let file = fopen(path.as_ptr() as *const u8, c"r".as_ptr() as *const u8);
        if file.is_null() {
            return out;
        }
        out.room(65536);
        out.len = fread(out.ptr, 1, 65536, file);
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

/// Asks a name server for the address of a name. `query` and `answer` are the two
/// datagrams: the question is the name as labels, each with its length before it.
/// Returns nothing if the server did not answer, and otherwise the address, or that
/// there is none.
unsafe fn ask_name_server(server: [u8; 4], name: &[u8]) -> Option<Option<[u8; 4]>> {
    unsafe {
        let mut query = [0u8; 300];
        let mut id = [0u8; 2];
        getentropy(id.as_mut_ptr(), 2);
        query[..2].copy_from_slice(&id);
        query[2] = 1; // the server may ask others
        query[5] = 1; // one question
        let mut at = 12;
        for label in name.split(|byte| *byte == b'.').filter(|label| !label.is_empty()) {
            if label.len() > 63 || at + label.len() + 6 > query.len() {
                return None;
            }
            query[at] = label.len() as u8;
            query[at + 1..at + 1 + label.len()].copy_from_slice(label);
            at += 1 + label.len();
        }
        // The end of the name, then: an address, of the Internet.
        query[at + 2] = 1;
        query[at + 4] = 1;
        let length = at + 5;
        let fd = socket(AF_INET, SOCK_DGRAM, 0);
        if fd < 0 {
            return None;
        }
        let (address, address_len) = socket_address(server, 53);
        let mut found = None;
        for _ in 0..2 {
            if sendto(fd, query.as_ptr(), length, 0, address.as_ptr(), address_len) < 0 {
                break;
            }
            let mut fds = PollFd {
                fd,
                events: 1,
                revents: 0,
            };
            if poll(&mut fds, 1, 2000) <= 0 {
                continue;
            }
            let mut answer = [0u8; 1500];
            let got = recv(fd, answer.as_mut_ptr(), answer.len(), 0);
            if got < 12 || answer[..2] != id {
                continue;
            }
            found = Some(address_in(&answer[..got as usize]));
            break;
        }
        close(fd);
        found
    }
}

/// The first address in a name server's answer.
fn address_in(answer: &[u8]) -> Option<[u8; 4]> {
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
        if kind == 1 && length == 4 {
            return answer.get(at..at + 4)?.try_into().ok();
        }
        at += length;
    }
    None
}

/// The address of a host: as it is written, from the machine's table of hosts, or from
/// the name servers the machine is set up with, trying the name inside each of the
/// domains it searches, as the system's own resolver does.
unsafe fn resolve(host: &[u8]) -> Option<[u8; 4]> {
    unsafe {
        if let Some(address) = dotted(host) {
            return Some(address);
        }
        if host.eq_ignore_ascii_case(b"localhost") {
            return Some([127, 0, 0, 1]);
        }
        let mut hosts = file(c"/etc/hosts");
        let mut found = None;
        for line in hosts.bytes().split(|byte| *byte == b'\n') {
            let mut parts = words(line);
            let Some(address) = parts.next().and_then(dotted) else {
                continue;
            };
            if parts.any(|name| name.eq_ignore_ascii_case(host)) {
                found = Some(address);
                break;
            }
        }
        hosts.release();
        if found.is_some() {
            return found;
        }
        let mut settings = file(c"/etc/resolv.conf");
        let mut servers = [[0u8; 4]; 3];
        let mut server_count = 0;
        let mut dots_wanted = 1;
        let mut search: &[u8] = &[];
        for line in settings.bytes().split(|byte| *byte == b'\n') {
            let mut parts = words(line);
            match parts.next() {
                Some(b"nameserver") => {
                    if let Some(address) = parts.next().and_then(dotted)
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
            servers[0] = [127, 0, 0, 1];
            server_count = 1;
        }
        let dots = host.iter().filter(|byte| **byte == b'.').count();
        // A name with few dots is looked for in the searched domains first.
        let alone_first = dots >= dots_wanted || host.ends_with(b".");
        let mut answer = None;
        // The first server that answers is believed, whatever it says.
        let ask = |name: &[u8]| {
            servers[..server_count]
                .iter()
                .find_map(|server| ask_name_server(*server, name))
                .flatten()
        };
        if alone_first {
            answer = ask(host);
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
                answer = ask(&name[..host.len() + 1 + domain.len()]);
                if answer.is_some() {
                    break;
                }
            }
        }
        if answer.is_none() && !alone_first {
            answer = ask(host);
        }
        settings.release();
        answer
    }
}

/// An address and a port as the system wants them.
fn socket_address(address: [u8; 4], port: u16) -> ([u8; 16], u32) {
    let mut out = [0u8; 16];
    if cfg!(target_os = "macos") {
        out[0] = 16;
        out[1] = AF_INET as u8;
    } else {
        out[..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes());
    }
    out[2..4].copy_from_slice(&port.to_be_bytes());
    out[4..8].copy_from_slice(&address);
    (out, 16)
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

/// Writes to a connection, through its encryption if it has one.
unsafe fn send(conn: &Conn, data: &[u8]) -> bool {
    unsafe {
        if conn.tls.is_null() {
            send_all(conn.fd, data)
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

/// Reads until `conn.input` holds `need` bytes past `conn.at`.
unsafe fn fill(conn: &mut Conn, need: usize) -> bool {
    unsafe {
        if conn.at > 0 && conn.at == conn.input.len {
            conn.at = 0;
            conn.input.len = 0;
        }
        while conn.input.len - conn.at < need {
            conn.input.room(need.max(8192));
            let room =
                core::slice::from_raw_parts_mut(conn.input.ptr.add(conn.input.len), conn.input.cap - conn.input.len);
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
                failed(
                    CONNECTION,
                    &[b"the server closed the connection while it was being set up"],
                );
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

/// Connects to a host, giving it so many seconds to answer and not the minutes the
/// system would wait. Returns the socket, or -1 with the reason written into `why`.
pub(crate) unsafe fn dial(host: &[u8], port: u16, seconds: i32, why: &mut Buffer) -> i32 {
    unsafe {
        let Some(address) = resolve(host) else {
            why.push(b"there is no address for the host `");
            why.push(host);
            why.push(b"`");
            return -1;
        };
        let fd = socket(AF_INET, SOCK_STREAM, 0);
        let (target, target_len) = socket_address(address, port);
        let seconds = seconds.max(1);
        let patience: [i64; 2] = [seconds as i64, 0];
        if fd >= 0 {
            if cfg!(target_os = "macos") {
                setsockopt(fd, IPPROTO_TCP, 0x20, &seconds, 4);
            } else {
                setsockopt(fd, 1, 21, patience.as_ptr() as *const i32, 16);
            }
        }
        let connected = fd >= 0 && connect(fd, target.as_ptr(), target_len) == 0;
        if connected && !cfg!(target_os = "macos") {
            let forever: [i64; 2] = [0, 0];
            setsockopt(fd, 1, 21, forever.as_ptr() as *const i32, 16);
        }
        if !connected {
            // What the system calls a connection that was not answered in its time.
            let slow = *errno_location() == if cfg!(target_os = "macos") { 60 } else { 115 };
            why.push(b"cannot connect to ");
            why.push(host);
            let _ = write!(why, ":{port}: ");
            if slow {
                let _ = write!(why, "the server did not answer in {seconds} seconds");
            } else {
                why.push(system_error());
            }
            if fd >= 0 {
                close(fd);
            }
            return -1;
        }
        let on = 1i32;
        setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &on, 4);
        if cfg!(target_os = "macos") {
            // No signal for a write to a server that has gone.
            setsockopt(fd, 0xffff, 0x1022, &on, 4);
        }
        fd
    }
}

/// Connects to a database and waits until it is ready for a query.
unsafe fn establish(conn: &mut Conn, db: &Db) -> bool {
    unsafe {
        let host = db.host.bytes();
        let mut why = Buffer::new();
        let fd = dial(host, db.port, 10, &mut why);
        if fd < 0 {
            failed(CONNECTION, &[&why.data[..why.len]]);
            return false;
        }
        conn.fd = fd;
        conn.at = 0;
        conn.input.len = 0;
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
            if !asked || read(fd, answer.as_mut_ptr(), 1) != 1 {
                failed(
                    CONNECTION,
                    &[b"the server closed the connection while it was being set up"],
                );
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
        while ready {
            match message(conn) {
                Some((b'Z', _)) => break,
                Some((b'E', body)) => {
                    server_error(body);
                    outcome().kind = CONNECTION;
                    ready = false;
                }
                Some(_) => {}
                None => {
                    failed(
                        CONNECTION,
                        &[b"the server closed the connection while it was being set up"],
                    );
                    ready = false;
                }
            }
        }
        if !ready {
            hang_up(conn, false);
        }
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

/// A connection for the calling thread: one that is idle, or a new one if there may be
/// more, or the first to be given back. Nothing if one could not be made.
#[allow(static_mut_refs)]
unsafe fn check_out(db: &mut Db) -> Option<&mut Conn> {
    unsafe {
        adopt();
        POOL.lock();
        let slot = loop {
            if let Some(index) = db.conns.iter().position(|conn| conn.fd >= 0 && !conn.busy) {
                db.conns[index].busy = true;
                POOL.unlock();
                return Some(&mut db.conns[index]);
            }
            let open = db.conns.iter().filter(|conn| conn.fd >= 0 || conn.busy).count();
            if open < db.limit
                && let Some(index) = db.conns.iter().position(|conn| conn.fd < 0 && !conn.busy)
            {
                db.conns[index].busy = true;
                break index;
            }
            POOL.wait();
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
        POOL.wake_one();
        POOL.unlock();
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

/// Opens a database: `postgres://user:password@host:port/database`. Returns its number,
/// or -1 if the address is not understood or the database cannot be reached; a thread
/// then finds why in `koo_sql_error`.
#[unsafe(no_mangle)]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn koo_sql_open(url: *const Header, limit: i64) -> i64 {
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
        // Whether connections are encrypted, by the names PostgreSQL gives the ways.
        let setting = |name: &[u8]| {
            options
                .split(|byte| *byte == b'&')
                .find_map(|option| option.strip_prefix(name)?.strip_prefix(b"="))
        };
        let authorities = setting(b"sslrootcert").unwrap_or(b"");
        let secure = match setting(b"sslmode") {
            None | Some(b"prefer") => 1,
            Some(b"disable") | Some(b"allow") => 0,
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
        let (host, port) = match place.iter().rposition(|byte| *byte == b':') {
            Some(at) => (&place[..at], &place[at + 1..]),
            None => (place, &b"5432"[..]),
        };
        let port_number = port
            .iter()
            .try_fold(0u32, |sum, digit| {
                digit.is_ascii_digit().then(|| sum * 10 + (*digit - b'0') as u32)
            })
            .filter(|number| !port.is_empty() && *number > 0 && *number < 65536);
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
        unescaped(if user.is_empty() { b"postgres" } else { user }, &mut db.user);
        unescaped(password, &mut db.password);
        unescaped(if host.is_empty() { b"localhost" } else { host }, &mut db.host);
        let user_again = core::slice::from_raw_parts(db.user.ptr, db.user.len);
        unescaped(
            if database.is_empty() { user_again } else { database },
            &mut db.database,
        );
        let settings = options
            .split(|byte| *byte == b'&')
            .find_map(|option| option.strip_prefix(b"options="))
            .unwrap_or(b"");
        unescaped(settings, &mut db.options);
        unescaped(authorities, &mut db.authorities);
        db.secure = secure;
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
        let mut ok = true;
        // An idle connection that the server closed while nobody was looking is found
        // out before the server has said anything of the statement: at most it has said
        // that it is shutting down. Then nothing was done, and the statement may be
        // sent again on another connection.
        let mut untouched = true;
        loop {
            let Some((kind, body)) = message(conn) else {
                return Err(untouched);
            };
            let farewell = kind == b'E' && {
                let code = error_fields(body).0;
                code.starts_with(b"08") || code.starts_with(b"57P")
            };
            untouched = untouched && farewell;
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
                    ok = false;
                }
                b'Z' => return Ok(ok),
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
        for attempt in 0..2 {
            let conn = check_out(db)?;
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
                    if !(untouched && attempt == 0) {
                        failed(CONNECTION, &[b"the connection to the database was lost"]);
                        return None;
                    }
                    fresh();
                }
            }
        }
        None
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
