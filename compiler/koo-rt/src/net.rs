//! Conversations with other programs over TCP, behind `std/net`.
//!
//! A program has a connection only for as long as a function it gives is running: the
//! function ends, by returning, failing or panicking, and the connection is given back
//! or closed. So there is no connection for a program to forget to close, and no number
//! of one that still means anything once its function is over (the number has a serial
//! in it, which is then another's).
//!
//! Connections to one address are kept for the next conversation with it, up to as many
//! as the program says. One is kept only if nothing went wrong while it was out and
//! nothing is waiting to be read on it: this library does not know what is spoken over a
//! connection, and an answer that arrives late, to a question whose asker has gone, must
//! not be the answer to the next question. The connections are a process's own: one that
//! was made before a server started its workers is not used by them.
//!
//! Every wait has a time, and a handler of a server that waits here is left where it is
//! while its worker answers other requests, as with a database (`pg.rs`).

use super::pg::{Buf, UNANSWERED, dial, has_something, ready, send_within};
use super::thread::{Lock, me};
use super::*;
use core::sync::atomic::AtomicI32;
use core::sync::atomic::Ordering::{AcqRel, Acquire, Release};

unsafe extern "C" {
    fn getpid() -> i32;
    fn close(fd: i32) -> i32;
    fn recv(fd: i32, data: *mut u8, len: usize, flags: i32) -> isize;
    fn setsockopt(fd: i32, level: i32, name: i32, value: *const i32, len: u32) -> i32;
}

// The kinds of failure, as `std/net` numbers them.
const ADDRESS: i64 = 1;
const CONNECTION: i64 = 2;
const TIMEOUT: i64 = 3;
const CLOSED: i64 = 4;
const TOO_LONG: i64 = 5;
const NOT_TEXT: i64 = 6;

/// How many connections a process has at once, and to how many addresses it keeps them.
const MOST_CONNS: usize = 1024;
const MOST_POOLS: usize = 64;
/// The most that one read gives a program, whatever it asks for or its peer says.
const MOST_READ: usize = 1 << 28;
/// How long a connection that nothing uses is kept, in nanoseconds.
const KEPT_FOR: i64 = 60_000_000_000;

struct Conn {
    fd: i32,
    /// Which of the conversations that have had this place it is: the number a program
    /// has of a connection is of one conversation, and of no later one.
    serial: u32,
    /// The address it is kept for, among `POOLS`, or `ALONE` for one that was made
    /// for a single conversation and is kept for no other.
    pool: usize,
    /// Whether a function of the program has it, and which thread that function is on.
    out: bool,
    holder: usize,
    /// Whether anything went wrong on it while it was out: it is then not kept. Nor
    /// is one that was made beyond the size of its pool (`beyond`).
    broken: bool,
    beyond: bool,
    /// Whether anything has ever been written to it or read from it.
    spoken: bool,
    /// What has been read from it and not yet given to the program, from `at` on.
    input: Buf,
    at: usize,
    /// What the last read gave, for the program to take.
    taken: Buf,
    /// When it was given back, for one that nothing uses.
    since: i64,
    /// Whether nothing had been said on it when the conversation it is in began: what
    /// is said first on a connection, a greeting or who is asking, is said then.
    fresh: bool,
}

/// The pool of a connection that has none.
const ALONE: usize = usize::MAX;

const NO_CONN: Conn = Conn {
    fd: -1,
    serial: 0,
    pool: 0,
    out: false,
    holder: 0,
    broken: false,
    beyond: false,
    spoken: false,
    input: Buf::new(),
    at: 0,
    taken: Buf::new(),
    since: 0,
    fresh: false,
};

struct Pool {
    used: bool,
    /// `host:port`, as the program wrote it, and after a line feed the name of the
    /// pool, which may be empty.
    address: Buf,
    /// How many connections it has, in use or not.
    open: usize,
    /// Those that wait for one, who have it in the order they came.
    turns: crate::thread::Turns,
}

