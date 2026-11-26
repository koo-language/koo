//! Threads: what each has of its own, and what they wait with.
//!
//! A program starts with one thread. The actors add more (`actor.rs`), so what the
//! support library keeps between two calls of the program (the last error, the number
//! that was just read from JSON) is kept for each thread here, reached through a key of
//! the system's, and what all threads share is behind a `Lock` or a `Gate`.

use super::*;
use core::cell::UnsafeCell;
use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize};

unsafe extern "C" {
    fn pthread_key_create(key: *mut usize, destructor: usize) -> i32;
    fn pthread_getspecific(key: usize) -> *mut u8;
    fn pthread_setspecific(key: usize, value: *const u8) -> i32;
    fn pthread_create(
        thread: *mut usize,
        attributes: *const u8,
        start: extern "C" fn(*mut u8) -> *mut u8,
        argument: *mut u8,
    ) -> i32;
    fn pthread_detach(thread: usize) -> i32;
    fn pthread_attr_init(attributes: *mut u8) -> i32;
    fn pthread_attr_setstacksize(attributes: *mut u8, size: usize) -> i32;
    fn pthread_attr_destroy(attributes: *mut u8) -> i32;
    fn pthread_mutex_init(mutex: *mut u8, attributes: *const u8) -> i32;
    fn pthread_mutex_lock(mutex: *mut u8) -> i32;
    fn pthread_mutex_unlock(mutex: *mut u8) -> i32;
    fn pthread_cond_init(condition: *mut u8, attributes: *const u8) -> i32;
    fn pthread_cond_wait(condition: *mut u8, mutex: *mut u8) -> i32;
    fn pthread_cond_timedwait(condition: *mut u8, mutex: *mut u8, until: *const [i64; 2]) -> i32;
    fn pthread_cond_signal(condition: *mut u8) -> i32;
    fn pthread_cond_broadcast(condition: *mut u8) -> i32;
    fn sched_yield() -> i32;
    fn sysconf(name: i32) -> i64;
}

/// The most threads a program has: the one it starts with and the workers.
pub const MOST_THREADS: usize = 256;

// ---- locks ------------------------------------------------------------------------------

/// A lock that is held for a few instructions, and waited for by trying again.
pub struct Lock(AtomicBool);

impl Default for Lock {
    fn default() -> Lock {
        Lock::new()
    }
}

impl Default for Gate {
    fn default() -> Gate {
        Gate::new()
    }
}

impl Lock {
    pub const fn new() -> Lock {
        Lock(AtomicBool::new(false))
    }

    pub fn lock(&self) {
        let mut tries = 0u32;
        while self.0.swap(true, Acquire) {
            while self.0.load(Relaxed) {
                tries += 1;
                if tries.is_multiple_of(128) {
                    unsafe { sched_yield() };
                } else {
                    core::hint::spin_loop();
                }
            }
        }
    }

    pub fn unlock(&self) {
        self.0.store(false, Release);
    }

    /// Takes the lock if nobody has it. Returns whether it was taken.
    pub fn try_lock(&self) -> bool {
        !self.0.swap(true, Acquire)
    }
}

/// Lets go of the processor for a moment, when trying again has not helped.
pub fn give_way() {
    unsafe { sched_yield() };
}

/// A lock that a thread can sleep at until another wakes it: the system's mutex and
/// condition, in room enough for either on any system this is built for.
#[repr(C, align(16))]
pub struct Gate {
    mutex: UnsafeCell<[u64; 8]>,
    condition: UnsafeCell<[u64; 8]>,
}

unsafe impl Sync for Gate {}

impl Gate {
    pub const fn new() -> Gate {
        Gate {
            mutex: UnsafeCell::new([0; 8]),
            condition: UnsafeCell::new([0; 8]),
        }
    }

