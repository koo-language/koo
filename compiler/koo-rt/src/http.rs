//! The server of `std/http`.
//!
//! One thread, one event loop, as in Node: the loop accepts connections, reads requests
//! and writes responses without ever waiting on one client, and hands the program one
//! complete request at a time. `koo_http_next` runs the loop until a request is ready and
//! returns the number of the route it matches; the program answers with
//! `koo_http_respond` and asks for the next. Parsing and routing happen here, so a
//! request costs the program one call in and one call out.
//!
//! A server may run several of these loops, each in a process of its own, all accepting
//! on one socket (`workers`). A handler shares nothing with another request, so the
//! program cannot tell the difference, except that it uses every core. The process that
//! started the others replaces one that dies, so a handler that panics costs the
//! requests its worker was holding and nothing more.
//!
//! The loop waits with `kqueue` on macOS and `epoll` on Linux (`sys`, below). Everything
//! else is the same code on both.
//!
//! What a client sends is not trusted. A request's head is at most 16 KB with at most 64
//! headers, its body at most what the server was started with; a request that takes
//! longer than 30 seconds to arrive, or a connection idle that long, is closed; a body
//! announced in two ways is refused, so that no proxy in front can be made to disagree
//! with the server about where a request ends.

use super::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use imp::*;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub use stub::*;