const NO_POOL: Pool = Pool {
    used: false,
    address: Buf::new(),
    open: 0,
    turns: crate::thread::Turns::new(),
};

static NET: Lock = Lock::new();
static mut CONNS: [Conn; MOST_CONNS] = [NO_CONN; MOST_CONNS];
static mut POOLS: [Pool; MOST_POOLS] = [NO_POOL; MOST_POOLS];
static mut SERIAL: u32 = 0;
/// The process whose connections these are.
static OWNER: AtomicI32 = AtomicI32::new(0);

/// Notes why something failed, and returns the kind of failure.
unsafe fn failed(kind: i64, parts: &[&[u8]]) -> i64 {
    unsafe {
        let length: usize = parts.iter().map(|part| part.len()).sum();
        let message = koo_str_new(length as i64);
        let mut at = (message as *mut u8).add(HEADER);
        for part in parts {
            core::ptr::copy_nonoverlapping(part.as_ptr(), at, part.len());
            at = at.add(part.len());
        }
        (*message).len = length as i64;
        failed_with(message);
        kind
    }
}

/// In a process that did not make them, the connections are another's: a worker of a
/// server has those its server made before it started workers, and the two must not
/// speak on one connection. They are closed here, which says nothing to the other end,
/// since the process that made them has them still.
///
/// `OWNER` is the process whose connections these are; or 0, before any process has
/// had one; or a process's number with its sign changed, while a thread of that process
/// is making them its own. Two threads that come here at once, in a process that has
/// not used a connection before, must not each take the other for the process it was
/// forked from: one makes the tables the process's, and the other waits for it.
#[allow(static_mut_refs)]
unsafe fn adopt() {
    unsafe {
        let pid = getpid();
        loop {
            let owner = OWNER.load(Acquire);
            if owner == pid {
                return;
            }
            if owner == -pid {
                core::hint::spin_loop();
                continue;
            }
            if OWNER.compare_exchange(owner, -pid, AcqRel, Acquire).is_err() {
                continue;
            }
            if owner != 0 {
                // What is here was another process's. (After a fork there is one
                // thread, and the lock may have been held by one that is not here.)
                NET.unlock();
                for conn in CONNS.iter_mut() {
                    if conn.fd >= 0 {
                        close(conn.fd);
                    }
                    conn.input.release();
                    conn.taken.release();
                    *conn = NO_CONN;
                }
                for pool in POOLS.iter_mut() {
                    pool.address.release();
                    *pool = NO_POOL;
                }
            }
            OWNER.store(pid, Release);
            return;
        }
    }
}

/// The connection a number is of, if the number is of a conversation that is still
/// going on, on the calling thread.
#[allow(static_mut_refs)]
unsafe fn conn_of<'a>(number: i64) -> Result<&'a mut Conn, i64> {
    unsafe {
        let (index, serial) = ((number & 0xffff) as usize, (number >> 16) as u32);
        let over = || {
            failed(
                CLOSED,
                &[b"this connection is over: one is used only inside the function it was given to"],
            )
        };
        if number <= 0 || index >= MOST_CONNS || OWNER.load(Acquire) != getpid() {
            return Err(over());
        }
        let conn = &mut CONNS[index];
        if !conn.out || conn.serial != serial || serial == 0 {
            return Err(over());
        }
        if conn.holder != me() as *mut thread::Thread as usize {
            return Err(failed(
                CLOSED,
                &[b"a connection is used only by the thread it was given to: an actor that is told of one runs elsewhere"],
            ));
        }
        if conn.fd < 0 {
            return Err(failed(CLOSED, &[b"the connection was closed"]));
        }
        Ok(conn)
    }
}

