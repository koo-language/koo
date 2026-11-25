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
    pub unsafe extern "C" fn koo_http_respond(_: i64, _: i64, _: *const Header, _: *const Header, _: *const Header) {}
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_http_respond_file(
        _: i64,
        _: i64,
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
        fn fcntl(fd: i32, command: i32, ...) -> i32;
        fn close(fd: i32) -> i32;
        fn signal(number: i32, handler: usize) -> usize;
        fn fork() -> i32;
        fn getpid() -> i32;
        fn kill(process: i32, number: i32) -> i32;
        fn waitpid(process: i32, status: *mut i32, options: i32) -> i32;
        fn sysconf(name: i32) -> i64;
        fn setrlimit(resource: i32, limit: *const [u64; 2]) -> i32;
        fn getrlimit(resource: i32, limit: *mut [u64; 2]) -> i32;
        fn opendir(path: *const u8) -> *mut u8;
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
        pub const SC_NPROCESSORS_ONLN: i32 = 58;
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

        /// Waits for the socket to take more output, once.
        pub unsafe fn want_write(queue: i32, fd: i32, tag: usize, _reading: bool) {
            unsafe { change(queue, fd as usize, EVFILT_WRITE, EV_ADD | EV_ONESHOT, 0, tag) };
        }

        /// The output is written: nothing more is waited for on that side.
        pub unsafe fn wrote(_queue: i32, _fd: i32, _tag: usize, _reading: bool) {}

        pub unsafe fn stop_reading(queue: i32, fd: i32, _tag: usize, _writing: bool) {
            unsafe { change(queue, fd as usize, EVFILT_READ, EV_DELETE, 0, 0) };
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
        pub const SC_NPROCESSORS_ONLN: i32 = 84;
        pub const RLIMIT_NOFILE: i32 = 7;
        pub const CLOCK_MONOTONIC: i32 = 1;
        const EPOLLIN: u32 = 0x1;
        const EPOLLOUT: u32 = 0x4;
        const EPOLLERR: u32 = 0x8;
        const EPOLLHUP: u32 = 0x10;
        const EPOLLRDHUP: u32 = 0x2000;
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

        pub unsafe fn want_write(queue: i32, fd: i32, tag: usize, reading: bool) {
            unsafe { control(queue, MOD, fd, interest(reading, true), tag) };
        }

        pub unsafe fn wrote(queue: i32, fd: i32, tag: usize, reading: bool) {
            unsafe { control(queue, MOD, fd, interest(reading, false), tag) };
        }

        pub unsafe fn stop_reading(queue: i32, fd: i32, tag: usize, writing: bool) {
            unsafe { control(queue, MOD, fd, interest(false, writing), tag) };
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
        /// When it last read or wrote, and when the request being read began, in seconds.
        active: i64,
        began: i64,
    }

    struct Route {
        method: Range,
        pattern: Range,
    }

    /// The request the program is answering.
    struct Current {
        id: i64,
        answered: bool,
        conn: usize,
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
    }

    static mut LISTEN: i32 = -1;
    static mut QUEUE: i32 = -1;
    static mut ACCEPTING: bool = false;
    static mut MAX_BODY: usize = 1024 * 1024;
    static mut LOG: bool = false;
    static mut CONNS: List<Conn> = List::new();
    static mut FREE: List<u32> = List::new();
    static mut READY: List<u32> = List::new();
    static mut ROUTES: List<Route> = List::new();
    static mut ROUTE_TEXT: List<u8> = List::new();
    static mut SCRATCH: List<u8> = List::new();
    /// The methods a path has routes for, when the request's is not one of them.
    static mut ALLOWED: List<u8> = List::new();
    static mut EVENTS: [Event; 64] = [NO_EVENT; 64];
    static mut EVENT_COUNT: usize = 0;
    static mut EVENT_AT: usize = 0;
    static mut STOP: bool = false;
    static mut CHILD: bool = false;
    /// Whether this process only watches over the workers, and answers nothing itself.
    static mut SUPERVISOR: bool = false;
    static mut PARENT_ID: i32 = 0;
    static mut CHILDREN: [i32; MAX_WORKERS] = [0; MAX_WORKERS];
    static mut CHILD_COUNT: usize = 0;
    static mut SWEPT: i64 = 0;
    static mut NEXT_ID: i64 = 0;
    static mut CURRENT: Current = Current {
        id: 0,
        answered: true,
        conn: 0,
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
    };
    static mut DATE: [u8; 29] = [0; 29];
    static mut DATE_AT: i64 = -1;

    extern "C" fn stop(_: i32) {
        unsafe { STOP = true };
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
            c.input.release();
            c.output.release();
        }
        c.fd = -1;
        c.generation = c.generation.wrapping_add(1);
        unsafe {
            FREE.push(index as u32);
            if !ACCEPTING && LISTEN >= 0 {
                // There is room for another connection again.
                ACCEPTING = true;
                sys::listener_on(QUEUE, LISTEN, true);
            }
        }
    }

    #[allow(static_mut_refs)]
    unsafe fn take_connections() {
        loop {
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
                            ACCEPTING = false;
                            sys::listener_on(QUEUE, LISTEN, false);
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
                out.extend(content_type);
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

    /// One line of the log: the request, the status of its answer, and how long the
    /// answer took. `known` says whether the request was read far enough to name.
    #[allow(static_mut_refs)]
    unsafe fn log(status: i64, size: usize, known: bool) {
        unsafe {
            if !LOG {
                return;
            }
            let mut out = Buffer::new();
            if known {
                let c = conn(CURRENT.conn);
                let data = core::slice::from_raw_parts(c.input.ptr, c.input.len);
                let text = |range: Range| core::str::from_utf8(&data[range.start..range.end]).unwrap_or("?");
                let path = text(CURRENT.path);
                let shown = path.char_indices().nth(200).map_or(path.len(), |(at, _)| at);
                let _ = write!(out, "{} {}", text(CURRENT.method), &path[..shown]);
                let micros = (nanos() - CURRENT.at) / 1000;
                let _ = writeln!(out, " {status} {size}B {}.{:03}ms", micros / 1000, micros % 1000);
            } else {
                let _ = writeln!(out, "- - {status} {size}B");
            }
            out.flush(2);
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
        current.at = if unsafe { LOG } { nanos() } else { 0 };
        unsafe {
            NEXT_ID += 1;
            current.id = NEXT_ID;
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
            for index in 0..CONNS.len {
                let c = conn(index);
                let busy = !CURRENT.answered && CURRENT.conn == index;
                if c.fd >= 0 && !busy && (seconds - c.active > TIMEOUT || (c.began != 0 && seconds - c.began > TIMEOUT))
                {
                    shut(index);
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
                if STOP {
                    finish();
                    return 0;
                }
                loop {
                    let mut status = 0;
                    let gone = waitpid(-1, &mut status, WNOHANG);
                    if gone <= 0 {
                        break;
                    }
                    let Some(slot) = CHILDREN[..CHILD_COUNT].iter().position(|child| *child == gone) else {
                        continue;
                    };
                    let child = fork();
                    if child == 0 {
                        become_worker();
                        return 1;
                    }
                    if child > 0 {
                        CHILDREN[slot] = child;
                    } else {
                        CHILD_COUNT -= 1;
                        CHILDREN[slot] = CHILDREN[CHILD_COUNT];
                    }
                }
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
            // The actors are the parent's too, and are reached through its socket.
            super::super::actor::after_fork();
            // The connections are the parent's, which still holds them.
            for index in 0..CONNS.len {
                let c = conn(index);
                if c.fd >= 0 {
                    close(c.fd);
                    c.input.release();
                    c.output.release();
                    c.fd = -1;
                }
            }
            CONNS.len = 0;
            FREE.len = 0;
            READY.len = 0;
            CURRENT.answered = true;
            EVENT_COUNT = 0;
            EVENT_AT = 0;
            if QUEUE >= 0 {
                close(QUEUE);
            }
            start_queue();
            if !sys::watch_parent(QUEUE, PARENT_ID) {
                STOP = true;
            }
        }
    }

    unsafe fn start_queue() {
        unsafe {
            QUEUE = sys::queue();
            ACCEPTING = true;
            sys::watch_listener(QUEUE, LISTEN);
        }
    }

    #[allow(static_mut_refs)]
    unsafe fn event(e: Event) {
        unsafe {
            if e.parent_gone {
                STOP = true;
                return;
            }
            if e.tag == LISTENER {
                take_connections();
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
                flush(index);
                if !live(index) {
                    return;
                }
            }
            if e.readable && conn(index).reading {
                read_from(index);
            } else if e.hangup {
                shut(index);
            }
        }
    }

    #[allow(static_mut_refs)]
    unsafe fn finish() {
        unsafe {
            if !CHILD {
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
                    c.input.release();
                    c.output.release();
                    c.fd = -1;
                }
            }
            CONNS.release();
            FREE.release();
            READY.release();
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
            signal(SIGINT, 0);
            signal(SIGTERM, 0);
            if CHILD {
                koo_flush();
                exit(0);
            }
            STOP = false;
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
            LISTEN = socket(AF_INET, SOCK_STREAM, 0);
            if LISTEN < 0 {
                return failed(b"open a socket for", port);
            }
            let on = 1i32;
            setsockopt(LISTEN, sys::SOL_SOCKET, sys::SO_REUSEADDR, &on, 4);
            let mut address = sys::address(port as u16);
            if bind(LISTEN, &address as *const sys::SockAddr as *const u8, 16) != 0 || listen(LISTEN, 1024) != 0 {
                return failed(b"listen on", port);
            }
            let mut len = 16u32;
            getsockname(LISTEN, &mut address as *mut sys::SockAddr as *mut u8, &mut len);
            let port = u16::from_be(address.port) as i64;
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
            STOP = false;
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
            let cores = sysconf(sys::SC_NPROCESSORS_ONLN).max(1);
            let workers = if workers <= 0 { cores } else { workers }.min(MAX_WORKERS as i64);
            // What is waiting to be printed is printed once, not once by every worker.
            koo_flush();
            for _ in 0..workers {
                let child = fork();
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
            // This process answers nothing: it watches over those that do.
            SUPERVISOR = true;
            port
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
            loop {
                while READY.len > 0 {
                    READY.len -= 1;
                    let index = *READY.ptr.add(READY.len) as usize;
                    let c = conn(index);
                    c.queued = false;
                    if c.fd < 0 {
                        continue;
                    }
                    // A client that does not read its answers is not read from.
                    if c.output.len - c.sent > MAX_PENDING {
                        continue;
                    }
                    match parse(index) {
                        Parsed::Request(-1) => no_route(index),
                        Parsed::Request(number) => return number,
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
                if STOP {
                    finish();
                    return -2;
                }
                sweep();
                EVENT_COUNT = sys::wait(QUEUE, &mut EVENTS, 1000);
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

    /// The value of a name among `name=value` pairs joined by `&`, as it is written.
    fn pair<'a>(pairs: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
        for pair in pairs.split(|b| *b == b'&') {
            let equals = pair.iter().position(|b| *b == b'=').unwrap_or(pair.len());
            if pair[..equals] == *name {
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
            let keep = CURRENT.keep && !conn(index).closing && !STOP;
            Some((index, keep))
        }
    }

    /// The answer is queued: it is written, and the next request is looked for.
    #[allow(static_mut_refs)]
    unsafe fn answered(index: usize) {
        unsafe {
            CURRENT.answered = true;
            pass(index, CURRENT.length);
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
    ) {
        unsafe {
            let Some((index, keep)) = answering(id) else { return };
            // A directory is not a file, though it opens like one.
            let name = c_string(path);
            let directory = opendir(name);
            free(name);
            let file = if directory.is_null() {
                open(path, c"rb")
            } else {
                closedir(directory);
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
        }
    }
}