    /// Makes the gate usable: before its first use, and again in a forked process,
    /// where a thread that is gone may have held it.
    pub unsafe fn open(&self) {
        unsafe {
            *self.mutex.get() = [0; 8];
            *self.condition.get() = [0; 8];
            pthread_mutex_init(self.mutex.get() as *mut u8, core::ptr::null());
            pthread_cond_init(self.condition.get() as *mut u8, core::ptr::null());
        }
    }

    pub unsafe fn lock(&self) {
        unsafe { pthread_mutex_lock(self.mutex.get() as *mut u8) };
    }

    pub unsafe fn unlock(&self) {
        unsafe { pthread_mutex_unlock(self.mutex.get() as *mut u8) };
    }

    /// Sleeps until woken. The gate is held before and after, and not meanwhile.
    pub unsafe fn wait(&self) {
        unsafe { pthread_cond_wait(self.condition.get() as *mut u8, self.mutex.get() as *mut u8) };
    }

    /// Sleeps until woken, or for some milliseconds. Returns whether the time ran out.
    pub unsafe fn wait_for(&self, milliseconds: i64) -> bool {
        unsafe {
            let mut until = [0i64; 2];
            clock_gettime(0, &mut until);
            // Whole seconds apart from the rest: a time whose second part is a second
            // or more is no time at all, and the wait would return at once.
            let milliseconds = milliseconds.max(0);
            until[0] += milliseconds / 1000;
            until[1] += (milliseconds % 1000) * 1_000_000;
            if until[1] >= 1_000_000_000 {
                until[0] += 1;
                until[1] -= 1_000_000_000;
            }
            pthread_cond_timedwait(self.condition.get() as *mut u8, self.mutex.get() as *mut u8, &until) != 0
        }
    }

    pub unsafe fn wake_one(&self) {
        unsafe { pthread_cond_signal(self.condition.get() as *mut u8) };
    }

    pub unsafe fn wake_all(&self) {
        unsafe { pthread_cond_broadcast(self.condition.get() as *mut u8) };
    }
}

// ---- what a thread has of its own -------------------------------------------------------

/// An actor a thread is in the middle of a message of (`actor.rs`).
#[derive(Clone, Copy)]
pub struct Held {
    pub slot: u32,
    /// Whether the actor's state is out, with the program.
    pub taken: bool,
    /// Whether what is running is the actor's mail: each letter takes the state from
    /// here, and puts it back here without letting go of the actor.
    pub told: bool,
    pub state: *mut Header,
}

/// A place a panic comes back to, and how many actors the thread held there.
#[repr(C)]
pub struct Guard {
    pub buffer: [u64; 64],
    pub mark: usize,
    pub below: *mut Guard,
}

#[repr(C)]
pub struct Thread {
    /// Which thread this is, from 1.
    pub index: u32,
    /// Whether it is one of the threads that run what actors are told.
    pub worker: bool,
    /// The lowest address of the thread's stack, below which is the page that stops it.
    pub stack_low: usize,
    /// The actor this thread is waiting for, as its slot and one more, or 0.
    pub waiting_for: AtomicU32,
    /// Allocations made on this thread, less those freed on it.
    pub live: i64,
    /// Whether the last operation that can fail did, and why.
    pub failed: bool,
    pub at_end: bool,
    pub json_is_int: bool,
    pub last_error: *mut Header,
    pub json_text: *mut Header,
    pub json_int: i64,
    pub json_float: f64,
    pub json_nest: i64,
    /// How deep a reading that begins now already is: more than nothing while a
    /// function of the program reads a part of a text that is being read.
    pub json_base: i64,
    /// Whether what the thread writes as JSON is for another process of the program.
    pub json_wire: bool,
    pub held: *mut Held,
    pub held_len: usize,
    pub held_cap: usize,
    pub guard: *mut Guard,
    pub spare_guard: *mut Guard,
    /// What the thread keeps of its last query of a database (`pg.rs`).
    pub sql: *mut u8,
    /// What the thread keeps of the last answer to a request it made (`client.rs`).
    pub reply: *mut u8,
    /// Bytes from the system's source of randomness that have not been handed out yet
    /// (`codec.rs`).
    pub random: [u8; 256],
    pub random_left: usize,
    /// The kind of the last answer from an actor of another process.
    pub answer_kind: [u8; 104],
    pub answer_kind_len: usize,
    /// Whether the thread is in the middle of talking to another process (`actor.rs`).
    pub talking: bool,
    /// The top of a stack the thread comes back by when its own is exhausted in a
    /// message to an actor, or 0 if it has none.
    pub landing: usize,
    /// The message from another process that the thread is answering: the connection
    /// it came on and one more, turned about once the connection is to be let go of,
    /// or 0 for none; its kind; the message, its length, and where its method ends.
    pub asked_on: i32,
    pub asked_kind: u8,
    pub asked: *mut u8,
    pub asked_len: usize,
    pub asked_method: usize,
}