/// Closes a connection and frees its place; and the place of its address among those
/// that connections are kept to, when this was the last of them. Called with the lock.
#[allow(static_mut_refs)]
unsafe fn drop_conn(conn: &mut Conn) {
    unsafe {
        if conn.fd >= 0 {
            close(conn.fd);
        }
        conn.input.release();
        conn.taken.release();
        if conn.pool != ALONE {
            let pool = &mut POOLS[conn.pool];
            pool.open = pool.open.saturating_sub(1);
            if pool.open == 0 {
                pool.used = false;
                pool.address.len = 0;
            }
        }
        *conn = NO_CONN;
    }
}

/// `host:port`, with the host in brackets if it is an address of the new kind.
fn parts(address: &[u8]) -> Option<(&[u8], u16)> {
    let colon = address.iter().rposition(|byte| *byte == b':')?;
    let (host, port) = (&address[..colon], &address[colon + 1..]);
    let host = match (host.first(), host.last()) {
        (Some(b'['), Some(b']')) => &host[1..host.len() - 1],
        _ if host.contains(&b':') => return None,
        _ => host,
    };
    let port = core::str::from_utf8(port)
        .ok()?
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)?;
    // (Nothing in a host that is not seen: an address has a name set after it, on a
    // line of its own, where connections are kept by both.)
    (!host.is_empty() && host.iter().all(|byte| *byte > b' ' && *byte != 0x7f)).then_some((host, port))
}

/// The place among `POOLS` of an address that connections are kept to: the one it
/// has, or a free one, or that of an address none of whose connections is in use,
/// which are then closed. Called with the lock.
#[allow(static_mut_refs)]
unsafe fn pool_of(address: &[u8]) -> Option<usize> {
    unsafe {
        if let Some(pool) = POOLS
            .iter()
            .position(|pool| pool.used && pool.address.bytes() == address)
        {
            return Some(pool);
        }
        let free = POOLS.iter().position(|pool| !pool.used).or_else(|| {
            let idle = (0..MOST_POOLS).find(|pool| !CONNS.iter().any(|conn| conn.pool == *pool && conn.out))?;
            for conn in CONNS.iter_mut() {
                if conn.pool == idle && conn.fd >= 0 {
                    drop_conn(conn);
                }
            }
            Some(idle)
        })?;
        POOLS[free].used = true;
        POOLS[free].address.len = 0;
        POOLS[free].address.push(address);
        POOLS[free].open = 0;
        POOLS[free].turns = crate::thread::Turns::new();
        Some(free)
    }
}

