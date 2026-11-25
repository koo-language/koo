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
            until[1] += milliseconds * 1_000_000;
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
    pub held: *mut Held,
    pub held_len: usize,
    pub held_cap: usize,
    pub guard: *mut Guard,
    pub spare_guard: *mut Guard,
    /// The kind of the last answer from an actor of another process.
    pub answer_kind: [u8; 104],
    pub answer_kind_len: usize,
}

static KEY: AtomicUsize = AtomicUsize::new(0);
static REGISTRY: Lock = Lock::new();
static THREADS: [AtomicPtr<Thread>; MOST_THREADS] = [const { AtomicPtr::new(core::ptr::null_mut()) }; MOST_THREADS];
static THREAD_COUNT: AtomicUsize = AtomicUsize::new(0);

/// What the calling thread has of its own.
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
        pthread_setspecific(KEY.load(Relaxed) - 1, block as *const u8);
        REGISTRY.unlock();
        &mut *block
    }
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
        REGISTRY.unlock();
        for thread in &THREADS[..THREAD_COUNT.load(Relaxed)] {
            if thread.load(Relaxed) != mine {
                thread.store(core::ptr::null_mut(), Relaxed);
            }
        }
    }
}

/// Starts a thread that runs `work` for as long as the program does.
pub unsafe fn start(work: extern "C" fn(*mut u8) -> *mut u8) -> bool {
    unsafe {
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
        }
        made
    }
}

/// How many processors the program may run on.
pub fn processors() -> usize {
    let name = if cfg!(target_os = "macos") { 58 } else { 84 };
    let count = unsafe { sysconf(name) };
    if count < 1 { 1 } else { count as usize }
}