static KEY: AtomicUsize = AtomicUsize::new(0);
static REGISTRY: Lock = Lock::new();
static THREADS: [AtomicPtr<Thread>; MOST_THREADS] = [const { AtomicPtr::new(core::ptr::null_mut()) }; MOST_THREADS];
static THREAD_COUNT: AtomicUsize = AtomicUsize::new(0);
/// Threads that have been started and have not yet taken their place in the table.
static STARTING: AtomicUsize = AtomicUsize::new(0);

/// Whether the table of threads has room for another, beside those on their way to it
/// and a few kept for the process's own use.
pub fn room() -> bool {
    THREAD_COUNT.load(Relaxed) + STARTING.load(Relaxed) + 4 < MOST_THREADS
}

/// What the calling thread has of its own.
// (A function that goes on to run a program's code, or anything else that asks for the
// thread's record again, keeps what this returns as a pointer, `core::ptr::from_mut(me())`,
// and never takes a `&mut Thread` as a parameter: a parameter is promised to be the only
// way to the record, and what was read through it is not read again after a call that
// changed it, the list of actors held for one.)
#[inline]
pub unsafe fn me<'a>() -> &'a mut Thread {
    unsafe {
        let key = KEY.load(Relaxed);
        if key != 0 {
            let block = pthread_getspecific(key - 1) as *mut Thread;
            if !block.is_null() {
                return &mut *block;
            }
        }
        join()
    }
}

/// The first time a thread asks: its block is made, and the thread is counted.
#[cold]
unsafe fn join<'a>() -> &'a mut Thread {
    unsafe {
        REGISTRY.lock();
        if KEY.load(Relaxed) == 0 {
            let mut key = 0usize;
            if pthread_key_create(&mut key, 0) != 0 {
                fatal(b"koo: threads could not be started\n");
            }
            KEY.store(key + 1, Release);
        }
        let block = calloc(1, core::mem::size_of::<Thread>()) as *mut Thread;
        let index = THREAD_COUNT.load(Relaxed);
        if block.is_null() || index == MOST_THREADS {
            fatal(b"koo: too many threads\n");
        }
        (*block).index = index as u32 + 1;
        THREADS[index].store(block, Release);
        THREAD_COUNT.store(index + 1, Release);
        // One of those that were on their way is here, if any was.
        let _ = STARTING.fetch_update(Relaxed, Relaxed, |starting| starting.checked_sub(1));
        pthread_setspecific(KEY.load(Relaxed) - 1, block as *const u8);
        REGISTRY.unlock();
        &mut *block
    }
}