/// Takes a connection to an address for the calling thread: one that is kept, or a new
/// one if there are fewer than `limit`, or the next that is given back, waited for no
/// longer than `nanos`. With a `limit` of 0 the connection is made for this once, and
/// is no pool's. Returns the connection's number, or the kind of failure, negative.
#[unsafe(no_mangle)]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn koo_net_take(address: *const Header, name: *const Header, limit: i64, nanos: i64) -> i64 {
    unsafe {
        koo_flush();
        let (address, name) = (bytes(address), bytes(name));
        // A handler whose request was given up begins nothing more.
        if crate::http::cut_short() {
            return -failed(
                TIMEOUT,
                &[b"the request this was for was given up: its client has left, or its time is over"],
            );
        }
        if address.len() > 300 || name.len() > 200 {
            return -failed(
                ADDRESS,
                &[b"an address to connect to is no longer than 300 bytes, and the name of a pool no longer than 200"],
            );
        }
        let Some((host, port)) = parts(address) else {
            return -failed(
                ADDRESS,
                &[
                    b"`",
                    address,
                    b"` is not an address to connect to: one is `host:port`, as `cache.internal:6379` or `[::1]:6379`",
                ],
            );
        };
        adopt();
        // Connections are kept for an address and a name: what was said first on one, a
        // password or which database, is what the next conversation of the same name
        // relies on, and one of another name must not be given it.
        let mut kept_as = Buffer::new();
        kept_as.push(address);
        kept_as.push(b"\n");
        kept_as.push(name);
        let kept_as = &kept_as.data[..kept_as.len];
        let until = koo_now_nanos().saturating_add(nanos.max(0));
        let once = limit <= 0;
        let limit = limit.max(1) as usize;
        let holder = me() as *mut thread::Thread as usize;
        NET.lock();
        // The place a new connection is made in is this thread's while it is made,
        // without the lock: `Err`. One that is kept is taken as it is: `Ok`.
        let reserve = |pool: usize, beyond: bool| -> Option<usize> {
            let index = CONNS.iter().position(|conn| conn.fd < 0 && !conn.out)?;
            CONNS[index].out = true;
            CONNS[index].pool = pool;
            CONNS[index].holder = holder;
            CONNS[index].beyond = beyond;
            if pool != ALONE {
                POOLS[pool].open += 1;
            }
            Some(index)
        };
        let too_many = || {
            NET.unlock();
            -failed(CONNECTION, &[b"a program has at most 1,024 connections at once"])
        };
        // The number of this one among those that wait for a connection of the pool,
        // once it has had to.
        let mut number = 0u64;
        let place = if once {
            match reserve(ALONE, false) {
                Some(index) => Err(index),
                None => return too_many(),
            }
        } else {
            loop {
                let Some(pool) = pool_of(kept_as) else {
                    NET.unlock();
                    return -failed(
                        CONNECTION,
                        &[b"a program has conversations going on with at most sixty-four addresses at once"],
                    );
                };
                // One that is kept, if it is still good: the other end may have closed
                // it, or said something that nobody asked for, and then it is not.
                let now = koo_now_nanos();
                // (And a connection that is free is for whoever has waited longest.)
                let turn = POOLS[pool].turns.mine(number, now);
                let mut kept = None;
                for (index, conn) in CONNS.iter_mut().enumerate() {
                    if !turn || conn.fd < 0 || conn.out || conn.pool != pool {
                        continue;
                    }
                    if has_something(conn.fd) || now - conn.since > KEPT_FOR {
                        drop_conn(conn);
                        continue;
                    }
                    kept = Some(index);
                    break;
                }
                if let Some(index) = kept {
                    POOLS[pool].turns.done(number);
                    break Ok(index);
                }
                // (The last connection of the address may just have been closed, and
                // its place with it: it is found again.)
                let Some(pool) = pool_of(kept_as) else { continue };
                // Every connection is in a conversation. If each is a handler's of this
                // very thread, which waits somewhere, and this thread cannot be left,
                // because an actor's method runs on it, in a server of one process, then
                // waiting is for nothing: those handlers go on only when this thread
                // does. There is one connection more than the pool was to have, for
                // this conversation and no longer.
                let stuck = POOLS[pool].open >= limit
                    && CONNS
                        .iter()
                        .filter(|conn| conn.out && conn.pool == pool)
                        .all(|conn| crate::http::waits_elsewhere(conn.holder));
                if (turn && POOLS[pool].open < limit) || stuck {
                    POOLS[pool].turns.done(number);
                    match reserve(pool, stuck) {
                        Some(index) => break Err(index),
                        None => {
                            if POOLS[pool].open == 0 {
                                POOLS[pool].used = false;
                            }
                            return too_many();
                        }
                    }
                }
                if koo_now_nanos() >= until || crate::http::given_up() {
                    POOLS[pool].turns.done(number);
                    NET.unlock();
                    return -failed(
                        TIMEOUT,
                        &[
                            b"every connection to `",
                            address,
                            b"` was in use for as long as was waited",
                        ],
                    );
                }
                POOLS[pool].turns.wait(&mut number, until);
                NET.unlock();
                // A handler of a server lets the others go on, one of which has one,
                // and is woken when it is given back; and looks again before long
                // whether or not, since a connection is also freed by being closed.
                if !crate::http::await_turn(2_000_000) && !fiber::pause(1_000_000) {
                    koo_sleep(1_000_000);
                }
                NET.lock();
            }
        };
        let index = match place {
            Ok(index) => index,
            Err(index) => {
                NET.unlock();
                let mut why = Buffer::new();
                // What is left of the time; for a pool no less than a second, since one
                // that is told not to wait for a connection to be free may still make one.
                let left = ((until - koo_now_nanos()) / 1_000_000).max(if once { 1 } else { 1000 });
                let fd = dial(host, port, left, &mut why);
                NET.lock();
                if fd < 0 {
                    CONNS[index].out = false;
                    drop_conn(&mut CONNS[index]);
                    NET.unlock();
                    let kind = if fd == UNANSWERED { TIMEOUT } else { CONNECTION };
                    return -failed(kind, &[&why.data[..why.len]]);
                }
                // What is written goes at once: a conversation is questions and answers.
                let on = 1i32;
                setsockopt(fd, 6, 1, &on, 4);
                CONNS[index].fd = fd;
                CONNS[index].spoken = false;
                index
            }
        };
        SERIAL = if SERIAL >= 0x7fff_ffff { 1 } else { SERIAL + 1 };
        let conn = &mut CONNS[index];
        conn.serial = SERIAL;
        conn.out = true;
        conn.holder = holder;
        conn.broken = false;
        conn.fresh = !conn.spoken;
        conn.input.len = 0;
        conn.at = 0;
        conn.taken.len = 0;
        let number = ((conn.serial as i64) << 16) | index as i64;
        NET.unlock();
        number
    }
}