/// On a system the server is not written for yet, listening fails and says so.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod stub {
    use super::*;

    unsafe fn none() -> *mut Header {
        unsafe { new_str(b"") }
    }
    pub unsafe fn answer_before_ending() {}
    pub unsafe fn landing() -> Option<(usize, usize)> {
        None
    }
    pub unsafe fn given_up() -> bool {
        false
    }
    pub unsafe fn leave_if_given_up() {}
    pub unsafe fn cut_short() -> bool {
        false
    }
    pub unsafe fn waits_elsewhere(_: usize) -> bool {
        false
    }
    pub unsafe fn await_turn(_: i64) -> bool {
        false
    }
    pub unsafe fn name_now() -> ([u8; 64], usize) {
        ([0; 64], 0)
    }
    pub unsafe fn give_turns() {}
    pub unsafe fn asking(_: bool) {}
    pub unsafe fn leave_if_out_of_grace() {}
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_os_stopping() -> bool {
        false
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_given_up() -> bool {
        false
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_request_id(_: i64) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_request_now() -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_metrics() -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_timeout(_: i64) {}
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_reset() {}
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_route(_: *const Header, _: *const Header) {}
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_listen(_: i64, _: i64, _: i64, _: i64) -> i64 {
        unsafe {
            failed_with(new_str(b"std/http is not built for this system yet"));
            thread::me().failed = false;
        }
        -1
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_next() -> i64 {
        -2
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_id() -> i64 {
        0
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_method(_: i64) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_path(_: i64) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_rest(_: i64) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_param(_: i64, _: *const Header) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_query_has(_: i64, _: *const Header) -> bool {
        false
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_query(_: i64, _: *const Header) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_form_has(_: i64, _: *const Header) -> bool {
        false
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_form(_: i64, _: *const Header) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_cookie_has(_: i64, _: *const Header) -> bool {
        false
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_cookie(_: i64, _: *const Header) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_header_has(_: i64, _: *const Header) -> bool {
        false
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_header(_: i64, _: *const Header) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_body(_: i64) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_body_raw(_: i64) -> *mut Header {
        unsafe { none() }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_respond(_: i64, _: i64, _: *const Header, _: *const Header, _: *const Header) {}
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_respond_file(
        _: i64,
        _: i64,
        _: *const Header,
        _: *const Header,
        _: *const Header,
        _: *const Header,
    ) {
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod imp {
    use super::*;
    use core::fmt::Write;

    unsafe extern "C" {
        fn socket(domain: i32, kind: i32, protocol: i32) -> i32;
        fn setsockopt(fd: i32, level: i32, name: i32, value: *const i32, len: u32) -> i32;
        fn bind(fd: i32, address: *const u8, len: u32) -> i32;
        fn getsockname(fd: i32, address: *mut u8, len: *mut u32) -> i32;
        fn listen(fd: i32, backlog: i32) -> i32;
        fn accept(fd: i32, address: *mut u8, len: *mut u32) -> i32;
        fn recv(fd: i32, data: *mut u8, len: usize, flags: i32) -> isize;
        fn fcntl(fd: i32, command: i32, ...) -> i32;
        fn close(fd: i32) -> i32;
        fn signal(number: i32, handler: usize) -> usize;
        fn fork() -> i32;
        fn getpid() -> i32;
        fn kill(process: i32, number: i32) -> i32;
        fn waitpid(process: i32, status: *mut i32, options: i32) -> i32;
        fn setrlimit(resource: i32, limit: *const [u64; 2]) -> i32;
        fn getrlimit(resource: i32, limit: *mut [u64; 2]) -> i32;
        fn opendir(path: *const u8) -> *mut u8;
        fn realpath(path: *const u8, resolved: *mut u8) -> *mut u8;
        fn strlen(text: *const u8) -> usize;
        fn closedir(directory: *mut u8) -> i32;
        fn fseek(file: *mut u8, offset: i64, whence: i32) -> i32;
        fn ftell(file: *mut u8) -> i64;
    }

    const AF_INET: i32 = 2;
    const SOCK_STREAM: i32 = 1;
    const IPPROTO_TCP: i32 = 6;
    const TCP_NODELAY: i32 = 1;
    const F_GETFL: i32 = 3;
    const F_SETFL: i32 = 4;
    const EINTR: i32 = 4;
    const MSG_PEEK: i32 = 2;
    const MSG_DONTWAIT: i32 = if cfg!(target_os = "macos") { 0x80 } else { 0x40 };
    const SIGINT: i32 = 2;
    const SIGPIPE: i32 = 13;
    const SIGTERM: i32 = 15;
    const WNOHANG: i32 = 1;

    /// What one wait of the loop found out about one thing it watches.
    #[derive(Clone, Copy)]
    pub(super) struct Event {
        /// What the thing was registered with: `LISTENER`, or a connection's tag.
        pub tag: usize,
        pub readable: bool,
        pub writable: bool,
        /// The other side is gone, whether or not anything was being waited for.
        pub hangup: bool,
        /// The process that started this one has ended.
        pub parent_gone: bool,
    }

    pub(super) const NO_EVENT: Event = Event {
        tag: 0,
        readable: false,
        writable: false,
        hangup: false,
        parent_gone: false,
    };

    pub(super) const LISTENER: usize = usize::MAX;

    /// What differs between systems: the numbers of the C library, and the queue that
    /// says which sockets are ready.
    #[cfg(target_os = "macos")]
    mod sys {
        use super::{Event, LISTENER};

        pub const SOL_SOCKET: i32 = 0xffff;
        pub const SO_REUSEADDR: i32 = 0x4;
        pub const O_NONBLOCK: i32 = 0x4;
        pub const EAGAIN: i32 = 35;
        pub const RLIMIT_NOFILE: i32 = 8;
        pub const CLOCK_MONOTONIC: i32 = 6;
        const SO_NOSIGPIPE: i32 = 0x1022;
        const EVFILT_READ: i16 = -1;
        const EVFILT_WRITE: i16 = -2;
        const EVFILT_PROC: i16 = -5;
        const EV_ADD: u16 = 0x1;
        const EV_DELETE: u16 = 0x2;
        const EV_ENABLE: u16 = 0x4;
        const EV_DISABLE: u16 = 0x8;
        const EV_ONESHOT: u16 = 0x10;
        const NOTE_EXIT: u32 = 0x8000_0000;
        const PARENT: usize = usize::MAX - 1;

        #[repr(C)]
        pub struct SockAddr {
            len: u8,
            family: u8,
            pub port: u16,
            address: u32,
            zero: [u8; 8],
        }

        pub const AF_INET6: i32 = 30;
        pub const IPPROTO_IPV6: i32 = 41;
        pub const IPV6_V6ONLY: i32 = 27;

        pub fn address(port: u16) -> SockAddr {
            SockAddr {
                len: 16,
                family: 2,
                port: port.to_be(),
                address: 0,
                zero: [0; 8],
            }
        }

        #[repr(C)]
        #[derive(Clone, Copy)]
        struct KEvent {
            ident: usize,
            filter: i16,
            flags: u16,
            fflags: u32,
            data: isize,
            udata: usize,
        }

        unsafe extern "C" {
            fn kqueue() -> i32;
            fn kevent(
                queue: i32,
                changes: *const KEvent,
                change_count: i32,
                events: *mut KEvent,
                event_count: i32,
                timeout: *const [i64; 2],
            ) -> i32;
            fn setsockopt(fd: i32, level: i32, name: i32, value: *const i32, len: u32) -> i32;
        }

        unsafe fn change(queue: i32, ident: usize, filter: i16, flags: u16, fflags: u32, udata: usize) {
            let change = KEvent {
                ident,
                filter,
                flags,
                fflags,
                data: 0,
                udata,
            };
            unsafe { kevent(queue, &change, 1, core::ptr::null_mut(), 0, core::ptr::null()) };
        }

        pub unsafe fn queue() -> i32 {
            unsafe { kqueue() }
        }

        pub unsafe fn watch_listener(queue: i32, fd: i32) {
            unsafe { change(queue, fd as usize, EVFILT_READ, EV_ADD, 0, LISTENER) };
        }

        pub unsafe fn listener_on(queue: i32, fd: i32, on: bool) {
            let flag = if on { EV_ENABLE } else { EV_DISABLE };
            unsafe { change(queue, fd as usize, EVFILT_READ, flag, 0, LISTENER) };
        }

        /// A socket just accepted: a write to it must not raise a signal, and it is read.
        pub unsafe fn watch(queue: i32, fd: i32, tag: usize) {
            let on = 1i32;
            unsafe {
                setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, 4);
                change(queue, fd as usize, EVFILT_READ, EV_ADD, 0, tag);
            }
        }

        /// A wait for the socket to take more output is over when it has been told of
        /// once, and must be asked for again if there is still output to write.
        pub const ONCE: bool = true;

        /// Waits for the socket to take more output, once.
        pub unsafe fn want_write(queue: i32, fd: i32, tag: usize, _reading: bool) {
            unsafe { change(queue, fd as usize, EVFILT_WRITE, EV_ADD | EV_ONESHOT, 0, tag) };
        }

        /// The output is written: nothing more is waited for on that side.
        pub unsafe fn wrote(_queue: i32, _fd: i32, _tag: usize, _reading: bool) {}

        pub unsafe fn stop_reading(queue: i32, fd: i32, _tag: usize, _writing: bool) {
            unsafe { change(queue, fd as usize, EVFILT_READ, EV_DELETE, 0, 0) };
        }

        pub unsafe fn start_reading(queue: i32, fd: i32, tag: usize, _writing: bool) {
            unsafe { change(queue, fd as usize, EVFILT_READ, EV_ADD, 0, tag) };
        }

        /// Waits, once, for a socket that is not a connection of the server's to have
        /// something to read, or room to write.
        pub unsafe fn watch_once(queue: i32, fd: i32, write: bool, tag: usize) {
            let filter = if write { EVFILT_WRITE } else { EVFILT_READ };
            unsafe { change(queue, fd as usize, filter, EV_ADD | EV_ONESHOT, 0, tag) };
        }

        /// Ends such a wait. One that was told of has ended by itself.
        pub unsafe fn unwatch(queue: i32, fd: i32, write: bool, told: bool) {
            let filter = if write { EVFILT_WRITE } else { EVFILT_READ };
            if !told {
                unsafe { change(queue, fd as usize, filter, EV_DELETE, 0, 0) };
            }
        }

        /// A worker ends when the process that started it does. Returns whether that
        /// process is still there.
        pub unsafe fn watch_parent(queue: i32, parent: i32) -> bool {
            unsafe { change(queue, parent as usize, EVFILT_PROC, EV_ADD, NOTE_EXIT, PARENT) };
            true
        }

        pub unsafe fn wait(queue: i32, events: &mut [Event; 64], milliseconds: i64) -> usize {
            let mut raw = [KEvent {
                ident: 0,
                filter: 0,
                flags: 0,
                fflags: 0,
                data: 0,
                udata: 0,
            }; 64];
            let timeout = [milliseconds / 1000, milliseconds % 1000 * 1_000_000];
            let count = unsafe { kevent(queue, core::ptr::null(), 0, raw.as_mut_ptr(), 64, &timeout) };
            let count = if count > 0 { count as usize } else { 0 };
            for (event, raw) in events.iter_mut().zip(&raw[..count]) {
                *event = Event {
                    tag: raw.udata,
                    readable: raw.filter == EVFILT_READ,
                    writable: raw.filter == EVFILT_WRITE,
                    hangup: false,
                    parent_gone: raw.udata == PARENT && raw.filter == EVFILT_PROC,
                };
            }
            count
        }
    }

    #[cfg(target_os = "linux")]
    mod sys {
        use super::{Event, LISTENER};

        pub const SOL_SOCKET: i32 = 1;
        pub const SO_REUSEADDR: i32 = 2;
        pub const O_NONBLOCK: i32 = 0x800;
        pub const EAGAIN: i32 = 11;
        pub const RLIMIT_NOFILE: i32 = 7;
        pub const CLOCK_MONOTONIC: i32 = 1;
        const EPOLLIN: u32 = 0x1;
        const EPOLLOUT: u32 = 0x4;
        const EPOLLERR: u32 = 0x8;
        const EPOLLHUP: u32 = 0x10;
        const EPOLLRDHUP: u32 = 0x2000;
        const EPOLLONESHOT: u32 = 1 << 30;
        /// Wakes one of the processes that wait on a socket, not all of them.
        const EPOLLEXCLUSIVE: u32 = 1 << 28;
        const ADD: i32 = 1;
        const DEL: i32 = 2;
        const MOD: i32 = 3;
        const PR_SET_PDEATHSIG: i32 = 1;
        const SIGTERM: u64 = 15;

        #[repr(C)]
        pub struct SockAddr {
            family: u16,
            pub port: u16,
            address: u32,
            zero: [u8; 8],
        }

        pub const AF_INET6: i32 = 10;
        pub const IPPROTO_IPV6: i32 = 41;
        pub const IPV6_V6ONLY: i32 = 26;

        pub fn address(port: u16) -> SockAddr {
            SockAddr {
                family: 2,
                port: port.to_be(),
                address: 0,
                zero: [0; 8],
            }
        }

        /// The kernel's `struct epoll_event`, which is packed on x86-64 and nowhere else.
        #[cfg_attr(target_arch = "x86_64", repr(C, packed))]
        #[cfg_attr(not(target_arch = "x86_64"), repr(C))]
        #[derive(Clone, Copy)]
        struct EpollEvent {
            events: u32,
            data: u64,
        }

        unsafe extern "C" {
            fn epoll_create1(flags: i32) -> i32;
            fn epoll_ctl(queue: i32, operation: i32, fd: i32, event: *mut EpollEvent) -> i32;
            fn epoll_wait(queue: i32, events: *mut EpollEvent, count: i32, milliseconds: i32) -> i32;
            fn prctl(option: i32, ...) -> i32;
            fn getppid() -> i32;
        }

        unsafe fn control(queue: i32, operation: i32, fd: i32, events: u32, tag: usize) {
            let mut event = EpollEvent {
                events,
                data: tag as u64,
            };
            unsafe { epoll_ctl(queue, operation, fd, &mut event) };
        }

        fn interest(reading: bool, writing: bool) -> u32 {
            (if reading { EPOLLIN | EPOLLRDHUP } else { 0 }) | (if writing { EPOLLOUT } else { 0 })
        }

        pub unsafe fn queue() -> i32 {
            unsafe { epoll_create1(0) }
        }

        pub unsafe fn watch_listener(queue: i32, fd: i32) {
            unsafe { control(queue, ADD, fd, EPOLLIN | EPOLLEXCLUSIVE, LISTENER) };
        }

        pub unsafe fn listener_on(queue: i32, fd: i32, on: bool) {
            // What is watched exclusively cannot be changed, only taken out and put back.
            unsafe {
                if on {
                    watch_listener(queue, fd);
                } else {
                    control(queue, DEL, fd, 0, LISTENER);
                }
            }
        }

        pub unsafe fn watch(queue: i32, fd: i32, tag: usize) {
            unsafe { control(queue, ADD, fd, interest(true, false), tag) };
        }

        /// A wait for the socket to take more output lasts until `wrote` ends it.
        pub const ONCE: bool = false;

        pub unsafe fn want_write(queue: i32, fd: i32, tag: usize, reading: bool) {
            unsafe { control(queue, MOD, fd, interest(reading, true), tag) };
        }

        pub unsafe fn wrote(queue: i32, fd: i32, tag: usize, reading: bool) {
            unsafe { control(queue, MOD, fd, interest(reading, false), tag) };
        }

        pub unsafe fn stop_reading(queue: i32, fd: i32, tag: usize, writing: bool) {
            unsafe { control(queue, MOD, fd, interest(false, writing), tag) };
        }

        pub unsafe fn start_reading(queue: i32, fd: i32, tag: usize, writing: bool) {
            unsafe { control(queue, MOD, fd, interest(true, writing), tag) };
        }

        /// Waits, once, for a socket that is not a connection of the server's to have
        /// something to read, or room to write.
        pub unsafe fn watch_once(queue: i32, fd: i32, write: bool, tag: usize) {
            let wanted = if write { EPOLLOUT } else { EPOLLIN | EPOLLRDHUP };
            unsafe { control(queue, ADD, fd, wanted | EPOLLONESHOT, tag) };
        }

        /// Ends such a wait, whether or not it was told of.
        pub unsafe fn unwatch(queue: i32, fd: i32, _write: bool, _told: bool) {
            unsafe { control(queue, DEL, fd, 0, 0) };
        }

        /// A worker is sent the signal to stop when the process that started it ends.
        /// Returns whether that process is still there.
        pub unsafe fn watch_parent(_queue: i32, parent: i32) -> bool {
            unsafe {
                prctl(PR_SET_PDEATHSIG, SIGTERM);
                getppid() == parent
            }
        }

        pub unsafe fn wait(queue: i32, events: &mut [Event; 64], milliseconds: i64) -> usize {
            let mut raw = [EpollEvent { events: 0, data: 0 }; 64];
            let count = unsafe { epoll_wait(queue, raw.as_mut_ptr(), 64, milliseconds as i32) };
            let count = if count > 0 { count as usize } else { 0 };
            for (event, raw) in events.iter_mut().zip(&raw[..count]) {
                let bits = raw.events;
                *event = Event {
                    tag: raw.data as usize,
                    readable: bits & (EPOLLIN | EPOLLRDHUP) != 0,
                    writable: bits & EPOLLOUT != 0,
                    hangup: bits & (EPOLLERR | EPOLLHUP) != 0,
                    parent_gone: false,
                };
            }
            count
        }
    }

    /// The most a request's head may be, and the most headers it may have.
    const MAX_HEAD: usize = 16 * 1024;
    const MAX_HEADERS: usize = 64;
    /// Output a client has not read beyond which no more of its requests are read.
    const MAX_PENDING: usize = 1024 * 1024;
    /// The largest file sent whole.
    const MAX_FILE: usize = 256 * 1024 * 1024;
    /// Seconds a connection may be idle, and a request may take to arrive.
    const TIMEOUT: i64 = 30;
    const MAX_PARAMS: usize = 16;
    const MAX_WORKERS: usize = 256;

    /// A growable array, on the C allocator: the runtime has no other.
    struct List<T> {
        ptr: *mut T,
        len: usize,
        cap: usize,
    }

    impl<T> List<T> {
        const fn new() -> List<T> {
            List {
                ptr: core::ptr::null_mut(),
                len: 0,
                cap: 0,
            }
        }

        unsafe fn reserve(&mut self, extra: usize) {
            let need = self.len + extra;
            if need <= self.cap {
                return;
            }
            let cap = need.max(self.cap * 2).max(16);
            let ptr = unsafe { realloc(self.ptr as *mut u8, cap * core::mem::size_of::<T>()) } as *mut T;
            if ptr.is_null() {
                fatal(b"koo: out of memory\n");
            }
            self.ptr = ptr;
            self.cap = cap;
        }

        unsafe fn push(&mut self, value: T) {
            unsafe {
                self.reserve(1);
                self.ptr.add(self.len).write(value);
            }
            self.len += 1;
        }

        fn items(&self) -> &[T] {
            if self.len == 0 {
                return &[];
            }
            unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
        }

        fn at(&mut self, index: usize) -> &mut T {
            unsafe { &mut *self.ptr.add(index) }
        }

        unsafe fn release(&mut self) {
            unsafe { free(self.ptr as *mut u8) };
            *self = List::new();
        }
    }

    impl List<u8> {
        unsafe fn extend(&mut self, bytes: &[u8]) {
            unsafe {
                self.reserve(bytes.len());
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.add(self.len), bytes.len());
            }
            self.len += bytes.len();
        }

        unsafe fn number(&mut self, mut value: usize) {
            let mut digits = [0u8; 20];
            let mut at = digits.len();
            loop {
                at -= 1;
                digits[at] = b'0' + (value % 10) as u8;
                value /= 10;
                if value == 0 {
                    break;
                }
            }
            unsafe { self.extend(&digits[at..]) };
        }
    }

    /// A stretch of bytes in a connection's input, or in the text of the routes.
    #[derive(Clone, Copy)]
    struct Range {
        start: usize,
        end: usize,
    }

    const EMPTY: Range = Range { start: 0, end: 0 };

    struct Conn {
        /// The socket, or -1 for a slot that is free.
        fd: i32,
        /// Counts the connections the slot has held, so that an event for an earlier
        /// one is not taken for the present one.
        generation: u32,
        input: List<u8>,
        /// Where the request being read begins in `input`.
        start: usize,
        /// How much of it has been searched for the end of its head.
        scanned: usize,
        /// For a body sent in chunks: where the next chunk begins, counted from `start`
        /// (0 before the first), and how much of the body has been put together.
        chunk_at: usize,
        decoded: usize,
        output: List<u8>,
        sent: usize,
        /// Whether the loop waits for the socket to have input, and to take output.
        reading: bool,
        writing: bool,
        /// Whether the connection ends once its output is written.
        closing: bool,
        /// Whether it waits in `READY`.
        queued: bool,
        told_continue: bool,
        /// Whether a request of it is with a handler, which may be waiting: nothing
        /// more of the connection is looked at until the request is answered.
        handling: bool,
        /// Whether reading was stopped meanwhile, to be started again after the answer.
        paused: bool,
        /// Whether it was closed while its request was with a handler, and is kept
        /// until the handler is done.
        left: bool,
        /// When it last read or wrote, and when the request being read began, in seconds.
        active: i64,
        began: i64,
    }

    struct Route {
        method: Range,
        pattern: Range,
    }

    /// The request the program is answering.
    #[derive(Clone, Copy)]
    struct Current {
        id: i64,
        answered: bool,
        conn: usize,
        /// Which of the connections that have had that slot it is.
        generation: u32,
        method: Range,
        path: Range,
        query: Range,
        headers: [(Range, Range); MAX_HEADERS],
        header_count: usize,
        body: Range,
        /// The bytes of the whole request.
        length: usize,
        head_only: bool,
        keep: bool,
        /// Whether the request is of HTTP/1.0, whose clients are told when a connection
        /// is kept.
        old: bool,
        /// Each parameter of the route: its name in the route's text, its value in the path.
        params: [(Range, Range); MAX_PARAMS],
        param_count: usize,
        /// What a `*` at the end of the route stands for.
        rest: Range,
        /// When the request was complete, for the log.
        at: i64,
        /// The name the request goes by, in the log and to whoever made it: what its
        /// `X-Request-Id` said, or one made here. See `named`.
        name: [u8; MOST_NAME],
        name_len: usize,
    }

    /// The longest name of a request that is taken from the one who sends it.
    const MOST_NAME: usize = 64;

    /// What a server counts of itself, for whoever watches it (`koo_http_metrics`).
    /// The counts are in memory that the process which watches shares with its workers,
    /// made before they are started, so that each adds to the same numbers and any of
    /// them can say what all have done; a worker that is replaced takes nothing with it.
    #[repr(C)]
    struct Counts {
        /// Requests answered, by the hundred of the status: 1xx to 5xx, and the rest.
        answered: [AtomicU64; 6],
        /// Of those, the ones whose client had left before the answer (logged as 499).
        left: AtomicU64,
        /// How long answers took: how many within each of `WITHIN`, and all together,
        /// in microseconds.
        within: [AtomicU64; WITHIN.len()],
        took: AtomicU64,
        timed: AtomicU64,
        /// Handlers that panicked, and workers started (the first ones among them).
        panics: AtomicU64,
        workers: AtomicU64,
    }

    /// The limits of the histogram of how long answers take, in microseconds.
    const WITHIN: [u64; 11] = [
        1_000, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000, 2_500_000, 10_000_000,
    ];

    use core::sync::atomic::AtomicU64;
    static COUNTS: core::sync::atomic::AtomicPtr<Counts> = core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

    /// The counts, once there are any: from the first time a server listens.
    fn counts() -> Option<&'static Counts> {
        unsafe { COUNTS.load(core::sync::atomic::Ordering::Acquire).as_ref() }
    }

    /// Counts one answer: its status, and how long it took if that is known.
    unsafe fn count(status: i64, since: i64) {
        let Some(counts) = counts() else { return };
        use core::sync::atomic::Ordering::Relaxed;
        let hundred = match status / 100 {
            hundred @ 1..=5 => hundred as usize - 1,
            _ => 5,
        };
        counts.answered[hundred].fetch_add(1, Relaxed);
        if status == 499 {
            counts.left.fetch_add(1, Relaxed);
        }
        if since != 0 {
            let micros = ((nanos() - since) / 1000).max(0) as u64;
            for (at, limit) in WITHIN.iter().enumerate() {
                if micros <= *limit {
                    counts.within[at].fetch_add(1, Relaxed);
                }
            }
            counts.took.fetch_add(micros, Relaxed);
            counts.timed.fetch_add(1, Relaxed);
        }
    }

    /// What the server has counted, as the text that Prometheus and what speaks as it
    /// does read: a handler answers a scrape with it, `http.text(http.metrics())`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_metrics() -> *mut Header {
        unsafe {
            use core::sync::atomic::Ordering::Relaxed;
            let mut out = crate::pg::Buf::new();
            let mut line = |text: core::fmt::Arguments| {
                let mut one = Buffer::new();
                let _ = one.write_fmt(text);
                out.push(&one.data[..one.len]);
            };
            if let Some(counts) = counts() {
                line(format_args!(
                    "# HELP koo_http_requests_total Requests answered, by the class of their status.\n"
                ));
                line(format_args!("# TYPE koo_http_requests_total counter\n"));
                for (at, class) in ["1xx", "2xx", "3xx", "4xx", "5xx", "other"].iter().enumerate() {
                    let count = counts.answered[at].load(Relaxed);
                    line(format_args!("koo_http_requests_total{{status=\"{class}\"}} {count}\n"));
                }
                line(format_args!(
                    "# HELP koo_http_requests_abandoned_total Requests whose client left before the answer.\n"
                ));
                line(format_args!("# TYPE koo_http_requests_abandoned_total counter\n"));
                line(format_args!(
                    "koo_http_requests_abandoned_total {}\n",
                    counts.left.load(Relaxed)
                ));
                line(format_args!(
                    "# HELP koo_http_request_duration_seconds How long answers took, from the whole request to the answer.\n"
                ));
                line(format_args!("# TYPE koo_http_request_duration_seconds histogram\n"));
                for (at, limit) in WITHIN.iter().enumerate() {
                    let (whole, part) = (limit / 1_000_000, limit % 1_000_000 / 1000);
                    let count = counts.within[at].load(Relaxed);
                    line(format_args!(
                        "koo_http_request_duration_seconds_bucket{{le=\"{whole}.{part:03}\"}} {count}\n"
                    ));
                }
                let (timed, took) = (counts.timed.load(Relaxed), counts.took.load(Relaxed));
                line(format_args!(
                    "koo_http_request_duration_seconds_bucket{{le=\"+Inf\"}} {timed}\n"
                ));
                line(format_args!(
                    "koo_http_request_duration_seconds_sum {}.{:06}\n",
                    took / 1_000_000,
                    took % 1_000_000
                ));
                line(format_args!("koo_http_request_duration_seconds_count {timed}\n"));
                line(format_args!(
                    "# HELP koo_http_handler_panics_total Handlers that panicked.\n"
                ));
                line(format_args!("# TYPE koo_http_handler_panics_total counter\n"));
                line(format_args!(
                    "koo_http_handler_panics_total {}\n",
                    counts.panics.load(Relaxed)
                ));
                line(format_args!(
                    "# HELP koo_http_workers_started_total Worker processes started, the first ones and those that replaced others.\n"
                ));
                line(format_args!("# TYPE koo_http_workers_started_total counter\n"));
                line(format_args!(
                    "koo_http_workers_started_total {}\n",
                    counts.workers.load(Relaxed)
                ));
            }
            let text = new_str(out.bytes());
            out.release();
            text
        }
    }

    static mut LISTEN: i32 = -1;
    static mut QUEUE: i32 = -1;
    /// How many workers the server is to have, and when one that could not be started
    /// is tried again.
    static mut WORKERS_WANTED: usize = 0;
    static mut START_AT: i64 = 0;
    static mut ACCEPTING: bool = false;
    /// Why it is not, when it is not for want of being told to stop: the system had no
    /// socket for another connection, or the worker has nearly all the handlers it can.
    static mut NO_SOCKETS: bool = false;
    static mut CROWDED: bool = false;
    /// The most connections a process has open at a time: what the system allows it in
    /// files, less what its handlers are left for themselves.
    static mut MOST_OPEN: usize = usize::MAX;
    static mut MAX_BODY: usize = 1024 * 1024;
    static mut LOG: bool = false;
    /// How long a handler is given to answer a request, in nanoseconds, or 0 for as
    /// long as it takes.
    static mut REQUEST_TIME: i64 = 30_000_000_000;
    static mut CONNS: List<Conn> = List::new();
    static mut FREE: List<u32> = List::new();
    static mut READY: List<u32> = List::new();
    /// How far into `READY` the connections have been taken up: they are taken in the
    /// order in which they became ready, so that under more load than the worker has
    /// room for, the one that has waited longest is the next, and none waits for ever.
    static mut READY_AT: usize = 0;
    static mut ROUTES: List<Route> = List::new();
    static mut ROUTE_TEXT: List<u8> = List::new();
    static mut SCRATCH: List<u8> = List::new();
    /// The methods a path has routes for, when the request's is not one of them.
    static mut ALLOWED: List<u8> = List::new();
    static mut EVENTS: [Event; 64] = [NO_EVENT; 64];
    static mut EVENT_COUNT: usize = 0;
    static mut EVENT_AT: usize = 0;
    /// Set where a signal is handled, on whichever thread the signal finds, and read by
    /// the loops: so it is a value that threads may share.
    static STOP: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    static mut CHILD: bool = false;
    /// Whether this process only watches over the workers, and answers nothing itself.
    static mut SUPERVISOR: bool = false;
    static mut PARENT_ID: i32 = 0;
    static mut CHILDREN: [i32; MAX_WORKERS] = [0; MAX_WORKERS];
    static mut CHILD_COUNT: usize = 0;
    static mut SWEPT: i64 = 0;
    static mut NEXT_ID: i64 = 0;
    static mut CURRENT: Current = NO_CURRENT;

    const NO_CURRENT: Current = Current {
        id: 0,
        answered: true,
        conn: 0,
        generation: 0,
        method: EMPTY,
        path: EMPTY,
        query: EMPTY,
        headers: [(EMPTY, EMPTY); MAX_HEADERS],
        header_count: 0,
        body: EMPTY,
        length: 0,
        head_only: false,
        keep: true,
        old: false,
        params: [(EMPTY, EMPTY); MAX_PARAMS],
        param_count: 0,
        rest: EMPTY,
        at: 0,
        name: [0; MOST_NAME],
        name_len: 0,
    };
    static mut DATE: [u8; 29] = [0; 29];
    static mut DATE_AT: i64 = -1;

    extern "C" fn stop(_: i32) {
        STOP.store(true, core::sync::atomic::Ordering::Relaxed);
    }

    /// Whether a program that is no server has asked to be told when it is to stop.
    static WATCHING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

    /// `os.stopping()`: whether the program has been told to stop, by the signal that
    /// systems send for the purpose or by Ctrl-C. A program that is a server is told so
    /// by `serve` returning. One that is not, a program that takes work from a queue,
    /// is ended by the signal in the middle of whatever it is doing, unless it asks:
    /// from the first time it asks, the signal ends nothing, and is what this says.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_os_stopping() -> bool {
        unsafe {
            use core::sync::atomic::Ordering::{AcqRel, Relaxed};
            // (Not where a server has its own way with the signal already: there this
            // says what the server was told.)
            if LISTEN < 0 && !WATCHING.swap(true, AcqRel) {
                signal(SIGINT, stop as extern "C" fn(i32) as usize);
                signal(SIGTERM, stop as extern "C" fn(i32) as usize);
            }
            STOP.load(Relaxed)
        }
    }

    fn now() -> i64 {
        let mut time = [0i64; 2];
        unsafe { clock_gettime(0, &mut time) };
        time[0]
    }

    fn nanos() -> i64 {
        let mut time = [0i64; 2];
        unsafe { clock_gettime(sys::CLOCK_MONOTONIC, &mut time) };
        time[0] * 1_000_000_000 + time[1]
    }

    unsafe fn error() -> i32 {
        unsafe { *errno_location() }
    }

    #[allow(static_mut_refs)]
    unsafe fn conn<'a>(index: usize) -> &'a mut Conn {
        unsafe { CONNS.at(index) }
    }

    fn tag(index: usize, generation: u32) -> usize {
        ((generation as usize) << 32) | index
    }

    // ---- the date -----------------------------------------------------------------------

    /// `Sun, 06 Nov 1994 08:49:37 GMT`, worked out once a second.
    #[allow(static_mut_refs)]
    unsafe fn date() -> &'static [u8] {
        let seconds = now();
        unsafe {
            if seconds != DATE_AT {
                DATE_AT = seconds;
                let days = seconds.div_euclid(86400);
                let time = seconds.rem_euclid(86400);
                // The civil date of a day number, by Howard Hinnant's algorithm.
                let z = days + 719468;
                let era = z.div_euclid(146097);
                let doe = z.rem_euclid(146097);
                let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
                let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
                let mp = (5 * doy + 2) / 153;
                let day = doy - (153 * mp + 2) / 5 + 1;
                let month = if mp < 10 { mp + 3 } else { mp - 9 };
                let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
                let weekday = (days + 4).rem_euclid(7) as usize;
                let two = |out: &mut [u8], value: i64| {
                    out[0] = b'0' + (value / 10 % 10) as u8;
                    out[1] = b'0' + (value % 10) as u8;
                };
                DATE[0..3].copy_from_slice(&b"SunMonTueWedThuFriSat"[weekday * 3..weekday * 3 + 3]);
                DATE[3..5].copy_from_slice(b", ");
                two(&mut DATE[5..7], day);
                DATE[7] = b' ';
                let m = (month - 1) as usize * 3;
                DATE[8..11].copy_from_slice(&b"JanFebMarAprMayJunJulAugSepOctNovDec"[m..m + 3]);
                DATE[11] = b' ';
                two(&mut DATE[12..14], year / 100);
                two(&mut DATE[14..16], year % 100);
                DATE[16] = b' ';
                two(&mut DATE[17..19], time / 3600);
                DATE[19] = b':';
                two(&mut DATE[20..22], time / 60 % 60);
                DATE[22] = b':';
                two(&mut DATE[23..25], time % 60);
                DATE[25..29].copy_from_slice(b" GMT");
            }
            &DATE
        }
    }

    fn reason(status: i64) -> &'static [u8] {
        match status {
            200 => b"OK",
            201 => b"Created",
            202 => b"Accepted",
            204 => b"No Content",
            301 => b"Moved Permanently",
            302 => b"Found",
            303 => b"See Other",
            304 => b"Not Modified",
            307 => b"Temporary Redirect",
            308 => b"Permanent Redirect",
            400 => b"Bad Request",
            401 => b"Unauthorized",
            403 => b"Forbidden",
            404 => b"Not Found",
            405 => b"Method Not Allowed",
            408 => b"Request Timeout",
            409 => b"Conflict",
            410 => b"Gone",
            413 => b"Content Too Large",
            415 => b"Unsupported Media Type",
            422 => b"Unprocessable Content",
            429 => b"Too Many Requests",
            431 => b"Request Header Fields Too Large",
            500 => b"Internal Server Error",
            501 => b"Not Implemented",
            502 => b"Bad Gateway",
            503 => b"Service Unavailable",
            504 => b"Gateway Timeout",
            505 => b"HTTP Version Not Supported",
            _ => b"Status",
        }
    }

    /// The type of a file, by the end of its name.
    fn media_type(path: &[u8]) -> &'static [u8] {
        let dot = path.iter().rposition(|b| *b == b'.').map_or(path.len(), |i| i + 1);
        let mut lower = [0u8; 8];
        let ending = &path[dot..];
        if ending.len() > lower.len() {
            return b"application/octet-stream";
        }
        for (to, from) in lower.iter_mut().zip(ending) {
            *to = from.to_ascii_lowercase();
        }
        match &lower[..ending.len()] {
            b"html" | b"htm" => b"text/html; charset=utf-8",
            b"css" => b"text/css; charset=utf-8",
            b"js" | b"mjs" => b"text/javascript; charset=utf-8",
            b"json" | b"map" => b"application/json",
            b"txt" | b"md" => b"text/plain; charset=utf-8",
            b"csv" => b"text/csv; charset=utf-8",
            b"xml" => b"application/xml",
            b"svg" => b"image/svg+xml",
            b"png" => b"image/png",
            b"jpg" | b"jpeg" => b"image/jpeg",
            b"gif" => b"image/gif",
            b"webp" => b"image/webp",
            b"ico" => b"image/x-icon",
            b"woff" => b"font/woff",
            b"woff2" => b"font/woff2",
            b"pdf" => b"application/pdf",
            b"wasm" => b"application/wasm",
            b"mp4" => b"video/mp4",
            b"mp3" => b"audio/mpeg",
            b"zip" => b"application/zip",
            _ => b"application/octet-stream",
        }
    }

    // ---- connections --------------------------------------------------------------------

    #[allow(static_mut_refs)]
    unsafe fn shut(index: usize) {
        let c = unsafe { conn(index) };
        if c.fd < 0 {
            return;
        }
        unsafe {
            // Closing the socket takes it out of the queue.
            close(c.fd);
            // There is a socket to be had again for another connection.
            NO_SOCKETS = false;
        }
        c.fd = -1;
        if c.handling {
            // Its request is with a handler, which may yet read of the request: its
            // path, its headers, its body are where the connection kept them. They
            // stay, and the connection's place in the table with them, until the
            // handler is done (`answering`). And the handler is told, however it was
            // that the connection came to be closed: there is nobody to answer.
            c.left = true;
            unsafe { give_up(index) };
            return;
        }
        unsafe { let_go(index) };
    }

    /// Gives back what a connection that is closed still held, and its place in the table.
    #[allow(static_mut_refs)]
    unsafe fn let_go(index: usize) {
        let c = unsafe { conn(index) };
        unsafe {
            c.input.release();
            c.output.release();
        }
        c.left = false;
        c.handling = false;
        // (Within thirty bits: what the queue is given for a connection has its
        // generation in the high half, where the words for a handler's wait, for the
        // listener and for the parent are numbers that no connection's may be.)
        c.generation = c.generation.wrapping_add(1) & 0x3fff_ffff;
        unsafe { FREE.push(index as u32) };
    }

    /// Takes new connections or leaves them to the other workers, as things stand. A
    /// worker leaves them when it is told to stop, when it has no socket for another,
    /// and when its handlers are nearly as many as it has places for: the system gives
    /// a new connection to whichever worker is looking, and is as ready to give them all
    /// to one, whose requests would then wait for a place while its neighbours had
    /// nothing to do. It takes them again when a good few of its handlers are done.
    #[allow(static_mut_refs)]
    unsafe fn take_or_leave() {
        unsafe {
            let handlers = PLACES[..PLACE_COUNT]
                .iter()
                .filter(|place| matches!(place.doing, Doing::Handling | Doing::Waiting | Doing::Ready))
                .count();
            let open = CONNS.len - FREE.len;
            if CROWDED {
                // (It takes them again a little below the most it may have open: a
                // quarter below where files are so few that eight would be all of them.)
                CROWDED =
                    handlers > MOST_PLACES - 40 || open + (MOST_OPEN / 4).min(8) > MOST_OPEN || !a_place_to_spare();
            } else {
                CROWDED = handlers >= MOST_PLACES - 24 || open >= MOST_OPEN || !a_place_to_spare();
            }
            let wanted = !STOP.load(core::sync::atomic::Ordering::Relaxed) && !NO_SOCKETS && !CROWDED;
            if wanted != ACCEPTING && LISTEN >= 0 {
                ACCEPTING = wanted;
                sys::listener_on(QUEUE, LISTEN, wanted);
            }
        }
    }

    #[allow(static_mut_refs)]
    unsafe fn take_connections() {
        // So many at a time and no more, and none past the most there may be: the rest
        // are still waiting when the requests on these have been looked at, and it is
        // known by then whether this worker has room for them or its neighbours are to
        // have them.
        for _ in 0..16 {
            if unsafe { CONNS.len - FREE.len >= MOST_OPEN } {
                return;
            }
            let fd = unsafe { accept(LISTEN, core::ptr::null_mut(), core::ptr::null_mut()) };
            if fd < 0 {
                let code = unsafe { error() };
                if code == EINTR {
                    continue;
                }
                if code != sys::EAGAIN {
                    // No more sockets to be had: stop asking until one is given back.
                    unsafe {
                        if CONNS.items().iter().any(|c| c.fd >= 0) {
                            NO_SOCKETS = true;
                            take_or_leave();
                        }
                    }
                }
                return;
            }
            let on = 1i32;
            unsafe {
                fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | sys::O_NONBLOCK);
                setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &on, 4);
            }
            let fresh = Conn {
                fd,
                generation: 0,
                input: List::new(),
                start: 0,
                scanned: 0,
                chunk_at: 0,
                decoded: 0,
                output: List::new(),
                sent: 0,
                reading: true,
                writing: false,
                closing: false,
                queued: false,
                told_continue: false,
                handling: false,
                paused: false,
                left: false,
                active: now(),
                began: 0,
            };
            let index = unsafe {
                if FREE.len > 0 {
                    FREE.len -= 1;
                    let index = *FREE.ptr.add(FREE.len) as usize;
                    let generation = conn(index).generation;
                    *conn(index) = Conn { generation, ..fresh };
                    index
                } else {
                    CONNS.push(fresh);
                    CONNS.len - 1
                }
            };
            unsafe { sys::watch(QUEUE, fd, tag(index, conn(index).generation)) };
        }
    }

    #[allow(static_mut_refs)]
    unsafe fn ready(index: usize) {
        let c = unsafe { conn(index) };
        if !c.queued {
            c.queued = true;
            unsafe { READY.push(index as u32) };
        }
    }

    #[allow(static_mut_refs)]
    unsafe fn read_from(index: usize) {
        let c = unsafe { conn(index) };
        // The most that is read of a request before it is looked at: a body in chunks
        // takes more room than it holds.
        let most = MAX_HEAD + unsafe { MAX_BODY } * 2 + 8192;
        loop {
            unsafe { c.input.reserve(8192) };
            let room = c.input.cap - c.input.len;
            let n = unsafe { read(c.fd, c.input.ptr.add(c.input.len), room) };
            if n > 0 {
                if c.began == 0 {
                    c.began = now();
                }
                c.active = now();
                c.input.len += n as usize;
                // A client that sends without end is not read without end.
                if (n as usize) < room || c.input.len - c.start > most {
                    break;
                }
            } else if n == 0 {
                // The client has finished sending. A request it sent whole is still
                // answered, and then the connection ends.
                if c.input.len == c.start {
                    unsafe { shut(index) };
                    return;
                }
                c.closing = true;
                c.reading = false;
                unsafe { sys::stop_reading(QUEUE, c.fd, tag(index, c.generation), c.writing) };
                break;
            } else {
                let code = unsafe { error() };
                if code == EINTR {
                    continue;
                }
                if code == sys::EAGAIN {
                    break;
                }
                unsafe { shut(index) };
                return;
            }
        }
        unsafe { ready(index) };
    }

    /// Writes what a connection is owed, as far as its socket takes it.
    #[allow(static_mut_refs)]
    unsafe fn flush(index: usize) {
        let c = unsafe { conn(index) };
        if c.fd < 0 {
            return;
        }
        while c.sent < c.output.len {
            let n = unsafe { write(c.fd, c.output.ptr.add(c.sent), c.output.len - c.sent) };
            if n > 0 {
                c.sent += n as usize;
                c.active = now();
                continue;
            }
            let code = unsafe { error() };
            if code == EINTR {
                continue;
            }
            if code == sys::EAGAIN {
                if !c.writing {
                    c.writing = true;
                    unsafe { sys::want_write(QUEUE, c.fd, tag(index, c.generation), c.reading) };
                }
                return;
            }
            unsafe { shut(index) };
            return;
        }
        c.output.len = 0;
        c.sent = 0;
        if c.writing {
            c.writing = false;
            unsafe { sys::wrote(QUEUE, c.fd, tag(index, c.generation), c.reading) };
        }
        if c.closing {
            unsafe { shut(index) };
        } else if c.input.len > c.start {
            // Requests that waited for the client to read its answers.
            unsafe { ready(index) };
        }
    }

    /// The head of a response, queued on a connection. Returns whether a body follows.
    #[allow(static_mut_refs)]
    unsafe fn head(
        index: usize,
        status: i64,
        content_type: &[u8],
        extra: &[u8],
        length: usize,
        head_only: bool,
        keep: bool,
    ) -> bool {
        let c = unsafe { conn(index) };
        let bodiless = status < 200 || status == 204 || status == 304;
        let out = &mut c.output;
        unsafe {
            out.reserve(256 + extra.len() + length);
            out.extend(b"HTTP/1.1 ");
            out.number(status as usize);
            out.extend(b" ");
            out.extend(reason(status));
            out.extend(b"\r\n");
            if !content_type.is_empty() && !bodiless {
                out.extend(b"Content-Type: ");
                // A type is one line, whatever it was made of: an end of line in it
                // would begin a header that the handler never meant to send.
                for piece in content_type.split(|byte| *byte == b'\r' || *byte == b'\n') {
                    out.extend(piece);
                }
                out.extend(b"\r\n");
            }
            out.extend(extra);
            if !bodiless {
                out.extend(b"Content-Length: ");
                out.number(length);
                out.extend(b"\r\n");
            }
            out.extend(b"Date: ");
            out.extend(date());
            out.extend(b"\r\n");
            // The name of the request, for whoever made it to quote; unless the
            // handler has said one of its own.
            let said = extra
                .split(|byte| *byte == b'\n')
                .any(|line| line.len() > 13 && line[..13].eq_ignore_ascii_case(b"x-request-id:"));
            if CURRENT.name_len > 0 && CURRENT.conn == index && !said {
                out.extend(b"X-Request-Id: ");
                out.extend(&CURRENT.name[..CURRENT.name_len]);
                out.extend(b"\r\n");
            }
            if !keep {
                out.extend(b"Connection: close\r\n");
            } else if CURRENT.old {
                out.extend(b"Connection: keep-alive\r\n");
            }
            out.extend(b"\r\n");
        }
        if !keep {
            c.closing = true;
        }
        !bodiless && !head_only
    }

    /// Queues a response on a connection, and writes what the socket takes of it.
    unsafe fn send(
        index: usize,
        status: i64,
        content_type: &[u8],
        extra: &[u8],
        body: &[u8],
        head_only: bool,
        keep: bool,
    ) {
        unsafe {
            if conn(index).fd < 0 {
                return;
            }
            if head(index, status, content_type, extra, body.len(), head_only, keep) {
                conn(index).output.extend(body);
            }
            flush(index);
        }
    }

    /// A request is done with: what follows it in the input is the next.
    unsafe fn pass(index: usize, length: usize) {
        let c = unsafe { conn(index) };
        c.start += length;
        c.scanned = 0;
        c.chunk_at = 0;
        c.decoded = 0;
        c.began = 0;
        c.told_continue = false;
        if c.start >= c.input.len {
            c.input.len = 0;
            c.start = 0;
        }
    }

    /// Refuses what a connection sent, and ends it.
    unsafe fn refuse(index: usize, status: i64) {
        let mut body = [0u8; 64];
        let text = reason(status);
        body[..text.len()].copy_from_slice(text);
        body[text.len()] = b'\n';
        unsafe {
            let c = conn(index);
            let left = c.input.len - c.start;
            pass(index, left);
            log(status, text.len() + 1, false);
            send(
                index,
                status,
                b"text/plain; charset=utf-8",
                b"",
                &body[..text.len() + 1],
                false,
                false,
            );
        }
    }

    /// Gives a request the name it goes by: the one it came with, in `X-Request-Id`, so
    /// that what several services write of one piece of work can be found together;
    /// or, if it came with none, or with one that is not fit to be written into a log
    /// and a header as it is (longer than 64 characters, or of other characters than
    /// letters, digits and `.`, `_`, `:`, `-`), sixteen digits of chance.
    unsafe fn named(current: &mut Current, data: &[u8]) {
        let given = current.headers[..current.header_count]
            .iter()
            .find(|(name, _)| data[name.start..name.end].eq_ignore_ascii_case(b"x-request-id"))
            .map(|(_, value)| &data[value.start..value.end])
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= MOST_NAME
                    && value
                        .iter()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(byte))
            });
        match given {
            Some(value) => {
                current.name[..value.len()].copy_from_slice(value);
                current.name_len = value.len();
            }
            None => {
                let mut chance = [0u8; 8];
                unsafe { crate::codec::random(&mut chance) };
                for (at, byte) in chance.iter().enumerate() {
                    current.name[2 * at] = b"0123456789abcdef"[(byte >> 4) as usize];
                    current.name[2 * at + 1] = b"0123456789abcdef"[(byte & 15) as usize];
                }
                current.name_len = 16;
            }
        }
    }

    /// One line of the log: the request, the status of its answer, and how long the
    /// answer took. `known` says whether the request was read far enough to name.
    #[allow(static_mut_refs)]
    unsafe fn log(status: i64, size: usize, known: bool) {
        unsafe {
            count(status, if known { CURRENT.at } else { 0 });
            if !LOG {
                return;
            }
            use super::super::log::{Field, said};
            let mut out = Buffer::new();
            if known {
                let c = conn(CURRENT.conn);
                let data = core::slice::from_raw_parts(c.input.ptr, c.input.len);
                let text = |range: Range| core::str::from_utf8(&data[range.start..range.end]).unwrap_or("?");
                let (method, path) = (text(CURRENT.method), text(CURRENT.path));
                let shown = path.char_indices().nth(200).map_or(path.len(), |(at, _)| at);
                let _ = write!(out, "{method} {} {status}", &path[..shown]);
                let message = out.len;
                let micros = (nanos() - CURRENT.at) / 1000;
                let mut took = Buffer::new();
                let _ = write!(took, "{}.{:03}", micros / 1000, micros % 1000);
                let _ = writeln!(
                    out,
                    " {size}B {}ms",
                    core::str::from_utf8(&took.data[..took.len]).unwrap_or("0")
                );
                let fields = [
                    (&b"method"[..], Field::Text(method.as_bytes())),
                    (&b"path"[..], Field::Text(&path.as_bytes()[..shown])),
                    (&b"status"[..], Field::Number(status)),
                    (&b"bytes"[..], Field::Number(size as i64)),
                    (&b"ms"[..], Field::Digits(&took.data[..took.len])),
                    (&b"request"[..], Field::Text(&CURRENT.name[..CURRENT.name_len])),
                ];
                said(b"info", &out.data[..out.len], &out.data[..message], &fields);
            } else {
                let _ = write!(out, "- - {status}");
                let message = out.len;
                let _ = writeln!(out, " {size}B");
                let fields = [
                    (&b"status"[..], Field::Number(status)),
                    (&b"bytes"[..], Field::Number(size as i64)),
                ];
                said(b"info", &out.data[..out.len], &out.data[..message], &fields);
            }
        }
    }

    // ---- requests -----------------------------------------------------------------------

    enum Parsed {
        /// More of the request is to come.
        Partial,
        /// A request is in `CURRENT`, for the route of this number, or for none.
        Request(i64),
        /// The connection was answered here, or is gone.
        Handled,
    }

    fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        if haystack.len() < needle.len() {
            return None;
        }
        (from..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
    }

    fn token(bytes: &[u8]) -> bool {
        !bytes.is_empty()
            && bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(b))
    }

    fn trim(bytes: &[u8]) -> (usize, usize) {
        let start = bytes
            .iter()
            .position(|b| *b != b' ' && *b != b'\t')
            .unwrap_or(bytes.len());
        let end = bytes
            .iter()
            .rposition(|b| *b != b' ' && *b != b'\t')
            .map_or(start, |i| i + 1);
        (start, end)
    }

    /// How a body sent in chunks stands.
    enum Chunks {
        Partial,
        /// The body is whole: the request is this long.
        Whole(usize),
        Bad(i64),
    }

    /// Puts together what has arrived of a body sent in chunks. Each chunk is a length
    /// in hexadecimal on a line, then that many bytes, then a line end; a chunk of no
    /// length ends the body. The bytes are moved down as they arrive, so that the body
    /// lies in one piece where it began.
    #[allow(static_mut_refs)]
    unsafe fn chunks(index: usize, body_at: usize) -> Chunks {
        let c = unsafe { conn(index) };
        let max = unsafe { MAX_BODY };
        if c.chunk_at == 0 {
            c.chunk_at = body_at;
            c.decoded = 0;
        }
        loop {
            // The input is looked at afresh each time round: the bytes of a chunk move.
            let data = unsafe { core::slice::from_raw_parts(c.input.ptr.add(c.start), c.input.len - c.start) };
            let Some(line_end) = find(data, b"\r\n", c.chunk_at) else {
                return if data.len() - c.chunk_at > 64 {
                    Chunks::Bad(400)
                } else {
                    Chunks::Partial
                };
            };
            let line = &data[c.chunk_at..line_end];
            let digits = &line[..line.iter().position(|b| *b == b';').unwrap_or(line.len())];
            let (from, to) = trim(digits);
            let digits = &digits[from..to];
            if digits.is_empty() || digits.len() > 8 || !digits.iter().all(u8::is_ascii_hexdigit) {
                return Chunks::Bad(400);
            }
            let size = digits
                .iter()
                .fold(0usize, |n, b| n * 16 + (*b as char).to_digit(16).unwrap_or(0) as usize);
            if size == 0 {
                // Whatever follows the last chunk, up to an empty line, is passed over.
                let mut at = line_end + 2;
                loop {
                    let Some(end) = find(data, b"\r\n", at) else {
                        return if data.len() - at > MAX_HEAD {
                            Chunks::Bad(431)
                        } else {
                            Chunks::Partial
                        };
                    };
                    if end == at {
                        return Chunks::Whole(end + 2);
                    }
                    at = end + 2;
                }
            }
            if c.decoded + size > max {
                return Chunks::Bad(413);
            }
            let from = line_end + 2;
            if data.len() < from + size + 2 {
                return Chunks::Partial;
            }
            if &data[from + size..from + size + 2] != b"\r\n" {
                return Chunks::Bad(400);
            }
            unsafe {
                let base = c.input.ptr.add(c.start);
                core::ptr::copy(base.add(from), base.add(body_at + c.decoded), size);
            }
            c.decoded += size;
            c.chunk_at = from + size + 2;
        }
    }

    /// Reads the request at the start of a connection's input, if it is all there.
    #[allow(static_mut_refs)]
    unsafe fn parse(index: usize) -> Parsed {
        let c = unsafe { conn(index) };
        let all = unsafe { core::slice::from_raw_parts(c.input.ptr, c.input.len) };
        // Line ends between requests are let pass.
        while c.start < all.len() && c.chunk_at == 0 && (all[c.start] == b'\r' || all[c.start] == b'\n') {
            c.start += 1;
        }
        let base = c.start;
        let data = &all[base..];
        if data.is_empty() {
            unsafe { pass(index, 0) };
            return Parsed::Partial;
        }
        let Some(end) = find(data, b"\r\n\r\n", c.scanned.saturating_sub(3)) else {
            c.scanned = data.len();
            if data.len() > MAX_HEAD {
                unsafe { refuse(index, 431) };
                return Parsed::Handled;
            }
            return Parsed::Partial;
        };
        if end > MAX_HEAD {
            unsafe { refuse(index, 431) };
            return Parsed::Handled;
        }
        // The end of the head is found again at once while the body arrives.
        c.scanned = end + 3;
        let head = &data[..end];
        let line_end = find(head, b"\r\n", 0).unwrap_or(head.len());
        let line = &head[..line_end];
        let mut parts = line.split(|b| *b == b' ');
        let (Some(method), Some(target), Some(version), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            unsafe { refuse(index, 400) };
            return Parsed::Handled;
        };
        if !token(method)
            || method.len() > 16
            || target.first() != Some(&b'/')
            || target.iter().any(|b| *b < 0x21 || *b == 0x7f)
        {
            unsafe { refuse(index, 400) };
            return Parsed::Handled;
        }
        let old = match version {
            b"HTTP/1.1" => false,
            b"HTTP/1.0" => true,
            _ => {
                unsafe { refuse(index, if version.starts_with(b"HTTP/") { 505 } else { 400 }) };
                return Parsed::Handled;
            }
        };
        let current = unsafe { &mut CURRENT };
        current.header_count = 0;
        // (Nameless until it is read whole: what is answered to half a request, or to
        // one that is no request, has not the name of the one before it.)
        current.name_len = 0;
        let mut length: Option<usize> = None;
        let mut chunked = false;
        let mut keep = !old;
        let mut expects = false;
        let mut at = line_end + 2;
        while at < head.len() {
            let stop = find(head, b"\r\n", at).unwrap_or(head.len());
            let field = &head[at..stop];
            let Some(colon) = field.iter().position(|b| *b == b':') else {
                unsafe { refuse(index, 400) };
                return Parsed::Handled;
            };
            let name = &field[..colon];
            let (from, to) = trim(&field[colon + 1..]);
            let value = &field[colon + 1 + from..colon + 1 + to];
            // A name is a token: no space before the colon, and no line that goes on.
            if !token(name) || value.iter().any(|b| (*b < 0x20 && *b != b'\t') || *b == 0x7f) {
                unsafe { refuse(index, 400) };
                return Parsed::Handled;
            }
            if current.header_count == MAX_HEADERS {
                unsafe { refuse(index, 431) };
                return Parsed::Handled;
            }
            let name_at = base + at;
            let value_at = base + at + colon + 1 + from;
            current.headers[current.header_count] = (
                Range {
                    start: name_at,
                    end: name_at + name.len(),
                },
                Range {
                    start: value_at,
                    end: value_at + value.len(),
                },
            );
            current.header_count += 1;
            if name.eq_ignore_ascii_case(b"content-length") {
                // Digits, and the same number if it is said twice.
                let digits = !value.is_empty() && value.len() <= 12 && value.iter().all(u8::is_ascii_digit);
                let number = value.iter().fold(0usize, |n, b| n * 10 + (*b - b'0') as usize);
                if !digits || length.is_some_and(|before| before != number) {
                    unsafe { refuse(index, 400) };
                    return Parsed::Handled;
                }
                length = Some(number);
            } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
                // In chunks, and in no other way, which would not be read rightly.
                if chunked || !value.eq_ignore_ascii_case(b"chunked") || old {
                    unsafe { refuse(index, 501) };
                    return Parsed::Handled;
                }
                chunked = true;
            } else if name.eq_ignore_ascii_case(b"connection") {
                for word in value.split(|b| *b == b',') {
                    let (from, to) = trim(word);
                    if word[from..to].eq_ignore_ascii_case(b"close") {
                        keep = false;
                    } else if word[from..to].eq_ignore_ascii_case(b"keep-alive") {
                        keep = true;
                    }
                }
            } else if name.eq_ignore_ascii_case(b"expect") {
                expects = value.eq_ignore_ascii_case(b"100-continue");
            }
            at = stop + 2;
        }
        // A body whose length is said in two ways is where a proxy and a server come to
        // disagree about where the next request begins.
        if chunked && length.is_some() {
            unsafe { refuse(index, 400) };
            return Parsed::Handled;
        }
        let body_at = end + 4;
        let (body_len, total) = if chunked {
            match unsafe { chunks(index, body_at) } {
                Chunks::Whole(total) => (unsafe { conn(index) }.decoded, Some(total)),
                Chunks::Partial => (0, None),
                Chunks::Bad(status) => {
                    unsafe { refuse(index, status) };
                    return Parsed::Handled;
                }
            }
        } else {
            let body_len = length.unwrap_or(0);
            if body_len > unsafe { MAX_BODY } {
                unsafe { refuse(index, 413) };
                return Parsed::Handled;
            }
            (
                body_len,
                (data.len() >= body_at + body_len).then_some(body_at + body_len),
            )
        };
        let Some(total) = total else {
            let c = unsafe { conn(index) };
            if c.input.len - c.start > MAX_HEAD + unsafe { MAX_BODY } * 2 + 4096 {
                unsafe { refuse(index, 413) };
                return Parsed::Handled;
            }
            if expects && !c.told_continue {
                c.told_continue = true;
                unsafe {
                    c.output.extend(b"HTTP/1.1 100 Continue\r\n\r\n");
                    flush(index);
                }
            }
            return Parsed::Partial;
        };
        let question = target.iter().position(|b| *b == b'?');
        let path_len = question.unwrap_or(target.len());
        let target_at = base + method.len() + 1;
        current.conn = index;
        current.answered = false;
        current.method = Range {
            start: base,
            end: base + method.len(),
        };
        current.path = Range {
            start: target_at,
            end: target_at + path_len,
        };
        current.query = match question {
            Some(q) => Range {
                start: target_at + q + 1,
                end: target_at + target.len(),
            },
            None => EMPTY,
        };
        current.body = Range {
            start: base + body_at,
            end: base + body_at + body_len,
        };
        current.length = total;
        current.head_only = method == b"HEAD";
        current.keep = keep;
        current.old = old;
        current.at = nanos();
        unsafe {
            NEXT_ID += 1;
            current.id = NEXT_ID;
            current.generation = c.generation;
            c.handling = true;
            named(current, core::slice::from_raw_parts(c.input.ptr, c.input.len));
        }
        Parsed::Request(unsafe { route(method, &target[..path_len], target_at) })
    }

    /// Whether a path fits the pattern of a route. A segment `:name` of a pattern fits
    /// any one segment of a path, and `*` at the end fits whatever is left. What the
    /// parameters and the `*` stand for is left in `CURRENT`.
    #[allow(static_mut_refs)]
    unsafe fn fits(pattern: &[u8], pattern_at: usize, path: &[u8], path_at: usize) -> bool {
        let current = unsafe { &mut CURRENT };
        let pattern = match pattern {
            [rest @ .., b'/'] if !rest.is_empty() => rest,
            other => other,
        };
        current.param_count = 0;
        current.rest = EMPTY;
        let (mut p, mut q) = (0, 0);
        loop {
            if p == pattern.len() || q == path.len() {
                // `/files/*` fits `/files` as well, with nothing left over.
                let open = pattern[p..] == *b"/*";
                return (p == pattern.len() || open) && q == path.len();
            }
            if pattern[p] != b'/' || path[q] != b'/' {
                return false;
            }
            p += 1;
            q += 1;
            let segment = pattern[p..]
                .iter()
                .position(|b| *b == b'/')
                .map_or(pattern.len(), |i| p + i);
            let piece = path[q..].iter().position(|b| *b == b'/').map_or(path.len(), |i| q + i);
            if pattern[p..segment] == *b"*" && segment == pattern.len() {
                current.rest = Range {
                    start: path_at + q,
                    end: path_at + path.len(),
                };
                return true;
            }
            if pattern.get(p) == Some(&b':') {
                if piece == q || current.param_count == MAX_PARAMS {
                    return false;
                }
                current.params[current.param_count] = (
                    Range {
                        start: pattern_at + p + 1,
                        end: pattern_at + segment,
                    },
                    Range {
                        start: path_at + q,
                        end: path_at + piece,
                    },
                );
                current.param_count += 1;
            } else if pattern[p..segment] != path[q..piece] {
                return false;
            }
            p = segment;
            q = piece;
        }
    }

    /// The first route a request matches, or -1. If the path has routes, but none for
    /// the method, the methods it has are left in `ALLOWED`.
    #[allow(static_mut_refs)]
    unsafe fn route(method: &[u8], path: &[u8], path_at: usize) -> i64 {
        let method = if method == b"HEAD" { b"GET" as &[u8] } else { method };
        let text = unsafe { ROUTE_TEXT.items() };
        // One `/` at the end of a path is not told from none.
        let path = match path {
            [rest @ .., b'/'] if !rest.is_empty() => rest,
            other => other,
        };
        unsafe { ALLOWED.len = 0 };
        for (number, route) in unsafe { ROUTES.items() }.iter().enumerate() {
            let wanted = &text[route.method.start..route.method.end];
            let pattern = &text[route.pattern.start..route.pattern.end];
            if !unsafe { fits(pattern, route.pattern.start, path, path_at) } {
                continue;
            }
            if wanted == b"*" || wanted == method {
                return number as i64;
            }
            unsafe {
                if find(ALLOWED.items(), wanted, 0).is_none() {
                    if ALLOWED.len > 0 {
                        ALLOWED.extend(b", ");
                    }
                    ALLOWED.extend(wanted);
                }
            }
        }
        let current = unsafe { &mut CURRENT };
        current.param_count = 0;
        current.rest = EMPTY;
        -1
    }

    /// Answers a request that no route is for: 404, or 405 with the methods the path has.
    #[allow(static_mut_refs)]
    unsafe fn no_route(index: usize) {
        unsafe {
            let (head_only, keep) = (CURRENT.head_only, CURRENT.keep && !conn(index).closing);
            let (status, body): (i64, &[u8]) = if ALLOWED.len > 0 {
                (405, b"Method Not Allowed\n")
            } else {
                (404, b"Not Found\n")
            };
            SCRATCH.len = 0;
            if status == 405 {
                SCRATCH.extend(b"Allow: ");
                SCRATCH.extend(ALLOWED.items());
                SCRATCH.extend(b"\r\n");
            }
            log(status, body.len(), true);
            CURRENT.answered = true;
            // No handler has this request: the connection goes on to its next.
            conn(index).handling = false;
            pass(index, CURRENT.length);
            send(
                index,
                status,
                b"text/plain; charset=utf-8",
                SCRATCH.items(),
                body,
                head_only,
                keep,
            );
            if conn(index).fd >= 0 && conn(index).input.len > conn(index).start {
                ready(index);
            }
        }
    }

    /// Once a second: closes what has been idle too long and what is taking too long to
    /// arrive.
    #[allow(static_mut_refs)]
    unsafe fn sweep() {
        let seconds = now();
        unsafe {
            if seconds == SWEPT {
                return;
            }
            SWEPT = seconds;
            super::super::actor::let_go_of_old();
            for index in 0..CONNS.len {
                let c = conn(index);
                let busy = c.handling || (!CURRENT.answered && CURRENT.conn == index);
                if c.fd >= 0 && !busy && (seconds - c.active > TIMEOUT || (c.began != 0 && seconds - c.began > TIMEOUT))
                {
                    // One whose request has waited all that time for a handler to be
                    // free is told that the server has no room, and not left to guess
                    // from a connection that closes with nothing said.
                    if c.queued && c.output.len == c.sent {
                        refuse(index, 503);
                    } else {
                        shut(index);
                    }
                }
            }
        }
    }

    /// What the process that started the workers does while they serve: it replaces one
    /// that dies, so that a handler that panics takes down its own worker and no more.
    /// Returns 0 when the server has been told to stop, 1 in a process newly made to be
    /// a worker, and 2 when a worker has a message for an actor of this process, which
    /// is where the server's actors live.
    #[allow(static_mut_refs)]
    unsafe fn supervise() -> i32 {
        unsafe {
            loop {
                if STOP.load(core::sync::atomic::Ordering::Relaxed) {
                    // The workers are told to stop, and answer the requests they have
                    // in hand before they end. A handler may ask an actor meanwhile,
                    // and the actors live here: so they are answered until the last
                    // worker has gone. To end the workers and wait for them without
                    // answering would be to wait for ever on a worker that waits for
                    // an answer. A worker that has not ended in thirty seconds is ended.
                    if STOPPED_AT == 0 {
                        STOPPED_AT = nanos();
                        for &child in &CHILDREN[..CHILD_COUNT] {
                            kill(child, SIGTERM);
                        }
                    }
                    loop {
                        let mut status = 0;
                        let gone = waitpid(-1, &mut status, WNOHANG);
                        if gone <= 0 {
                            break;
                        }
                        if let Some(slot) = CHILDREN[..CHILD_COUNT].iter().position(|child| *child == gone) {
                            CHILD_COUNT -= 1;
                            CHILDREN[slot] = CHILDREN[CHILD_COUNT];
                        }
                    }
                    if CHILD_COUNT > 0 && nanos() - STOPPED_AT > 30_000_000_000 {
                        for &child in &CHILDREN[..CHILD_COUNT] {
                            kill(child, 9);
                            let mut status = 0;
                            waitpid(child, &mut status, 0);
                        }
                        CHILD_COUNT = 0;
                    }
                    if CHILD_COUNT == 0 {
                        finish();
                        return 0;
                    }
                    if super::super::actor::pending(50) {
                        return 2;
                    }
                    continue;
                }
                // (While it is found out which workers have gone, no thread of this
                // process takes the room they leave.)
                thread::RESERVE.store(1 + WORKERS_WANTED, core::sync::atomic::Ordering::Relaxed);
                loop {
                    let mut status = 0;
                    let gone = waitpid(-1, &mut status, WNOHANG);
                    if gone <= 0 {
                        break;
                    }
                    if let Some(slot) = CHILDREN[..CHILD_COUNT].iter().position(|child| *child == gone) {
                        CHILD_COUNT -= 1;
                        CHILDREN[slot] = CHILDREN[CHILD_COUNT];
                    }
                }
                // A worker that has gone has another started in its place: now, or, if
                // the system has no process to give this moment, a second from now and
                // every second after, until the server has the workers it was to have.
                while CHILD_COUNT < WORKERS_WANTED && nanos() >= START_AT {
                    // What is waiting to be printed is printed here, not by the worker too.
                    koo_flush();
                    // The threads that answer for the actors go on meanwhile, but leave
                    // their connections as they are until the new process has its own.
                    super::super::actor::forking(true);
                    let child = fork();
                    if child == 0 {
                        become_worker();
                        return 1;
                    }
                    if child > 0
                        && let Some(counts) = counts()
                    {
                        counts.workers.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                    super::super::actor::forking(false);
                    if child > 0 {
                        CHILDREN[CHILD_COUNT] = child;
                        CHILD_COUNT += 1;
                    } else {
                        START_AT = nanos() + 1_000_000_000;
                    }
                }
                // Room is left for one worker to be started in another's place, and
                // for each that is still to be started.
                thread::RESERVE.store(1 + WORKERS_WANTED - CHILD_COUNT, core::sync::atomic::Ordering::Relaxed);
                // The wait ends early when the signal to stop arrives.
                if super::super::actor::pending(100) {
                    return 2;
                }
            }
        }
    }

    /// What a process does first after it is forked to serve: it lets go of what it
    /// inherited, and waits on the listening socket with a queue of its own.
    #[allow(static_mut_refs)]
    unsafe fn become_worker() {
        unsafe {
            CHILD = true;
            SUPERVISOR = false;
            CHILD_COUNT = 0;
            PANICS = 0;
            STOPPED_AT = 0;
            // The stack that a signal is taken on, when the thread's own is exhausted, is
            // not the child's by inheritance everywhere: without one of its own a
            // handler that calls itself without end would end the worker without a word.
            thread::watch_stack();
            // The actors are the parent's too, and are reached through its socket.
            super::super::actor::after_fork();
            // The connections are the parent's, which still holds them.
            for index in 0..CONNS.len {
                let c = conn(index);
                if c.fd >= 0 {
                    close(c.fd);
                    c.fd = -1;
                }
                // (One that was closed while a handler had its request holds its
                // input still.)
                c.input.release();
                c.output.release();
            }
            CONNS.len = 0;
            FREE.len = 0;
            READY.len = 0;
            READY_AT = 0;
            CURRENT.answered = true;
            EVENT_COUNT = 0;
            EVENT_AT = 0;
            if QUEUE >= 0 {
                close(QUEUE);
            }
            start_queue();
            if !sys::watch_parent(QUEUE, PARENT_ID) {
                STOP.store(true, core::sync::atomic::Ordering::Relaxed);
            }
            // A worker that takes the place of one that died has the program's loop
            // from the process that made it.
            begin_places();
        }
    }

    /// The thread that answers requests in this process.
    static mut SERVING: usize = 0;
    /// The thread that runs handlers side by side, or 0 while none does.
    static SERVER: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

    /// In a process that is about to end because the stack of a handler ran out: the
    /// request the handler was answering is told, in a word, that the server failed.
    /// This is called where a signal is handled, so it allocates nothing and writes
    /// straight to the connection.
    #[allow(static_mut_refs)]
    pub unsafe fn answer_before_ending() {
        unsafe {
            if SERVING == 0 || SERVING != thread::this_thread() || CONNS.len == 0 || CURRENT.answered {
                return;
            }
            let index = CURRENT.conn;
            if index >= CONNS.len {
                return;
            }
            let c = conn(index);
            if c.fd >= 0 && c.handling && c.generation == CURRENT.generation {
                let said: &[u8] = b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain; charset=utf-8\r\n\
                                    Content-Length: 22\r\nConnection: close\r\n\r\nInternal Server Error\n";
                crate::write_all(c.fd, said);
            }
        }
    }

    /// Whether a handler of this process has run out of stack. The request was answered
    /// like any that panics, but the process is not to be trusted with more than the
    /// requests it has in hand: what was cut short may have been in the middle of
    /// anything.
    static mut OUT_OF_STACK: bool = false;

    /// A stack to stand on for the moment between a stack that is exhausted and the
    /// place the handler's panic comes back to.
    const LANDING: usize = 64 * 1024;
    static mut LANDING_STACK: [u8; LANDING] = [0; LANDING];

    unsafe extern "C" {
        fn alarm(seconds: u32) -> u32;
    }

    /// Where a handler that has run out of stack is to go on, and on what stack: the
    /// function below, on the stack kept for it. Nothing, if the thread that ran out
    /// is not answering a request under a guard, and so has nowhere to come back to.
    #[allow(static_mut_refs)]
    pub unsafe fn landing() -> Option<(usize, usize)> {
        unsafe {
            if SERVING == 0 || SERVING != thread::this_thread() || CONNS.len == 0 || CURRENT.answered {
                return None;
            }
            let index = CURRENT.conn;
            if index >= CONNS.len || !conn(index).handling || thread::me().guard.is_null() {
                return None;
            }
            let top = (LANDING_STACK.as_ptr() as usize + LANDING) & !15;
            Some((landed as extern "C" fn() -> ! as usize, top))
        }
    }

    /// Where the thread goes on after a handler has run out of stack: the panic has
    /// been said, and what is left is what any panic does, which is to come back to
    /// where the handler was called, to answer 500 and go on. The process then ends as
    /// soon as it has answered what it has in hand, and in ten seconds whatever
    /// happens: the handler may have been stopped in the middle of something that the
    /// rest of the process needs.
    extern "C" fn landed() -> ! {
        unsafe {
            OUT_OF_STACK = true;
            alarm(10);
            crate::actor::leave()
        }
    }

    unsafe fn start_queue() {
        unsafe {
            SERVING = thread::this_thread();
            QUEUE = sys::queue();
            ACCEPTING = true;
            sys::watch_listener(QUEUE, LISTEN);
        }
    }

    #[allow(static_mut_refs)]
    unsafe fn event(e: Event) {
        unsafe {
            if e.parent_gone {
                STOP.store(true, core::sync::atomic::Ordering::Relaxed);
                return;
            }
            if e.tag == LISTENER {
                take_connections();
                return;
            }
            if e.tag & !0xffff_ffff == PLACE {
                // What a handler waited for has come: if it is this wait of the
                // handler's that the word is of. The events of one look at the queue
                // are gone through while handlers run between them, and one that was
                // woken another way meanwhile, by its time or by its client's leaving,
                // may be waiting for something else by now, or only for a time.
                let (place, serial) = (e.tag & 0xff, (e.tag >> 8) as u32 & 0xff_ffff);
                if place < PLACE_COUNT
                    && PLACES[place].doing == Doing::Waiting
                    && PLACES[place].fd >= 0
                    && PLACES[place].serial == serial
                {
                    PLACES[place].doing = Doing::Ready;
                }
                return;
            }
            let index = e.tag & 0xffff_ffff;
            if index >= CONNS.len {
                return;
            }
            let generation = (e.tag >> 32) as u32;
            let live = |index: usize| conn(index).fd >= 0 && conn(index).generation == generation;
            if !live(index) {
                return;
            }
            if e.writable {
                // Where the wait was for once, it is over, and `flush` asks again if
                // the socket still does not take everything. Without this an answer
                // too long for two writes was never finished.
                if sys::ONCE {
                    conn(index).writing = false;
                }
                flush(index);
                if !live(index) {
                    return;
                }
            }
            if e.readable && conn(index).reading && conn(index).handling {
                // Its request is with a handler that waits. Either the client has sent
                // more, which stays where it is until the answer has gone; or it has
                // left, and the handler has nobody to answer.
                let c = conn(index);
                let mut byte = [0u8; 1];
                let there = recv(c.fd, byte.as_mut_ptr(), 1, MSG_PEEK | MSG_DONTWAIT);
                let gone = there == 0 || (there < 0 && error() != sys::EAGAIN && error() != EINTR);
                // (Unless an answer to an earlier request of the connection is still
                // going out: a client that has said it will send no more is reading
                // that one yet, and the connection ends when it has gone.)
                if gone && c.output.len == c.sent {
                    shut(index);
                } else {
                    c.reading = false;
                    c.paused = true;
                    sys::stop_reading(QUEUE, c.fd, tag(index, c.generation), c.writing);
                }
            } else if e.readable && conn(index).reading {
                read_from(index);
            } else if e.hangup {
                shut(index);
            }
        }
    }

    /// The connection of a request that is with a handler has been closed: the handler,
    /// which waits somewhere, is told that its request was given up, and woken to find
    /// it out. (A client that only says it will send no more, and waits for its answer,
    /// cannot be told from one that has gone, and is taken for one: so do the servers
    /// that are most used.)
    #[allow(static_mut_refs)]
    unsafe fn give_up(index: usize) {
        unsafe {
            if !SIDE_BY_SIDE {
                return;
            }
            let generation = conn(index).generation;
            for place in PLACES[..PLACE_COUNT].iter_mut() {
                let its = matches!(place.doing, Doing::Waiting | Doing::Ready)
                    && !place.current.answered
                    && place.current.conn == index
                    && place.current.generation == generation;
                if its {
                    place.given_up = true;
                    if place.doing == Doing::Waiting {
                        place.late = true;
                        place.doing = Doing::Ready;
                    }
                }
            }
        }
    }

    /// Whether the request the calling thread is answering was given up, so that what
    /// its handler does next is for nobody. It is so only where a handler may be left
    /// for another (`may_leave`): on the server's own thread, in a handler, with no
    /// actor held, since a method of an actor that has begun is never cut short.
    #[allow(static_mut_refs)]
    pub unsafe fn given_up() -> bool {
        unsafe {
            if !may_leave() {
                return false;
            }
            // A request is given up, too, when its handler has had the time that a
            // request is given: found out here, where the handler would wait or begin
            // something, and by the loop, for a handler that is waiting already.
            let place = &mut PLACES[HERE];
            if !place.given_up && place.deadline != 0 && nanos() >= place.deadline {
                place.given_up = true;
                place.timed_out = true;
            }
            place.given_up
        }
    }

    /// Sets how long a handler is given to answer a request: `serve`'s `timeout:`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_timeout(nanos: i64) {
        unsafe { REQUEST_TIME = nanos.max(0) };
    }

    /// Ends the handler of a request that was given up, as a panic would and without a
    /// word. What the handler held is not given back, as after a panic, so this is for
    /// where there is nothing else to do: an ask of an actor in another process, which
    /// has no way to fail. Called only where a panic might be, with nothing of the
    /// support library's in the middle of being used.
    pub unsafe fn leave_if_given_up() {
        unsafe {
            if given_up() && !thread::me().guard.is_null() {
                crate::actor::leave();
            }
        }
    }

    /// Asked where a handler is about to begin something that waits: a sleep, a
    /// statement, a request to another server. If its request was given up it begins
    /// nothing, and the caller says so in its own way, as it would say that the time
    /// had run out; the handler then runs on to its end, which with nothing to wait
    /// for comes soon, and gives back what it held as any handler does. One that goes
    /// on beginning things all the same, in a loop that tries again, is ended.
    #[allow(static_mut_refs)]
    pub unsafe fn cut_short() -> bool {
        unsafe {
            if !given_up() {
                return false;
            }
            PLACES[HERE].cut += 1;
            if PLACES[HERE].cut > MOST_CUT {
                leave_if_given_up();
            }
            // What it would have waited for, it does not wait for; but the others
            // have their turn here as they would have had it, or a handler that works
            // between its waits would keep the thread, now that it has no waits, until
            // it was done.
            give_way();
            true
        }
    }

    /// Lets the loop go round once, for the other handlers and for what has arrived,
    /// and comes back.
    #[allow(static_mut_refs)]
    unsafe fn give_way() {
        unsafe {
            if !may_leave() {
                return;
            }
            let Some(next) = another_place() else { return };
            let here = HERE;
            PLACES[here].serial = (PLACES[here].serial + 1) & 0xff_ffff;
            PLACES[here].fd = -1;
            // A wait for a time that has come.
            PLACES[here].until = nanos();
            PLACES[here].late = false;
            PLACES[here].doing = Doing::Waiting;
            go_to(next);
            PLACES[here].doing = Doing::Handling;
        }
    }

    /// How long a handler whose request was given up is still let wait for the answer
    /// of an actor it asks, in nanoseconds.
    const ASK_GRACE: i64 = 1_000_000_000;

    /// Said by a handler that begins to ask an actor in another process, and when it
    /// is done. An ask has no way to fail, so a handler that was not let wait for the
    /// answer could only be ended there, as a panic ends one, leaving behind what it
    /// held; and most answers are a moment away. So a handler whose request was given
    /// up is let wait a second more for the answers of what it asks, in all, and then
    /// comes to its end by returning, as any handler does. Only one whose actor takes
    /// longer than that is ended.
    #[allow(static_mut_refs)]
    pub unsafe fn asking(on: bool) {
        unsafe {
            if SERVER.load(core::sync::atomic::Ordering::Acquire) == thread::this_thread() && SIDE_BY_SIDE {
                PLACES[HERE].asking = on;
            }
        }
    }

    /// Whether the handler's request was given up and the second it had for the
    /// answers of actors is over. The second begins when this is first asked.
    #[allow(static_mut_refs)]
    unsafe fn out_of_grace() -> bool {
        unsafe {
            if !given_up() {
                return false;
            }
            let place = &mut PLACES[HERE];
            if place.grace == 0 {
                place.grace = nanos() + ASK_GRACE;
            }
            nanos() >= place.grace
        }
    }

    /// Ends the handler, as `leave_if_given_up` does, if that second is over.
    pub unsafe fn leave_if_out_of_grace() {
        unsafe {
            if out_of_grace() && !thread::me().guard.is_null() {
                PLACES[HERE].asking = false;
                crate::actor::leave();
            }
        }
    }

    /// Whether what holds something is a handler of this very thread that is waiting
    /// somewhere, while the thread itself cannot be left: asked by a thread that would
    /// wait for the thing to be given back, by an actor's method that runs in a
    /// server of one process, say. It would wait for ever, or for as long as it is
    /// given: the handler that has the thing goes on only when this thread does.
    #[allow(static_mut_refs)]
    pub unsafe fn waits_elsewhere(holder: usize) -> bool {
        unsafe {
            SERVER.load(core::sync::atomic::Ordering::Acquire) == thread::this_thread()
                && SIDE_BY_SIDE
                && !may_leave()
                && PLACES[..PLACE_COUNT]
                    .iter()
                    .enumerate()
                    .any(|(place, at)| place != HERE && at.thread as usize == holder && at.doing != Doing::Driving)
        }
    }

    /// How many things that wait a handler whose request was given up may begin. A
    /// loop that looks for something a hundred times, with a sleep between, ends by
    /// itself well inside it, giving back what it held; one that would never end is
    /// ended, which is better than a thread that turns without end.
    const MOST_CUT: u32 = 10_000;

    /// Whether the request being answered was given up, for the program's own loop,
    /// which then keeps what the handler failed with out of the log.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_given_up() -> bool {
        // (What has been found already, and nothing found here: a handler that was
        // never cut short and fails after its time is over fails with what it fails
        // with, in the log and to its client.)
        unsafe { may_leave() && PLACES[HERE].given_up }
    }

    #[allow(static_mut_refs)]
    unsafe fn finish() {
        unsafe {
            // A server of one process that lost a handler to an exhausted stack has
            // answered what it had in hand, and ends as a program that panicked does,
            // for whatever runs it to start it again.
            if OUT_OF_STACK && !CHILD && !SUPERVISOR {
                koo_flush();
                crate::actor::remove_socket();
                crate::koo_exit(101);
            }
            end_places();
            if !CHILD {
                // The program goes on without workers to leave room for.
                WORKERS_WANTED = 0;
                thread::RESERVE.store(0, core::sync::atomic::Ordering::Relaxed);
                for &child in &CHILDREN[..CHILD_COUNT] {
                    kill(child, SIGTERM);
                }
                for &child in &CHILDREN[..CHILD_COUNT] {
                    let mut status = 0;
                    waitpid(child, &mut status, 0);
                }
            }
            CHILD_COUNT = 0;
            for index in 0..CONNS.len {
                let c = conn(index);
                if c.fd >= 0 {
                    close(c.fd);
                    c.fd = -1;
                }
                // (One that was closed while a handler had its request holds its
                // input still.)
                c.input.release();
                c.output.release();
            }
            CONNS.release();
            FREE.release();
            READY.release();
            READY_AT = 0;
            SCRATCH.release();
            ALLOWED.release();
            if LISTEN >= 0 {
                close(LISTEN);
                LISTEN = -1;
            }
            if QUEUE >= 0 {
                close(QUEUE);
                QUEUE = -1;
            }
            CURRENT.answered = true;
            EVENT_COUNT = 0;
            EVENT_AT = 0;
            if CHILD {
                koo_flush();
                exit(0);
            }
            // A program that has asked whether it is to stop goes on hearing the
            // signal after its server has ended, as it did before: the signal ends
            // nothing for it (`koo_os_stopping`). For any other, the signal is again
            // what ends the program. And what the server was told is not forgotten:
            // the program was told to stop, and `main`, which goes on, can ask.
            let hears = WATCHING.load(core::sync::atomic::Ordering::Acquire);
            let heard = if hears { stop as extern "C" fn(i32) as usize } else { 0 };
            signal(SIGINT, heard);
            signal(SIGTERM, heard);
            STOPPED_AT = 0;
        }
    }

    // ---- what the program calls ---------------------------------------------------------

    /// Forgets the routes, before a server's own are given.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_reset() {
        unsafe {
            ROUTES.len = 0;
            ROUTE_TEXT.len = 0;
        }
    }

    /// Adds a route: a method, or `*` for any, and a path such as `/orders/:id`.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_route(method: *const Header, pattern: *const Header) {
        unsafe {
            let start = ROUTE_TEXT.len;
            ROUTE_TEXT.extend(bytes(method));
            let middle = ROUTE_TEXT.len;
            ROUTE_TEXT.extend(bytes(pattern));
            ROUTES.push(Route {
                method: Range { start, end: middle },
                pattern: Range {
                    start: middle,
                    end: ROUTE_TEXT.len,
                },
            });
        }
    }

    unsafe fn failed(what: &[u8], port: i64) -> i64 {
        let code = unsafe { error() };
        let mut out = Buffer::new();
        let _ = out.write_str("cannot ");
        let _ = out.write_str(core::str::from_utf8(what).unwrap_or(""));
        let _ = write!(out, " port {port}: ");
        let _ = out.write_str(core::str::from_utf8(unsafe { c_text(strerror(code)) }).unwrap_or("it failed"));
        unsafe {
            if LISTEN >= 0 {
                close(LISTEN);
                LISTEN = -1;
            }
            failed_with(new_str(&out.data[..out.len]));
            thread::me().failed = false;
        }
        -1
    }

    /// Starts listening on a port, with a number of worker processes (0 is one for each
    /// core), the most a request's body may be, and whether each request is logged.
    /// Returns the port, which is the one the system chose if 0 was asked for, or -1
    /// after leaving the reason where `koo_last_error` finds it.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_listen(port: i64, workers: i64, max_body: i64, log: i64) -> i64 {
        unsafe {
            if !(0..=65535).contains(&port) {
                *errno_location() = 22;
                return failed(b"listen on", port);
            }
            MAX_BODY = max_body.clamp(0, 1 << 30) as usize;
            LOG = log != 0;
            // (Nothing of a server that ran before in this program is in this one's way.)
            NO_SOCKETS = false;
            CROWDED = false;
            // At every address of the machine, of the newer kind and the older: one
            // socket of the newer kind takes both, where the system has the newer; where
            // it has not, or will not listen so, the socket is of the older kind, as it
            // always was.
            let on = 1i32;
            // Some systems let a socket of the newer kind take a port that another
            // program holds at the addresses of the older kind, and the server would
            // then start without a word and never hear the clients that reach the
            // other. So the port is first asked for in the older way, as it always
            // was, which says whether it is taken.
            if port != 0 {
                let probe = socket(AF_INET, SOCK_STREAM, 0);
                if probe >= 0 {
                    setsockopt(probe, sys::SOL_SOCKET, sys::SO_REUSEADDR, &on, 4);
                    let address = sys::address(port as u16);
                    let taken = bind(probe, &address as *const sys::SockAddr as *const u8, 16) != 0;
                    let why = *errno_location();
                    close(probe);
                    if taken {
                        *errno_location() = why;
                        return failed(b"listen on", port);
                    }
                }
            }
            let both = {
                let fd = socket(sys::AF_INET6, SOCK_STREAM, 0);
                let off = 0i32;
                // A socket's address as the system keeps it: on macOS a length and the
                // kind in a byte each, on Linux the kind in two; then the port, and
                // twenty-four bytes that are zero for "any address".
                let mut address = [0u8; 28];
                if cfg!(target_os = "macos") {
                    address[0] = 28;
                    address[1] = sys::AF_INET6 as u8;
                } else {
                    address[..2].copy_from_slice(&(sys::AF_INET6 as u16).to_ne_bytes());
                }
                address[2..4].copy_from_slice(&(port as u16).to_be_bytes());
                if fd >= 0
                    && setsockopt(fd, sys::IPPROTO_IPV6, sys::IPV6_V6ONLY, &off, 4) == 0
                    && setsockopt(fd, sys::SOL_SOCKET, sys::SO_REUSEADDR, &on, 4) == 0
                    && bind(fd, address.as_ptr(), 28) == 0
                    && listen(fd, 1024) == 0
                {
                    fd
                } else {
                    if fd >= 0 {
                        close(fd);
                    }
                    -1
                }
            };
            if both >= 0 {
                LISTEN = both;
            } else {
                LISTEN = socket(AF_INET, SOCK_STREAM, 0);
                if LISTEN < 0 {
                    return failed(b"open a socket for", port);
                }
                setsockopt(LISTEN, sys::SOL_SOCKET, sys::SO_REUSEADDR, &on, 4);
                let address = sys::address(port as u16);
                if bind(LISTEN, &address as *const sys::SockAddr as *const u8, 16) != 0 || listen(LISTEN, 1024) != 0 {
                    return failed(b"listen on", port);
                }
            }
            // The port is in the same place in an address of either kind.
            let (mut bound, mut len) = ([0u8; 28], 28u32);
            getsockname(LISTEN, bound.as_mut_ptr(), &mut len);
            let port = u16::from_be_bytes([bound[2], bound[3]]) as i64;
            fcntl(LISTEN, F_SETFL, fcntl(LISTEN, F_GETFL) | sys::O_NONBLOCK);
            // As many connections as the system allows this process.
            let mut limit = [0u64; 2];
            if getrlimit(sys::RLIMIT_NOFILE, &mut limit) == 0 {
                let wanted = [limit[1].min(65536).max(limit[0]), limit[1]];
                if setrlimit(sys::RLIMIT_NOFILE, &wanted) != 0 {
                    let fewer = [limit[1].min(10240).max(limit[0]), limit[1]];
                    setrlimit(sys::RLIMIT_NOFILE, &fewer);
                }
            }
            // Not all of them are for connections: a handler opens files of its own, to
            // ask an actor, a database or another server, and one that found none left
            // would wait for a connection to close that waits for the handler. So where
            // files are few, half are kept for the handlers; where they are many, two
            // for each handler there can be at a time.
            MOST_OPEN = usize::MAX;
            if getrlimit(sys::RLIMIT_NOFILE, &mut limit) == 0 {
                let files = limit[0].min(1 << 30) as usize;
                MOST_OPEN = (files / 2)
                    .max(files.saturating_sub(2 * MOST_PLACES))
                    .saturating_sub(16)
                    .max(8);
            }
            // What the server counts of itself, in memory its workers will share.
            if COUNTS.load(core::sync::atomic::Ordering::Acquire).is_null() {
                let made = crate::fiber::shared(core::mem::size_of::<Counts>()) as *mut Counts;
                COUNTS.store(made, core::sync::atomic::Ordering::Release);
            }
            // (A program that asks whether it is to stop, and was told before its server
            // began, is not untold: the server ends as soon as it has begun.)
            if !WATCHING.load(core::sync::atomic::Ordering::Acquire) {
                STOP.store(false, core::sync::atomic::Ordering::Relaxed);
            }
            STOPPED_AT = 0;
            CHILD = false;
            CHILD_COUNT = 0;
            PARENT_ID = getpid();
            QUEUE = -1;
            signal(SIGPIPE, 1);
            signal(SIGINT, stop as extern "C" fn(i32) as usize);
            signal(SIGTERM, stop as extern "C" fn(i32) as usize);
            SUPERVISOR = false;
            if workers == 1 {
                // One process, which serves: the plainest thing to run and to debug.
                start_queue();
                return port;
            }
            let cores = thread::processors() as i64;
            // (A number below zero is one that the environment gave: so many workers,
            // with a process to watch over them even where it is one.)
            let workers = match workers {
                0 => cores,
                below if below < 0 => -below,
                workers => workers,
            }
            .min(MAX_WORKERS as i64);
            // What is waiting to be printed is printed once, not once by every worker.
            koo_flush();
            // The actors of the program are kept here, by the process that watches.
            super::super::actor::before_fork();
            for _ in 0..workers {
                let child = fork();
                if child > 0
                    && let Some(counts) = counts()
                {
                    counts.workers.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
                if child == 0 {
                    // A queue belongs to the process that made it, so each worker
                    // makes its own.
                    become_worker();
                    return port;
                }
                if child > 0 {
                    CHILDREN[CHILD_COUNT] = child;
                    CHILD_COUNT += 1;
                }
            }
            if CHILD_COUNT == 0 {
                // No process could be started: this one serves.
                start_queue();
                return port;
            }
            // This process answers nothing: it watches over those that do, and starts
            // the ones that could not be started just now when they can be.
            WORKERS_WANTED = workers as usize;
            thread::RESERVE.store(1 + WORKERS_WANTED - CHILD_COUNT, core::sync::atomic::Ordering::Relaxed);
            SUPERVISOR = true;
            port
        }
    }

    // ---- handlers side by side ---------------------------------------------------------
    //
    // A place is a stack with a handler on it, or waiting for one (`fiber.rs`). The
    // thread is in one place at a time. The place it is in runs the loop below until
    // a request is ready, and then the request's handler. If the handler has to wait,
    // the thread leaves it there and goes to a place that is free, or makes one, which
    // runs the loop in its turn; when the loop finds that what a handler waited for has
    // come, the thread goes back to that handler. The first place is the thread's own
    // stack.

    /// The most places a worker has: the most requests it answers at one time.
    const MOST_PLACES: usize = 128;
    /// What the wait of a place is registered with: this, and the place's number.
    const PLACE: usize = 0x7fff_fffe_0000_0000;

    #[derive(Clone, Copy, PartialEq)]
    enum Doing {
        /// Running the loop, looking for a request.
        Driving,
        /// Left in the loop, for a time when there is more to do than one place can.
        Parked,
        /// Running a handler.
        Handling,
        /// In a handler that waits, for a socket or for a time.
        Waiting,
        /// In a handler whose wait is over.
        Ready,
        /// Its loop has ended, because the server is stopping.
        Done,
    }

    struct Place {
        /// Where its stack was left, and where the stack's memory begins.
        sp: usize,
        stack: *mut u8,
        /// What a thread has of its own, which a place has of its own.
        thread: *mut thread::Thread,
        doing: Doing,
        /// What it waits for: a socket, and a time (0 for none) after which it waits no
        /// longer; and whether it was the time that ended the wait.
        fd: i32,
        until: i64,
        late: bool,
        /// Whether what it waits for is its turn at something that another handler
        /// gives back, a connection of a pool: it is woken when one is (`give_turns`).
        turn: bool,
        /// Whether its request was given up: the client that made it has left, and
        /// there is nobody to answer. A handler whose request was given up waits for
        /// nothing more (see `given_up`).
        given_up: bool,
        /// When the time its handler is given for the request is over (0 for never),
        /// and whether that is why the request was given up: its client is still
        /// there then, and is told.
        deadline: i64,
        timed_out: bool,
        /// Whether its handler is asking an actor in another process, and until when
        /// one whose request was given up is still let wait for the actor's answer
        /// (0 before it has had to be decided). See `asking`.
        asking: bool,
        grace: i64,
        /// How many times a handler whose request was given up has gone on to begin
        /// something that waits: one that does so without end is ended (`cut_short`).
        cut: u32,
        /// The number of its wait, which the queue says back with what it waited for:
        /// word of an earlier wait, come late, is not taken for word of this one.
        serial: u32,
        /// Its request, while the thread is elsewhere.
        current: Current,
    }

    const NO_PLACE: Place = Place {
        sp: 0,
        stack: core::ptr::null_mut(),
        thread: core::ptr::null_mut(),
        doing: Doing::Driving,
        fd: -1,
        until: 0,
        late: false,
        turn: false,
        given_up: false,
        deadline: 0,
        timed_out: false,
        asking: false,
        grace: 0,
        cut: 0,
        serial: 0,
        current: NO_CURRENT,
    };

    static mut PLACES: [Place; MOST_PLACES] = [NO_PLACE; MOST_PLACES];
    static mut PLACE_COUNT: usize = 1;
    /// The place the thread is in.
    static mut HERE: usize = 0;
    /// Whether handlers have places: not in the process that only watches over workers,
    /// and not when the program was built to be checked by a sanitizer.
    static mut SIDE_BY_SIDE: bool = false;
    static mut PLAIN: bool = false;
    /// The server is stopping, and every place's loop is to end.
    static mut ENDING: bool = false;
    /// The program's loop, for a new place to run.
    static mut JOB: *mut Header = core::ptr::null_mut();

    /// Leaves the place the thread is in for another.
    #[allow(static_mut_refs)]
    unsafe fn go_to(next: usize) {
        unsafe {
            let from = HERE;
            if from == next {
                return;
            }
            PLACES[from].current = CURRENT;
            CURRENT = PLACES[next].current;
            HERE = next;
            thread::become_block(PLACES[next].thread);
            fiber::switch(&mut PLACES[from].sp, PLACES[next].sp);
        }
    }

    /// Whether the loop could go on in another place if this one began a request.
    #[allow(static_mut_refs)]
    unsafe fn a_place_to_spare() -> bool {
        unsafe {
            (PLACE_COUNT < MOST_PLACES && !ENDING && thread::room())
                || (0..PLACE_COUNT).any(|place| place != HERE && PLACES[place].doing == Doing::Parked)
        }
    }

    /// A place to run the loop while this one waits: one that was left in it, or a new one.
    #[allow(static_mut_refs)]
    unsafe fn another_place() -> Option<usize> {
        unsafe {
            if let Some(parked) = (0..PLACE_COUNT).find(|place| PLACES[*place].doing == Doing::Parked) {
                return Some(parked);
            }
            if PLACE_COUNT == MOST_PLACES || ENDING {
                return None;
            }
            let block = thread::spare_block();
            if block.is_null() {
                return None;
            }
            let (stack, sp) = fiber::new_stack(place_begins)?;
            (*block).stack_low = stack as usize + fiber::GUARD;
            PLACES[PLACE_COUNT] = Place {
                sp,
                stack,
                thread: block,
                ..NO_PLACE
            };
            PLACE_COUNT += 1;
            Some(PLACE_COUNT - 1)
        }
    }

    /// What a new place does: it runs the program's loop, which asks for requests and
    /// answers them, until the server stops.
    #[allow(static_mut_refs)]
    extern "C" fn place_begins() -> ! {
        unsafe {
            let run: extern "C" fn(*mut Header) = core::mem::transmute(actor::runner());
            // The function that is run lets go of the loop it is given.
            retain(JOB);
            run(JOB);
            PLACES[HERE].doing = Doing::Done;
            loop {
                let next = (1..PLACE_COUNT)
                    .find(|place| PLACES[*place].doing != Doing::Done)
                    .unwrap_or(0);
                go_to(next);
            }
        }
    }

    /// Whether the thread is in a handler that may be left where it is: on the server's
    /// thread, and holding no actor, which another handler might then wait for on the
    /// same thread, for ever.
    #[allow(static_mut_refs)]
    unsafe fn may_leave() -> bool {
        unsafe {
            // (Asked by any thread that waits or sleeps: one that is not the server's
            // looks at nothing of the server's but which thread that is.)
            SERVER.load(core::sync::atomic::Ordering::Acquire) == thread::this_thread()
                && SIDE_BY_SIDE
                && PLACES[HERE].doing == Doing::Handling
                && {
                    let mine = thread::me();
                    core::ptr::eq(mine, PLACES[HERE].thread) && mine.held_len == 0
                }
        }
    }

    /// The thread does other work until `fd` has something to read, or room to write, or
    /// the time has come. See `fiber::wait`.
    #[allow(static_mut_refs)]
    unsafe fn wait_in_place(fd: i32, write: bool, until: i64) -> fiber::Waited {
        unsafe {
            if !may_leave() {
                return fiber::Waited::NotHere;
            }
            // A handler whose request was given up waits for nothing: whatever it
            // asked for is said not to have come in time, which those who wait know
            // what to do about, and it ends at the next place where it can. (Except
            // for the answer of an actor it asks, for a second: see `asking`.)
            if given_up() && (!PLACES[HERE].asking || out_of_grace()) {
                return fiber::Waited::TimeUp;
            }
            let here = HERE;
            let asked_until = until;
            loop {
                // (Where the loop goes on meanwhile is found each time: the place it
                // went on in before may have a handler of its own by now.)
                let Some(next) = another_place() else {
                    return fiber::Waited::NotHere;
                };
                // No wait of a handler's is longer than the time its request has left;
                // nor, once that is over, than what it is still given for an actor.
                let mut until = asked_until;
                for limit in [PLACES[here].deadline, PLACES[here].grace] {
                    if limit != 0 && limit > nanos() && (until == 0 || limit < until) {
                        until = limit;
                    }
                }
                if PLACES[here].given_up && PLACES[here].grace != 0 && (until == 0 || PLACES[here].grace < until) {
                    until = PLACES[here].grace;
                }
                PLACES[here].serial = (PLACES[here].serial + 1) & 0xff_ffff;
                if fd >= 0 {
                    sys::watch_once(QUEUE, fd, write, PLACE | (PLACES[here].serial as usize) << 8 | here);
                }
                PLACES[here].fd = fd;
                PLACES[here].until = until;
                PLACES[here].late = false;
                PLACES[here].doing = Doing::Waiting;
                go_to(next);
                // The loop came back here: what was waited for has come, or the time has.
                PLACES[here].doing = Doing::Handling;
                let late = PLACES[here].late;
                if fd >= 0 {
                    sys::unwatch(QUEUE, fd, write, !late);
                }
                if !late {
                    return fiber::Waited::Came;
                }
                // The time that came may be the wait's own, or that of the request,
                // which is then given up (found here), or its client may have left.
                // A handler that is asking an actor waits on, for what it is given.
                let own = asked_until != 0 && nanos() >= asked_until;
                if !own && given_up() && PLACES[here].asking && !out_of_grace() {
                    continue;
                }
                return fiber::Waited::TimeUp;
            }
        }
    }

    /// The handler waits its turn at something that another handler will give back:
    /// the thread does other work until one says it has (`give_turns`), or for so long
    /// at the most. Returns whether it did; if not, the caller waits as a thread does
    /// where no server runs.
    #[allow(static_mut_refs)]
    pub unsafe fn await_turn(length: i64) -> bool {
        unsafe {
            if !may_leave() {
                return false;
            }
            let here = HERE;
            PLACES[here].turn = true;
            let waited = wait_in_place(-1, false, nanos() + length.max(0));
            PLACES[here].turn = false;
            waited != fiber::Waited::NotHere
        }
    }

    /// Something that handlers wait their turn at has been given back: those that
    /// wait are woken, to see whose turn it is. From any other thread than the
    /// server's it does nothing, since the handlers are that thread's alone.
    #[allow(static_mut_refs)]
    pub unsafe fn give_turns() {
        unsafe {
            if SERVER.load(core::sync::atomic::Ordering::Acquire) != thread::this_thread() || !SIDE_BY_SIDE {
                return;
            }
            for place in PLACES[..PLACE_COUNT].iter_mut() {
                if place.turn && place.doing == Doing::Waiting {
                    place.late = false;
                    place.doing = Doing::Ready;
                }
            }
        }
    }

    /// The thread does other work for so long. See `fiber::pause`.
    unsafe fn pause_in_place(length: i64) -> bool {
        unsafe {
            let until = nanos() + length.max(0);
            // The time is what is waited for, so its coming is no lateness. And nothing
            // but the time ends the wait: a thread that was told it had not waited
            // would go and sleep, the whole server with it.
            loop {
                match wait_in_place(-1, false, until) {
                    fiber::Waited::NotHere => return false,
                    fiber::Waited::TimeUp => return true,
                    fiber::Waited::Came if nanos() >= until => return true,
                    fiber::Waited::Came => {}
                }
            }
        }
    }

    /// When the server was told to stop, and how long after that it still answers what
    /// arrives on the connections it has, in nanoseconds.
    static mut STOPPED_AT: i64 = 0;
    const LINGER: i64 = 300_000_000;
    /// How long after it was told to stop a server still sends what is left of the
    /// answers it has begun.
    const DRAIN: i64 = 30_000_000_000;

    /// How many handlers have panicked in this process.
    static mut PANICS: usize = 0;
    /// After so many, a worker is done: what a panic leaves behind it is not given
    /// back, and a worker that is replaced gives everything back.
    const MOST_PANICS: usize = 1000;

    /// Says that a handler panicked. A worker that has seen too many answers the
    /// requests it has in hand and ends, and the process that watches starts another.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_panicked() {
        unsafe {
            if let Some(counts) = counts() {
                counts.panics.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
            crate::thread::me().json_wire = false;
            crate::thread::me().json_base = 0;
            PANICS += 1;
            if (CHILD && PANICS >= MOST_PANICS) || OUT_OF_STACK {
                STOP.store(true, core::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Called as the program starts, if it was built to be checked by a sanitizer, which
    /// does not know of stacks that are switched.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_plain_stacks() {
        unsafe { PLAIN = true };
    }

    /// Whether handlers may have places at all.
    unsafe fn places_allowed() -> bool {
        unsafe {
            unsafe extern "C" {
                fn getenv(name: *const u8) -> *const u8;
            }
            let said = getenv(c"KOO_SIDE_BY_SIDE".as_ptr() as *const u8);
            let off = !said.is_null() && c_text(said).eq_ignore_ascii_case(b"off");
            !PLAIN && !off && actor::runner() != 0
        }
    }

    /// From here on a handler that waits is left in its place: in a process that
    /// answers requests, once the program has said what its loop is.
    #[allow(static_mut_refs)]
    unsafe fn begin_places() {
        unsafe {
            if SUPERVISOR || JOB.is_null() || !places_allowed() {
                return;
            }
            PLACES[0] = Place {
                thread: thread::me(),
                ..NO_PLACE
            };
            PLACE_COUNT = 1;
            HERE = 0;
            ENDING = false;
            SIDE_BY_SIDE = true;
            SERVER.store(thread::this_thread(), core::sync::atomic::Ordering::Release);
            fiber::serving(Some((wait_in_place, pause_in_place)));
        }
    }

    /// The server has stopped: the places are given back, and the loop let go of.
    #[allow(static_mut_refs)]
    unsafe fn end_places() {
        unsafe {
            fiber::serving(None);
            SERVER.store(0, core::sync::atomic::Ordering::Release);
            SIDE_BY_SIDE = false;
            for place in PLACES[1..PLACE_COUNT].iter() {
                fiber::free_stack(place.stack);
            }
            PLACE_COUNT = 1;
            HERE = 0;
            if !JOB.is_null() {
                actor::drop_function(JOB);
                JOB = core::ptr::null_mut();
            }
        }
    }

    /// Says what the program's loop is, which takes requests and answers them: `job`
    /// runs it, and each place that is made runs it too. The program then runs the loop
    /// itself, on the thread's own stack, which is the first place.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_places(job: *mut Header) {
        unsafe {
            if !JOB.is_null() {
                actor::drop_function(JOB);
            }
            JOB = job;
            begin_places();
        }
    }

    /// Runs the server until a request is ready, and returns the number of its route, or
    /// -2 when the server has been told to stop. A request for no route is answered here.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_next() -> i64 {
        unsafe {
            if LISTEN < 0 {
                return -2;
            }
            if SUPERVISOR {
                match supervise() {
                    0 => return -2,
                    2 => return -3,
                    _ => {}
                }
            }
            if !CURRENT.answered {
                // The program asked for another request without answering the last.
                koo_http_respond(CURRENT.id, 500, core::ptr::null(), core::ptr::null(), core::ptr::null());
            }
            PLACES[HERE].doing = Doing::Driving;
            PLACES[HERE].given_up = false;
            PLACES[HERE].deadline = 0;
            PLACES[HERE].timed_out = false;
            PLACES[HERE].asking = false;
            PLACES[HERE].grace = 0;
            PLACES[HERE].cut = 0;
            loop {
                if ENDING {
                    // Every place's loop ends, the first place's last: it is the one
                    // that returns to `main`.
                    if HERE != 0 {
                        return -2;
                    }
                    if let Some(other) = (1..PLACE_COUNT).find(|place| PLACES[*place].doing != Doing::Done) {
                        PLACES[0].doing = Doing::Parked;
                        go_to(other);
                        continue;
                    }
                    finish();
                    return -2;
                }
                // A handler whose wait is over goes on, and this place waits its turn.
                if let Some(ready) = (0..PLACE_COUNT).find(|place| PLACES[*place].doing == Doing::Ready) {
                    PLACES[HERE].doing = Doing::Parked;
                    go_to(ready);
                    PLACES[HERE].doing = Doing::Driving;
                    continue;
                }
                // A place is kept for this loop: with every other in a handler, no new
                // request is begun here, so that a handler that waits can always be
                // left, and what it waits for, which may be another handler of this
                // worker, is attended to.
                while READY.len > READY_AT && (!SIDE_BY_SIDE || a_place_to_spare()) {
                    let index = *READY.ptr.add(READY_AT) as usize;
                    READY_AT += 1;
                    if READY_AT == READY.len {
                        (READY.len, READY_AT) = (0, 0);
                    } else if READY_AT >= 4096 {
                        // What has been taken up is let go of, where the list is never
                        // empty for long enough to begin again.
                        let left = READY.len - READY_AT;
                        core::ptr::copy(READY.ptr.add(READY_AT), READY.ptr, left);
                        (READY.len, READY_AT) = (left, 0);
                    }
                    let c = conn(index);
                    c.queued = false;
                    if c.fd < 0 || c.handling {
                        continue;
                    }
                    // A client that does not read its answers is not read from.
                    if c.output.len - c.sent > MAX_PENDING {
                        continue;
                    }
                    match parse(index) {
                        Parsed::Request(-1) => no_route(index),
                        Parsed::Request(number) => {
                            PLACES[HERE].doing = Doing::Handling;
                            if REQUEST_TIME > 0 {
                                PLACES[HERE].deadline = nanos().saturating_add(REQUEST_TIME);
                            }
                            return number;
                        }
                        Parsed::Partial => {
                            let c = conn(index);
                            if c.fd >= 0 && c.closing && c.output.len == c.sent {
                                shut(index);
                            }
                        }
                        Parsed::Handled => {}
                    }
                }
                if EVENT_AT < EVENT_COUNT {
                    EVENT_AT += 1;
                    event(EVENTS[EVENT_AT - 1]);
                    continue;
                }
                if STOP.load(core::sync::atomic::Ordering::Relaxed) {
                    // No new connection is taken. The handlers that are waiting finish
                    // first; and for a moment longer a request that was already on its
                    // way on a connection that is open is still answered, and told that
                    // the connection ends with the answer, so that a client that had
                    // just sent one is not left with nothing.
                    if STOPPED_AT == 0 {
                        STOPPED_AT = nanos();
                    }
                    let waiting = PLACES[..PLACE_COUNT]
                        .iter()
                        .any(|place| matches!(place.doing, Doing::Waiting | Doing::Ready));
                    // An answer that is still going out, to a client that takes it
                    // slowly, is in hand as much as one that is not yet written: it is
                    // sent to the end, for as long as half a minute. (One that the
                    // client has stopped taking is given up sooner, as at any time.)
                    let sending = nanos() - STOPPED_AT < DRAIN
                        && CONNS.items().iter().any(|c| c.fd >= 0 && c.output.len > c.sent);
                    if !waiting && !sending && nanos() - STOPPED_AT > LINGER {
                        if PLACE_COUNT > 1 {
                            ENDING = true;
                            continue;
                        }
                        finish();
                        return -2;
                    }
                }
                take_or_leave();
                sweep();
                // A handler that waits until a time is woken when the time has come.
                let (time, mut soonest, mut late) = (nanos(), 1000, false);
                for place in PLACES[..PLACE_COUNT].iter_mut() {
                    if place.doing == Doing::Waiting && place.until != 0 {
                        if place.until <= time {
                            place.late = true;
                            place.doing = Doing::Ready;
                            late = true;
                        } else {
                            soonest = soonest.min((place.until - time) / 1_000_000 + 1);
                        }
                    }
                }
                if late {
                    // What has arrived meanwhile is taken note of first, without
                    // waiting for any: a handler whose waits all end at once, as those
                    // of one that was given up do, would otherwise go from one to the
                    // next and the queue never be looked at.
                    EVENT_COUNT = sys::wait(QUEUE, &mut EVENTS, 0);
                    EVENT_AT = 0;
                    continue;
                }
                if STOP.load(core::sync::atomic::Ordering::Relaxed) {
                    soonest = soonest.min(50);
                }
                EVENT_COUNT = sys::wait(QUEUE, &mut EVENTS, soonest);
                EVENT_AT = 0;
            }
        }
    }

    #[allow(static_mut_refs)]
    unsafe fn input<'a>(id: i64) -> Option<&'a [u8]> {
        unsafe {
            if CURRENT.answered || CURRENT.id != id {
                return None;
            }
            let c = conn(CURRENT.conn);
            if c.generation != CURRENT.generation || c.input.ptr.is_null() {
                return None;
            }
            Some(core::slice::from_raw_parts(c.input.ptr, c.input.len))
        }
    }

    /// Text from a request as a string: what `%` escapes stand for, and `+` as a space
    /// where a query writes one.
    #[allow(static_mut_refs)]
    unsafe fn decoded(raw: &[u8], plus: bool) -> *mut Header {
        let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
        if !raw.iter().any(|b| *b == b'%' || (plus && *b == b'+')) {
            return unsafe { str_from_outside(raw) };
        }
        unsafe {
            SCRATCH.len = 0;
            let mut at = 0;
            while at < raw.len() {
                let byte = raw[at];
                let escaped = match (
                    byte,
                    raw.get(at + 1).copied().and_then(hex),
                    raw.get(at + 2).copied().and_then(hex),
                ) {
                    (b'%', Some(high), Some(low)) => Some(high * 16 + low),
                    _ => None,
                };
                match escaped {
                    Some(value) => {
                        SCRATCH.push(value);
                        at += 3;
                    }
                    None => {
                        SCRATCH.push(if plus && byte == b'+' { b' ' } else { byte });
                        at += 1;
                    }
                }
            }
            str_from_outside(SCRATCH.items())
        }
    }

    /// The number of the request being answered. What is asked of another number is empty.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_id() -> i64 {
        unsafe { CURRENT.id }
    }

    /// The name a request goes by (see `named`). What is asked of another request
    /// than the one being answered is empty.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_request_id(id: i64) -> *mut Header {
        unsafe {
            if id != CURRENT.id {
                return new_str(b"");
            }
            new_str(&CURRENT.name[..CURRENT.name_len])
        }
    }

    /// The name of the request that the calling thread is answering, for a line of the
    /// program's log; empty on any thread that is answering none: a thread of an
    /// actor's, or the program before and after it serves.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_request_now() -> *mut Header {
        unsafe {
            let (name, len) = name_now();
            new_str(&name[..len])
        }
    }

    /// The same, for the lines the library writes itself: the name, and how long it is.
    #[allow(static_mut_refs)]
    pub unsafe fn name_now() -> ([u8; MOST_NAME], usize) {
        unsafe {
            let here = SERVER.load(core::sync::atomic::Ordering::Acquire) == thread::this_thread();
            if !here || CURRENT.answered || CURRENT.id == 0 {
                return ([0; MOST_NAME], 0);
            }
            (CURRENT.name, CURRENT.name_len)
        }
    }

    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_method(id: i64) -> *mut Header {
        unsafe {
            match input(id) {
                Some(data) => str_from_outside(&data[CURRENT.method.start..CURRENT.method.end]),
                None => new_str(b""),
            }
        }
    }

    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_path(id: i64) -> *mut Header {
        unsafe {
            match input(id) {
                Some(data) => decoded(&data[CURRENT.path.start..CURRENT.path.end], false),
                None => new_str(b""),
            }
        }
    }

    /// What the `*` at the end of the route stands for. For a path that climbs out of
    /// where it is (`..`) the answer is `/`, which nothing else is answered with.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_rest(id: i64) -> *mut Header {
        unsafe {
            let Some(data) = input(id) else {
                return new_str(b"");
            };
            let rest = decoded(&data[CURRENT.rest.start..CURRENT.rest.end], false);
            let text = bytes(rest);
            let climbs = text.split(|b| *b == b'/' || *b == b'\\').any(|part| part == b"..")
                || text.contains(&0)
                || text.first() == Some(&b'/');
            if climbs {
                koo_free(rest as *mut u8);
                return new_str(b"/");
            }
            rest
        }
    }

    /// The part of the path that a `:name` of the route stands for.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_param(id: i64, name: *const Header) -> *mut Header {
        unsafe {
            let Some(data) = input(id) else {
                return new_str(b"");
            };
            let text = ROUTE_TEXT.items();
            for (key, value) in &CURRENT.params[..CURRENT.param_count] {
                if text[key.start..key.end] == *bytes(name) {
                    return decoded(&data[value.start..value.end], false);
                }
            }
            new_str(b"")
        }
    }

    /// Whether a name, as a query or a form writes it, is `name`: a browser writes
    /// `filter[name]` as `filter%5Bname%5D`, and a space as `+`. The escapes are read as
    /// `decoded` reads those of a value.
    fn is_name(raw: &[u8], name: &[u8]) -> bool {
        let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
        let (mut at, mut matched) = (0, 0);
        while at < raw.len() {
            let byte = raw[at];
            let (value, width) = match (
                byte,
                raw.get(at + 1).copied().and_then(hex),
                raw.get(at + 2).copied().and_then(hex),
            ) {
                (b'%', Some(high), Some(low)) => (high * 16 + low, 3),
                (b'+', ..) => (b' ', 1),
                _ => (byte, 1),
            };
            if name.get(matched) != Some(&value) {
                return false;
            }
            matched += 1;
            at += width;
        }
        matched == name.len()
    }

    /// The value of a name among `name=value` pairs joined by `&`, as it is written.
    fn pair<'a>(pairs: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
        for pair in pairs.split(|b| *b == b'&') {
            // Nothing between two `&`, or in a query that is empty, names nothing.
            if pair.is_empty() {
                continue;
            }
            let equals = pair.iter().position(|b| *b == b'=').unwrap_or(pair.len());
            if is_name(&pair[..equals], name) {
                return Some(pair.get(equals + 1..).unwrap_or(&[]));
            }
        }
        None
    }

    #[allow(static_mut_refs)]
    unsafe fn query<'a>(id: i64, name: &[u8]) -> Option<&'a [u8]> {
        unsafe {
            let data = input(id)?;
            pair(&data[CURRENT.query.start..CURRENT.query.end], name)
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_query_has(id: i64, name: *const Header) -> bool {
        unsafe { query(id, bytes(name)).is_some() }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_query(id: i64, name: *const Header) -> *mut Header {
        unsafe {
            match query(id, bytes(name)) {
                Some(value) => decoded(value, true),
                None => new_str(b""),
            }
        }
    }

    #[allow(static_mut_refs)]
    unsafe fn header<'a>(id: i64, name: &[u8]) -> Option<&'a [u8]> {
        unsafe {
            let data = input(id)?;
            for (key, value) in &CURRENT.headers[..CURRENT.header_count] {
                if data[key.start..key.end].eq_ignore_ascii_case(name) {
                    return Some(&data[value.start..value.end]);
                }
            }
            None
        }
    }

    /// A field of a form sent as the body, the way a page's form sends one.
    #[allow(static_mut_refs)]
    unsafe fn form<'a>(id: i64, name: &[u8]) -> Option<&'a [u8]> {
        unsafe {
            let data = input(id)?;
            let kind = header(id, b"content-type")?;
            if !kind
                .get(..33)
                .is_some_and(|start| start.eq_ignore_ascii_case(b"application/x-www-form-urlencoded"))
            {
                return None;
            }
            pair(&data[CURRENT.body.start..CURRENT.body.end], name)
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_form_has(id: i64, name: *const Header) -> bool {
        unsafe { form(id, bytes(name)).is_some() }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_form(id: i64, name: *const Header) -> *mut Header {
        unsafe {
            match form(id, bytes(name)) {
                Some(value) => decoded(value, true),
                None => new_str(b""),
            }
        }
    }

    /// A cookie the client sent back.
    #[allow(static_mut_refs)]
    unsafe fn cookie<'a>(id: i64, name: &[u8]) -> Option<&'a [u8]> {
        unsafe {
            let data = input(id)?;
            for (key, value) in &CURRENT.headers[..CURRENT.header_count] {
                if !data[key.start..key.end].eq_ignore_ascii_case(b"cookie") {
                    continue;
                }
                for one in data[value.start..value.end].split(|b| *b == b';') {
                    let (from, to) = trim(one);
                    let one = &one[from..to];
                    let equals = one.iter().position(|b| *b == b'=').unwrap_or(one.len());
                    if one[..equals] == *name {
                        return Some(one.get(equals + 1..).unwrap_or(&[]));
                    }
                }
            }
            None
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_cookie_has(id: i64, name: *const Header) -> bool {
        unsafe { cookie(id, bytes(name)).is_some() }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_cookie(id: i64, name: *const Header) -> *mut Header {
        unsafe {
            match cookie(id, bytes(name)) {
                Some(value) => decoded(value, false),
                None => new_str(b""),
            }
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_header_has(id: i64, name: *const Header) -> bool {
        unsafe { header(id, bytes(name)).is_some() }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_header(id: i64, name: *const Header) -> *mut Header {
        unsafe {
            match header(id, bytes(name)) {
                Some(value) => str_from_outside(value),
                None => new_str(b""),
            }
        }
    }

    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_body(id: i64) -> *mut Header {
        unsafe {
            match input(id) {
                Some(data) => str_from_outside(&data[CURRENT.body.start..CURRENT.body.end]),
                None => new_str(b""),
            }
        }
    }

    /// The body as the bytes it is, whatever they are. What is returned is not text
    /// that a program may have: the prelude makes a list of bytes of it at once.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_body_raw(id: i64) -> *mut Header {
        unsafe {
            match input(id) {
                Some(data) => new_str(&data[CURRENT.body.start..CURRENT.body.end]),
                None => new_str(b""),
            }
        }
    }

    unsafe fn text<'a>(s: *const Header) -> &'a [u8] {
        if s.is_null() { b"" } else { unsafe { bytes(s) } }
    }

    /// The connection of the request of this number, and whether it is to be kept, if
    /// the request is the one being answered.
    #[allow(static_mut_refs)]
    unsafe fn answering(id: i64) -> Option<(usize, bool)> {
        unsafe {
            if CURRENT.answered || CURRENT.id != id {
                return None;
            }
            let index = CURRENT.conn;
            // The connection may have ended while its handler waited, and another may
            // have its slot: the answer is for nobody then.
            if conn(index).fd < 0 || conn(index).generation != CURRENT.generation {
                if conn(index).left && conn(index).generation == CURRENT.generation {
                    // Closed while the handler had the request: the handler is done,
                    // and what was kept for it is let go of. 499 is what servers write
                    // in their logs for a request whose client did not wait.
                    log(499, 0, true);
                    let_go(index);
                }
                CURRENT.answered = true;
                return None;
            }
            let keep = CURRENT.keep && !conn(index).closing && !STOP.load(core::sync::atomic::Ordering::Relaxed);
            if SIDE_BY_SIDE
                && SERVER.load(core::sync::atomic::Ordering::Acquire) == thread::this_thread()
                && PLACES[HERE].timed_out
            {
                // Its handler had the time a request is given and was let wait for
                // nothing more: whatever it came to in the end, the client is told
                // that the server could not answer, and the log says which request.
                PLACES[HERE].timed_out = false;
                let body = b"Service Unavailable\n";
                log(503, body.len(), true);
                let data = core::slice::from_raw_parts(conn(index).input.ptr, conn(index).input.len);
                let text = |range: Range| &data[range.start..range.end];
                let mut seconds = Buffer::new();
                let _ = write!(seconds, "{}", REQUEST_TIME / 1_000_000);
                let mut said = Buffer::new();
                for part in [
                    text(CURRENT.method),
                    b" ",
                    &text(CURRENT.path)[..text(CURRENT.path).len().min(200)],
                    b": the handler was given ",
                    &seconds.data[..seconds.len],
                    b" ms and had not answered; it was let wait for nothing more, and the client was answered 503\n",
                ] {
                    said.push(part);
                }
                super::super::log::said(b"error", &said.data[..said.len], &said.data[..said.len - 1], &[]);
                if head(
                    index,
                    503,
                    b"text/plain; charset=utf-8",
                    b"",
                    body.len(),
                    CURRENT.head_only,
                    keep,
                ) {
                    conn(index).output.extend(body);
                }
                answered(index);
                return None;
            }
            Some((index, keep))
        }
    }

    /// The answer is queued: it is written, and the next request is looked for.
    #[allow(static_mut_refs)]
    unsafe fn answered(index: usize) {
        unsafe {
            CURRENT.answered = true;
            pass(index, CURRENT.length);
            let c = conn(index);
            c.handling = false;
            if c.paused {
                c.paused = false;
                c.reading = true;
                sys::start_reading(QUEUE, c.fd, tag(index, c.generation), c.writing);
            }
            flush(index);
            let c = conn(index);
            if c.fd >= 0 && c.input.len > c.start {
                // The client sent its next request already.
                ready(index);
            }
        }
    }

    /// Answers the request of this number. `extra` is whole header lines, each ended.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_respond(
        id: i64,
        status: i64,
        content_type: *const Header,
        extra: *const Header,
        body: *const Header,
    ) {
        unsafe {
            let Some((index, keep)) = answering(id) else { return };
            let status = if (100..1000).contains(&status) { status } else { 500 };
            let body = text(body);
            log(status, body.len(), true);
            if head(
                index,
                status,
                text(content_type),
                text(extra),
                body.len(),
                CURRENT.head_only,
                keep,
            ) {
                conn(index).output.extend(body);
            }
            answered(index);
        }
    }

    /// Answers the request of this number with a file. Its type is told by its name
    /// unless one is given. A file that cannot be read is answered with 404.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_http_respond_file(
        id: i64,
        status: i64,
        content_type: *const Header,
        extra: *const Header,
        path: *const Header,
        root: *const Header,
    ) {
        unsafe {
            let Some((index, keep)) = answering(id) else { return };
            // A directory is not a file, though it opens like one: what is sent for a
            // directory is the `index.html` in it, if it has one.
            let name = c_string(path);
            let directory = opendir(name);
            free(name);
            let mut inside: *mut Header = core::ptr::null_mut();
            let path = if directory.is_null() {
                path
            } else {
                closedir(directory);
                let named = bytes(path);
                inside = crate::koo_str_from_bytes(named.as_ptr(), named.len() as i64);
                let index: &[u8] = if named.ends_with(b"/") {
                    b"index.html"
                } else {
                    b"/index.html"
                };
                crate::koo_str_append_bytes(&mut inside, index.as_ptr(), index.len() as i64);
                inside as *const Header
            };
            let name = c_string(path);
            let directory = opendir(name);
            // A file that is served from a directory is in the directory, where it
            // really is and not only by its name: a link that leads out of the
            // directory leads to nothing that is served.
            let within = text(root).is_empty() || {
                let kept = c_string(root);
                let (really, bounds) = (
                    realpath(name, core::ptr::null_mut()),
                    realpath(kept, core::ptr::null_mut()),
                );
                free(kept);
                let inside_of = !really.is_null() && !bounds.is_null() && {
                    let file = core::slice::from_raw_parts(really, strlen(really));
                    let bounds = core::slice::from_raw_parts(bounds, strlen(bounds));
                    file.len() > bounds.len()
                        && file.starts_with(bounds)
                        && (bounds == b"/" || file[bounds.len()] == b'/')
                };
                if !really.is_null() {
                    free(really);
                }
                if !bounds.is_null() {
                    free(bounds);
                }
                inside_of
            };
            free(name);
            let file = if directory.is_null() && within {
                open(path, c"rb")
            } else {
                if !directory.is_null() {
                    closedir(directory);
                }
                core::ptr::null_mut()
            };
            let size = if file.is_null() || fseek(file, 0, 2) != 0 {
                -1
            } else {
                ftell(file)
            };
            if size < 0 || size as usize > MAX_FILE || fseek(file, 0, 0) != 0 {
                if !file.is_null() {
                    fclose(file);
                }
                log(404, 10, true);
                if head(
                    index,
                    404,
                    b"text/plain; charset=utf-8",
                    b"",
                    10,
                    CURRENT.head_only,
                    keep,
                ) {
                    conn(index).output.extend(b"Not Found\n");
                }
                answered(index);
                if !inside.is_null() {
                    koo_free(inside as *mut u8);
                }
                return;
            }
            let size = size as usize;
            let kind = if text(content_type).is_empty() {
                media_type(bytes(path))
            } else {
                text(content_type)
            };
            let status = if (100..1000).contains(&status) { status } else { 500 };
            log(status, size, true);
            if head(index, status, kind, text(extra), size, CURRENT.head_only, keep) {
                let out = &mut conn(index).output;
                out.reserve(size);
                let got = fread(out.ptr.add(out.len), 1, size, file);
                out.len += size;
                if got < size {
                    // The file could not be read to its end. The length is already
                    // said, so the rest is sent as nothing, and the connection ends.
                    core::ptr::write_bytes(out.ptr.add(out.len - size + got), 0, size - got);
                    conn(index).closing = true;
                }
            }
            fclose(file);
            answered(index);
            if !inside.is_null() {
                koo_free(inside as *mut u8);
            }
        }
    }
}