/// A block that is not any thread's yet: for a handler of a server that has a stack of
/// its own (`fiber.rs`), and with it its own guards, its own last query, and whatever
/// else a thread has of its own. Nothing if there can be no more.
pub unsafe fn spare_block() -> *mut Thread {
    unsafe {
        REGISTRY.lock();
        let index = THREAD_COUNT.load(Relaxed);
        // Not a place that a thread on its way to the table is counting on.
        let block = if index + STARTING.load(Relaxed) >= MOST_THREADS {
            core::ptr::null_mut()
        } else {
            calloc(1, core::mem::size_of::<Thread>()) as *mut Thread
        };
        if !block.is_null() {
            (*block).index = index as u32 + 1;
            THREADS[index].store(block, Release);
            THREAD_COUNT.store(index + 1, Release);
        }
        REGISTRY.unlock();
        block
    }
}

/// Makes a block the calling thread's own, until another is.
pub unsafe fn become_block(block: *mut Thread) {
    unsafe { pthread_setspecific(KEY.load(Relaxed) - 1, block as *const u8) };
}

/// The thread with an index, if it is one of this process's.
pub unsafe fn thread<'a>(index: u32) -> Option<&'a Thread> {
    unsafe {
        if index == 0 || index as usize > THREAD_COUNT.load(Acquire) {
            return None;
        }
        THREADS[index as usize - 1].load(Acquire).as_ref()
    }
}

/// Runs `each` for every thread. Only for when the others are at rest.
pub unsafe fn every(mut each: impl FnMut(&mut Thread)) {
    unsafe {
        for thread in &THREADS[..THREAD_COUNT.load(Acquire)] {
            if let Some(thread) = thread.load(Acquire).as_mut() {
                each(thread);
            }
        }
    }
}

/// In a process just forked: the calling thread is the only one there is.
pub unsafe fn after_fork() {
    unsafe {
        let mine = me() as *mut Thread;
        // What the parent has not handed out of its randomness is the parent's.
        (*mine).random_left = 0;
        REGISTRY.unlock();
        // The threads of the parent are not here: this one is the first of the
        // process, and the places of the others are free again.
        for thread in &THREADS[..THREAD_COUNT.load(Relaxed)] {
            thread.store(core::ptr::null_mut(), Relaxed);
        }
        (*mine).index = 1;
        THREADS[0].store(mine, Release);
        THREAD_COUNT.store(1, Release);
        STARTING.store(0, Release);
    }
}

/// Starts a thread that runs `work` for as long as the program does.
pub unsafe fn start(work: extern "C" fn(*mut u8) -> *mut u8) -> bool {
    unsafe {
        // A thread for which the table has no room is not started: whoever asked for it
        // does without, which every one of them can.
        // (Counted first and looked at after, so that two that ask at once do not both
        // take the last of the room.)
        STARTING.fetch_add(1, Relaxed);
        if THREAD_COUNT.load(Relaxed) + STARTING.load(Relaxed) + 4 > MOST_THREADS {
            STARTING.fetch_sub(1, Relaxed);
            return false;
        }
        let mut attributes = [0u64; 16];
        let attributes = attributes.as_mut_ptr() as *mut u8;
        pthread_attr_init(attributes);
        // As much stack as the thread the program starts on has.
        pthread_attr_setstacksize(attributes, 8 << 20);
        let mut thread = 0usize;
        let made = pthread_create(&mut thread, attributes, work, core::ptr::null_mut()) == 0;
        pthread_attr_destroy(attributes);
        if made {
            pthread_detach(thread);
        } else {
            STARTING.fetch_sub(1, Relaxed);
        }
        made
    }
}

// ---- a stack that runs out ---------------------------------------------------------------
//
// A function that calls itself too deeply, or without end, runs off the end of its
// thread's stack, into a page that the system keeps unreadable, and the system sends the
// process a signal. The signal is taken here, on a small stack of its own (the thread's
// has no room), and reported as the panic it is (Section 5.9).