/// Gives a connection back when its function is over. It is kept for the next that
/// asks only if the function ended well, nothing failed on it, and the other end has
/// nothing more to say.
#[unsafe(no_mangle)]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn koo_net_give(number: i64, keep: bool) {
    unsafe {
        let (index, serial) = ((number & 0xffff) as usize, (number >> 16) as u32);
        if number <= 0 || index >= MOST_CONNS || OWNER.load(Acquire) != getpid() {
            return;
        }
        NET.lock();
        let conn = &mut CONNS[index];
        if conn.out && conn.serial == serial {
            let pool = conn.pool;
            let good = keep
                && pool != ALONE
                && !conn.broken
                && !conn.beyond
                && conn.fd >= 0
                && conn.at == conn.input.len
                && !has_something(conn.fd)
                && !crate::http::given_up();
            if good {
                conn.out = false;
                conn.holder = 0;
                conn.serial = 0;
                conn.since = koo_now_nanos();
                conn.input.release();
                conn.at = 0;
                conn.taken.release();
            } else {
                drop_conn(conn);
            }
            // And those of the address that nothing has used for a long while are
            // closed, here as where one is taken.
            let now = koo_now_nanos();
            for other in CONNS.iter_mut() {
                if pool != ALONE && other.fd >= 0 && !other.out && other.pool == pool && now - other.since > KEPT_FOR {
                    drop_conn(other);
                }
            }
        }
        NET.unlock();
        crate::http::give_turns();
    }
}

/// Whether nothing had been said on a connection when its conversation began.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_net_fresh(number: i64) -> bool {
    unsafe { conn_of(number).is_ok_and(|conn| conn.fresh) }
}

/// In a process just forked: the connections it has are the other process's, and are
/// closed here at once, whether or not this process ever has a conversation of its own.
/// A worker that kept them open would keep the other end from seeing that the process
/// which made them had closed them.
pub unsafe fn after_fork() {
    unsafe {
        if OWNER.load(Acquire) != 0 {
            adopt();
        }
    }
}

/// When a thread recovers from a panic: the connections its functions had are closed.
#[allow(static_mut_refs)]
pub unsafe fn abandon() {
    unsafe {
        if OWNER.load(Acquire) != getpid() {
            return;
        }
        let holder = me() as *mut thread::Thread as usize;
        NET.lock();
        for conn in CONNS.iter_mut() {
            if conn.out && conn.holder == holder && conn.serial != 0 {
                drop_conn(conn);
            }
        }
        NET.unlock();
    }
}