unsafe extern "C" {
    fn sigaltstack(new: *const SignalStack, old: *mut SignalStack) -> i32;
    fn sigaction(signal: i32, new: *const SignalAction, old: *mut SignalAction) -> i32;
    fn pthread_self() -> usize;
    fn _exit(status: i32) -> !;
    #[cfg(target_os = "macos")]
    fn pthread_get_stackaddr_np(thread: usize) -> usize;
    #[cfg(target_os = "macos")]
    fn pthread_get_stacksize_np(thread: usize) -> usize;
    #[cfg(target_os = "linux")]
    fn pthread_getattr_np(thread: usize, attributes: *mut u8) -> i32;
    #[cfg(target_os = "linux")]
    fn pthread_attr_getstack(attributes: *const u8, low: *mut usize, size: *mut usize) -> i32;
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct SignalStack {
    base: *mut u8,
    size: usize,
    flags: i32,
}

#[cfg(not(target_os = "macos"))]
#[repr(C)]
struct SignalStack {
    base: *mut u8,
    flags: i32,
    size: usize,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct SignalAction {
    handler: usize,
    mask: u32,
    flags: i32,
}

#[cfg(not(target_os = "macos"))]
#[repr(C)]
struct SignalAction {
    handler: usize,
    mask: [u64; 16],
    flags: i32,
    restorer: usize,
}

/// On the signal's own stack, and with what the signal was about.
const ON_STACK_WITH_INFO: i32 = if cfg!(target_os = "macos") {
    0x0001 | 0x0040
} else {
    0x0800_0000 | 0x0004
};
const SEGV: i32 = 11;
const BUS: i32 = if cfg!(target_os = "macos") { 10 } else { 7 };
/// Where in what the system says of a signal the address is that could not be reached.
const ADDRESS_AT: usize = if cfg!(target_os = "macos") { 24 } else { 16 };
/// How far below a stack an address still counts as having run off it: a function
/// with large variables may step well past the page that stops it.
const BELOW: usize = 64 << 20;

/// Finds the calling thread's stack and gives the thread a stack to take the signal on.
/// The handler is the process's, and is set by whichever thread comes here first.
pub unsafe fn watch_stack() {
    unsafe {
        let thread = me();
        #[cfg(target_os = "macos")]
        {
            let this = pthread_self();
            thread.stack_low = pthread_get_stackaddr_np(this) - pthread_get_stacksize_np(this);
        }
        #[cfg(target_os = "linux")]
        {
            let mut attributes = [0u64; 16];
            let (mut low, mut size) = (0usize, 0usize);
            if pthread_getattr_np(pthread_self(), attributes.as_mut_ptr() as *mut u8) == 0 {
                pthread_attr_getstack(attributes.as_ptr() as *const u8, &mut low, &mut size);
                pthread_attr_destroy(attributes.as_mut_ptr() as *mut u8);
            }
            // The stack of the thread a program starts on grows as it is used, up to
            // the limit the process was given, and not every C library says how far
            // that is: one of them says only how far it has grown so far.
            unsafe extern "C" {
                fn getpid() -> i32;
                fn syscall(number: i64, ...) -> i64;
                fn getrlimit(resource: i32, limit: *mut [u64; 2]) -> i32;
            }
            unsafe extern "C" {
                fn getauxval(kind: u64) -> u64;
                fn strlen(text: *const u8) -> usize;
            }
            let own_number = if cfg!(target_arch = "aarch64") { 178 } else { 186 };
            let mut limit = [0u64; 2];
            if low != 0
                && syscall(own_number) == getpid() as i64
                && getrlimit(3, &mut limit) == 0
                && limit[0] != u64::MAX
            {
                // The limit is counted from the very top of the stack, where the system
                // put the program's arguments and environment and, last of all, the
                // name it was run by (what the system knows as 31): above what a C
                // library may call the top.
                let name = getauxval(31) as usize;
                let mut top = low + size;
                if name != 0 {
                    top = top.max((name + strlen(name as *const u8) + 1 + 8 + 4095) & !4095);
                }
                if (limit[0] as usize) < top {
                    low = low.min(top - limit[0] as usize);
                }
            }
            thread.stack_low = low;
        }
        if thread.stack_low == 0 {
            return;
        }
        const SIZE: usize = 64 << 10;
        let base = malloc(SIZE);
        if base.is_null() {
            return;
        }
        let stack = SignalStack {
            base,
            size: SIZE,
            flags: 0,
        };
        sigaltstack(&stack, core::ptr::null_mut());
        let mut action: SignalAction = core::mem::zeroed();
        action.handler = stack_signal as *const () as usize;
        action.flags = ON_STACK_WITH_INFO;
        sigaction(SEGV, &action, core::ptr::null_mut());
        sigaction(BUS, &action, core::ptr::null_mut());
    }
}

/// A signal that says memory could not be reached. If the address is just below the
/// thread's stack, the stack ran out, and that is a panic. Anything else is not the
/// program's doing: the signal is given back to the system, which ends the process as
/// it would have.
/// The thread that is running, as a number that no other thread has.
pub fn this_thread() -> usize {
    unsafe { pthread_self() }
}

/// Where the stack pointer and the place in the program are kept in what a signal is
/// given of the thread it interrupted, and whether that is found through a pointer,
/// for each system and processor the support library knows; measured with `offsetof`
/// on each. Elsewhere there is nothing, and a thread that runs out of stack ends its
/// process.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SAVED: Option<(Option<usize>, usize, usize)> = Some((Some(48), 264, 272));
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const SAVED: Option<(Option<usize>, usize, usize)> = Some((None, 432, 440));
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SAVED: Option<(Option<usize>, usize, usize)> = Some((None, 160, 168));
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64")
)))]
const SAVED: Option<(Option<usize>, usize, usize)> = None;

/// Makes the thread that a signal interrupted go on at `function`, on a stack whose
/// top is `top`, when the signal has been dealt with. Returns whether it could.
unsafe fn resume_at(context: *const u8, function: usize, top: usize) -> bool {
    unsafe {
        let Some((through, stack_at, place_at)) = SAVED else {
            return false;
        };
        if context.is_null() {
            return false;
        }
        let saved = match through {
            Some(at) => *(context.add(at) as *const *mut u8),
            None => context as *mut u8,
        };
        if saved.is_null() {
            return false;
        }
        // A function is entered with the stack as a call leaves it: on x86-64 that is
        // eight bytes below a multiple of sixteen, where the address to return to
        // would be.
        let top = if cfg!(target_arch = "x86_64") { top - 8 } else { top };
        *(saved.add(stack_at) as *mut usize) = top;
        *(saved.add(place_at) as *mut usize) = function;
        true
    }
}

extern "C" fn stack_signal(signal: i32, info: *const u8, context: *const u8) {
    unsafe {
        let address = *(info.add(ADDRESS_AT) as *const usize);
        let low = me().stack_low;
        if low != 0 && address < low + 4096 && address + BELOW >= low {
            let free = overflowed();
            // A handler of a server comes back to where it was called, as from any
            // panic, by way of a stack that has room: the signal is done with first,
            // and the thread goes on from there. Not if it holds something that the
            // rest of the process would wait for without end: it was printing, or
            // talking to another process, when its stack ran out.
            if free
                && crate::actor::quiet()
                && let Some((function, top)) = crate::http::landing().or_else(|| crate::actor::landing())
                && resume_at(context, function, top)
            {
                return;
            }
            // A handler of a server that ran out of stack takes its process with it,
            // but not without an answer to the request it was answering.
            crate::http::answer_before_ending();
            crate::actor::remove_socket();
            _exit(101);
        }
        let action: SignalAction = core::mem::zeroed();
        sigaction(signal, &action, core::ptr::null_mut());
    }
}