/// Writes to a connection, waiting no longer than `nanos` for the other end to take it.
unsafe fn write(number: i64, data: &[u8], nanos: i64) -> i64 {
    unsafe {
        let conn = match conn_of(number) {
            Ok(conn) => conn,
            Err(kind) => return kind,
        };
        let until = koo_now_nanos().saturating_add(nanos.max(1));
        conn.spoken = true;
        if send_within(conn.fd, data, until) {
            return 0;
        }
        conn.broken = true;
        if koo_now_nanos() >= until || crate::http::given_up() {
            failed(
                TIMEOUT,
                &[b"the other end did not take what was written in the time given"],
            )
        } else {
            failed(CLOSED, &[b"the connection was closed by the other end"])
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_net_write(number: i64, text: *const Header, nanos: i64) -> i64 {
    unsafe { write(number, bytes(text), nanos) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_net_write_bytes(number: i64, data: *const Header, nanos: i64) -> i64 {
    unsafe { write(number, bytes(data), nanos) }
}

/// Reads more of what the other end has sent into the connection's buffer, waiting for
/// it until `until`. Returns 0, or the kind of failure.
unsafe fn more(conn: &mut Conn, until: i64) -> i64 {
    unsafe {
        conn.spoken = true;
        // What the program has taken already is not kept: a stream that is read a
        // little at a time holds what is still to be read and no more.
        if conn.at > 0 {
            let left = conn.input.len - conn.at;
            core::ptr::copy(conn.input.ptr.add(conn.at), conn.input.ptr, left);
            conn.input.len = left;
            conn.at = 0;
        }
        loop {
            if !ready(conn.fd, false, until) {
                conn.broken = true;
                return failed(TIMEOUT, &[b"the other end did not answer in the time given"]);
            }
            conn.input.room(16384);
            let n = recv(
                conn.fd,
                conn.input.ptr.add(conn.input.len),
                conn.input.cap - conn.input.len,
                if cfg!(target_os = "macos") { 0x80 } else { 0x40 },
            );
            if n > 0 {
                conn.input.len += n as usize;
                return 0;
            }
            let error = *errno_location();
            // Interrupted, or nothing there after all: once more.
            if n < 0 && (error == 4 || error == if cfg!(target_os = "macos") { 35 } else { 11 }) {
                if koo_now_nanos() >= until {
                    conn.broken = true;
                    return failed(TIMEOUT, &[b"the other end did not answer in the time given"]);
                }
                continue;
            }
            conn.broken = true;
            return failed(CLOSED, &[b"the connection was closed by the other end"]);
        }
    }
}

/// What has been read and not given to the program is given, up to `count` bytes of it.
unsafe fn give(conn: &mut Conn, count: usize) {
    unsafe {
        conn.taken.len = 0;
        conn.taken
            .push(core::slice::from_raw_parts(conn.input.ptr.add(conn.at), count));
        conn.at += count;
        if conn.at == conn.input.len {
            conn.input.len = 0;
            conn.at = 0;
        }
    }
}

/// Reads whatever comes next, at least one byte and at most `most`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_net_read(number: i64, most: i64, nanos: i64) -> i64 {
    unsafe {
        let conn = match conn_of(number) {
            Ok(conn) => conn,
            Err(kind) => return kind,
        };
        let most = (most.max(1) as usize).min(MOST_READ);
        let until = koo_now_nanos().saturating_add(nanos.max(1));
        if conn.at == conn.input.len {
            let kind = more(conn, until);
            if kind != 0 {
                return kind;
            }
        }
        let count = (conn.input.len - conn.at).min(most);
        give(conn, count);
        0
    }
}

/// Reads exactly `count` bytes. What a program is told to expect by the other end is
/// not taken on trust: the bytes are kept as they come, and no room is made for more
/// than have come.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_net_read_exact(number: i64, count: i64, nanos: i64) -> i64 {
    unsafe {
        let conn = match conn_of(number) {
            Ok(conn) => conn,
            Err(kind) => return kind,
        };
        if count < 0 || count as usize > MOST_READ {
            conn.broken = true;
            return failed(
                TOO_LONG,
                &[if count < 0 {
                    b"fewer than no bytes cannot be read" as &[u8]
                } else {
                    b"more than 256 MB cannot be read at once"
                }],
            );
        }
        let count = count as usize;
        let until = koo_now_nanos().saturating_add(nanos.max(1));
        while conn.input.len - conn.at < count {
            let kind = more(conn, until);
            if kind != 0 {
                return kind;
            }
        }
        give(conn, count);
        0
    }
}

/// Reads a line: up to a line feed, which is not part of it, nor is a carriage return
/// before it. A line longer than `most` bytes is a failure, and so is one that is not
/// text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_net_read_line(number: i64, most: i64, nanos: i64) -> i64 {
    unsafe {
        let conn = match conn_of(number) {
            Ok(conn) => conn,
            Err(kind) => return kind,
        };
        let most = (most.max(1) as usize).min(MOST_READ);
        let until = koo_now_nanos().saturating_add(nanos.max(1));
        let mut looked = 0;
        let end = loop {
            let have = core::slice::from_raw_parts(conn.input.ptr.add(conn.at), conn.input.len - conn.at);
            if let Some(end) = have[looked..].iter().position(|byte| *byte == b'\n') {
                break looked + end;
            }
            looked = have.len();
            // (One byte more than the line may be is a carriage return, perhaps.)
            if looked > most + 1 {
                conn.broken = true;
                return failed(TOO_LONG, &[b"a line was longer than the longest that was asked for"]);
            }
            let kind = more(conn, until);
            if kind != 0 {
                return kind;
            }
        };
        let line = core::slice::from_raw_parts(conn.input.ptr.add(conn.at), end);
        let text = line.strip_suffix(b"\r").unwrap_or(line);
        if text.len() > most {
            conn.broken = true;
            return failed(TOO_LONG, &[b"a line was longer than the longest that was asked for"]);
        }
        if core::str::from_utf8(text).is_err() {
            conn.broken = true;
            return failed(
                NOT_TEXT,
                &[b"a line was not text: it is read as bytes, with `read_exact` or `read`"],
            );
        }
        let ending = line.len() - text.len();
        give(conn, end + 1);
        // The line, without its ending.
        conn.taken.len -= 1 + ending;
        0
    }
}

/// What the last read gave, as bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_net_taken(number: i64) -> *mut Header {
    unsafe {
        let data: &[u8] = match conn_of(number) {
            Ok(conn) => conn.taken.bytes(),
            Err(_) => &[],
        };
        let list = koo_list_new(data.len() as i64, 1);
        (*list).len = data.len() as i64;
        core::ptr::copy_nonoverlapping(data.as_ptr(), (list as *mut u8).add(HEADER), data.len());
        list
    }
}

/// What the last read gave, as text: after `koo_net_read_line`, which has seen that it is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_net_taken_text(number: i64) -> *mut Header {
    unsafe {
        let data: &[u8] = match conn_of(number) {
            Ok(conn) => conn.taken.bytes(),
            Err(_) => &[],
        };
        koo_str_from_bytes(data.as_ptr(), data.len() as i64)
    }
}

/// When the program ends: the connections are closed, and what they held is given back.
#[allow(static_mut_refs)]
pub unsafe fn at_exit() {
    unsafe {
        if OWNER.load(Acquire) != getpid() {
            return;
        }
        for conn in CONNS.iter_mut() {
            if conn.fd >= 0 {
                close(conn.fd);
            }
            conn.input.release();
            conn.taken.release();
            *conn = NO_CONN;
        }
        for pool in POOLS.iter_mut() {
            pool.address.release();
            *pool = NO_POOL;
        }
    }
}