/// How many processors the program may run on: those the machine has, or fewer where
/// the program is held to fewer, as it is in a container that is allowed so much
/// processor time and no more. A part of a processor counts as a whole one.
pub fn processors() -> usize {
    let name = if cfg!(target_os = "macos") { 58 } else { 84 };
    let count = unsafe { sysconf(name) };
    let count = if count < 1 { 1 } else { count as usize };
    count.min(allowed()).max(1)
}

/// How many processors the system lets the program use at a time, on Linux: the
/// processors it may run on, and the time its control group is allowed, which is
/// written as so many microseconds in each period of so many.
#[cfg(target_os = "linux")]
fn allowed() -> usize {
    unsafe extern "C" {
        fn sched_getaffinity(process: i32, size: usize, mask: *mut u8) -> i32;
        fn open(path: *const u8, flags: i32, ...) -> i32;
        fn read(fd: i32, buffer: *mut u8, count: usize) -> isize;
        fn close(fd: i32) -> i32;
    }
    /// The numbers in a small file of the system's, where `max` stands for no limit.
    unsafe fn numbers(path: &core::ffi::CStr) -> Option<[i64; 2]> {
        unsafe {
            let fd = open(path.as_ptr() as *const u8, 0);
            if fd < 0 {
                return None;
            }
            let mut text = [0u8; 64];
            let len = read(fd, text.as_mut_ptr(), text.len());
            close(fd);
            let text = core::str::from_utf8(&text[..len.max(0) as usize]).ok()?;
            let mut found = [-1i64; 2];
            for (at, word) in text.split_ascii_whitespace().take(2).enumerate() {
                found[at] = if word == "max" { -1 } else { word.parse().ok()? };
            }
            Some(found)
        }
    }
    unsafe {
        let mut most = usize::MAX;
        let mut mask = [0u8; 1024];
        if sched_getaffinity(0, mask.len(), mask.as_mut_ptr()) == 0 {
            let may: usize = mask.iter().map(|byte| byte.count_ones() as usize).sum();
            if may > 0 {
                most = may;
            }
        }
        // The second version of control groups has both numbers in one file, and the
        // first has a file for each.
        let (time, period) = match numbers(c"/sys/fs/cgroup/cpu.max") {
            Some([time, period]) => (time, period),
            None => {
                let one = |path| numbers(path).map_or(-1, |found| found[0]);
                let time = one(c"/sys/fs/cgroup/cpu/cpu.cfs_quota_us");
                (time, one(c"/sys/fs/cgroup/cpu/cpu.cfs_period_us"))
            }
        };
        if time > 0 && period > 0 {
            most = most.min(((time + period - 1) / period) as usize);
        }
        // Where the program sees the control groups of its whole machine, its own is
        // further in, at the path the system gives for the process: `0::/path`.
        let mut own = [0u8; 512];
        let fd = open(c"/proc/self/cgroup".as_ptr() as *const u8, 0);
        if fd >= 0 {
            let len = read(fd, own.as_mut_ptr(), own.len()).max(0) as usize;
            close(fd);
            let path = own[..len]
                .split(|byte| *byte == b'\n')
                .find_map(|line| line.strip_prefix(b"0::"))
                .filter(|path| path.len() > 1 && path.len() < 400 && !path.contains(&0));
            if let Some(path) = path {
                let mut file = [0u8; 448];
                let (before, after) = (b"/sys/fs/cgroup", b"/cpu.max\0");
                file[..before.len()].copy_from_slice(before);
                file[before.len()..before.len() + path.len()].copy_from_slice(path);
                let end = before.len() + path.len();
                file[end..end + after.len()].copy_from_slice(after);
                if let Ok(file) = core::ffi::CStr::from_bytes_until_nul(&file)
                    && let Some([time, period]) = numbers(file)
                    && time > 0
                    && period > 0
                {
                    most = most.min(((time + period - 1) / period) as usize);
                }
            }
        }
        most
    }
}

#[cfg(not(target_os = "linux"))]
fn allowed() -> usize {
    usize::MAX
}
