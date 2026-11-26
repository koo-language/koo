//! Actors (Section 7.3).
//!
//! An actor owns its state and answers one message at a time. Its state lives in the
//! process that started it, in a slot of the table below, and whichever thread has a
//! message for it holds the slot while the message runs: nothing else can reach the
//! state meanwhile, so nothing of it is copied or counted atomically.
//!
//! * To ask (`handle.method(...)`) is to take the slot, call the method on the state,
//!   and let go: on the asker's own thread, at about the cost of a call when nobody else
//!   wants the actor. An asker that finds the actor busy waits for it. If the wait can
//!   never end, because the thread that has the actor is itself waiting for one this
//!   thread has, that is the cycle of Section 7.3, and it panics.
//! * To tell (`handle.send.method(...)`) is to leave a letter in the actor's mail and go
//!   on. The worker threads, one for each processor, take the actors that have mail and
//!   run it, so that actors that were told run side by side. Letters are run in the
//!   order they were left, and before the actor is next asked.
//!
//! What crosses from one actor to another (arguments, answers) is frozen first
//! (`lib.rs`): it is never changed again, and so it is safe that two threads read it.
//!
//! Another process, such as a worker of a server (`http.rs`), reaches an actor through
//! the socket of the process that owns it. Its message is the arguments as text; the
//! owner reads it, asks the actor as it would itself, and sends the answer back.
//!
//! A panic in a message that nobody waits for on the same thread (one that was told, or
//! one from another process) ends the message and not the program: the place to come
//! back to was marked, the actors the message was in the middle of start again from the
//! state they started with (Section 7.4), and what the abandoned calls held is not freed.

use super::thread::{Gate, Guard, Held, Lock, Thread, me};
use super::*;
use core::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release, SeqCst};
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize};

unsafe extern "C" {
    fn _longjmp(buffer: *mut u64, value: i32) -> !;
    fn nanosleep(wanted: *const [i64; 2], left: *mut [i64; 2]) -> i32;
}

/// Restarts allowed in a minute before an actor stays stopped (Section 7.4).
const RESTARTS: usize = 5;
/// How many letters a worker runs for one actor before it lets an actor that waits
/// have a turn.
const BATCH: usize = 256;
/// How many times a thread looks again before it sleeps.
const LOOKS: u32 = 2000;

// What a slot's `word` says.
/// A thread is in the middle of a message of the actor, or is running its mail.
const LOCKED: u32 = 1;
/// There may be letters.
const MAIL: u32 = 2;
/// The actor is in the queue of the workers.
const QUEUED: u32 = 4;
/// A thread sleeps until the actor is let go of.
const WAITERS: u32 = 8;
const ALIVE: u32 = 16;
/// A thread sleeps until there is room in the actor's mail.
const FULL: u32 = 32;
/// How many letters may wait for an actor that does not say (Section 7.3).
const MAILBOX: u32 = 1024;

/// An actor. Everything after `holder` is touched only by the thread that has the slot
/// locked, but for `mail`, which anyone adds to.
#[repr(C)]
struct Slot {
    word: AtomicU32,
    /// Counts the actors the slot has held, so that the handle of one that stopped is
    /// not taken for the handle of the next.
    generation: AtomicU32,
    /// The thread that has the slot locked, or 0.
    holder: AtomicU32,
    /// Which of `KINDS` is the name of the actor's type.
    kind: u32,
    /// The state while no message is using it, and the state the actor starts with.
    /// Each is a list of one element, of the program's own type, which only the program
    /// can free.
    state: *mut Header,
    initial: *mut Header,
    /// Letters as they were left, the newest first.
    mail: AtomicPtr<Job>,
    /// Letters taken from `mail`, the oldest first.
    inbox: *mut Job,
    /// How many letters are in `mail`, and the most there may be: a thread that would
    /// leave one more waits until whoever runs the mail has taken what is there.
    waiting: AtomicU32,
    room: u32,
}

/// A letter: what was told to an actor. It is a function value of the program's, as
/// `koo-compiler/src/llvm/closures.rs` lays it out, that takes the actor's state, calls
/// the method, and puts the state back. The first two things it captured are a word for
/// the chain of letters it waits in, and the number of the actor it is for
/// (`koo-compiler/src/derive.rs` sees to that).
#[repr(C)]
pub struct Job {
    rc: i64,
    code: usize,
    free: Option<unsafe extern "C" fn(*mut Job)>,
    next: *mut Job,
    id: i64,
}

const CHUNK_BITS: usize = 12;
const CHUNKS: usize = 1 << 14;
const MOST_KINDS: usize = 1024;

/// The slots, in chunks that never move, so that a slot can be read while another
/// thread adds one.
static TABLE: [AtomicPtr<Slot>; CHUNKS] = [const { AtomicPtr::new(core::ptr::null_mut()) }; CHUNKS];
static SLOT_COUNT: AtomicUsize = AtomicUsize::new(0);
/// Held to add a slot, a kind, or a note that an actor failed.
static MAKING: Lock = Lock::new();
static mut FREE: Table<u32> = Table::new();
/// The names of the actor types, each as the program's string.
static KINDS: [AtomicPtr<Header>; MOST_KINDS] = [const { AtomicPtr::new(core::ptr::null_mut()) }; MOST_KINDS];
static KIND_COUNT: AtomicUsize = AtomicUsize::new(0);
/// The actors that have panicked, and when: `(number, times)`.
static mut FAILED_ACTORS: Table<(i64, [i64; RESTARTS])> = Table::new();
/// Whether a panic has been come back from: what it abandoned was not freed.
static LEAKED: AtomicBool = AtomicBool::new(false);
static mut SCAN: usize = 0;

// The workers.
/// 0 until the first letter, 1 while the workers are being started, then 2.
static POOL: AtomicU32 = AtomicU32::new(0);
/// What runs a letter: a function of the program's, which marks the place a panic comes
/// back to and calls the function value.
static RUNNER: AtomicUsize = AtomicUsize::new(0);
static WORKERS: AtomicUsize = AtomicUsize::new(0);
/// The workers that wait for room in a mailbox at this moment, and the threads that
/// were ever started to stand in for such workers: there are never more of the second
/// than there were of the first at one time.
static WAITING_WORKERS: AtomicUsize = AtomicUsize::new(0);
static STAND_INS: AtomicUsize = AtomicUsize::new(0);
/// The actors that wait for a worker, and what is held to add or take one.
static mut RUN: Ring = Ring::new();
static RUNNING: Lock = Lock::new();
static QUEUE_LEN: AtomicUsize = AtomicUsize::new(0);
/// How many actors the workers have taken from the queue, ever.
static TAKEN: AtomicUsize = AtomicUsize::new(0);
/// Where workers sleep when there is nothing to run, and how many do.
static QUEUE: Gate = Gate::new();
static SLEEPERS: AtomicUsize = AtomicUsize::new(0);
/// Workers that are awake with nothing to do, looking for something.
static LOOKING: AtomicUsize = AtomicUsize::new(0);
/// Whether one of the sleeping workers wakes now and then to see that nothing is stuck.
static WATCHED: AtomicBool = AtomicBool::new(false);
/// Whether actors wait while every worker is busy, and the monitor is awake to see how
/// long: where it sleeps otherwise.
static PRESSED: AtomicBool = AtomicBool::new(false);
static MONITOR: Gate = Gate::new();

// Waiting for a busy actor.
static GATES: AtomicU32 = AtomicU32::new(0);
static BENCHES: [Gate; 64] = [const { Gate::new() }; 64];
/// Held to say which actor a thread waits for, and to look for a cycle of waits.
static WAITS: Lock = Lock::new();

/// What a program built to be checked for races gives the support library, to say
/// where one thread hands something to another: the support library is not itself
/// watched by the checker, which would otherwise take every hand-over for a race.
static HANDING: AtomicUsize = AtomicUsize::new(0);
static HANDED: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_sync_hooks(acquired: usize, releasing: usize) {
    HANDED.store(acquired, Release);
    HANDING.store(releasing, Release);
}

/// Says that what is at an address is about to be let go of, for another thread.
#[inline]
pub(crate) unsafe fn handing<T>(what: *mut T) {
    let hook = HANDING.load(Relaxed);
    if hook != 0 {
        let hook: extern "C" fn(*mut u8) = unsafe { core::mem::transmute(hook) };
        hook(what as *mut u8);
    }
}

/// Says that what is at an address is this thread's now.
#[inline]
pub(crate) unsafe fn handed<T>(what: *mut T) {
    let hook = HANDED.load(Relaxed);
    if hook != 0 {
        let hook: extern "C" fn(*mut u8) = unsafe { core::mem::transmute(hook) };
        hook(what as *mut u8);
    }
}

struct Table<T> {
    ptr: *mut T,
    len: usize,
    cap: usize,
}

impl<T: Copy> Table<T> {
    const fn new() -> Table<T> {
        Table {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }

    unsafe fn push(&mut self, value: T) {
        unsafe {
            if self.len == self.cap {
                let cap = (self.cap * 2).max(64);
                self.ptr = realloc(self.ptr as *mut u8, cap * core::mem::size_of::<T>()) as *mut T;
                if self.ptr.is_null() {
                    fatal(b"koo: out of memory\n");
                }
                self.cap = cap;
            }
            self.ptr.add(self.len).write(value);
        }
        self.len += 1;
    }

    fn at(&mut self, index: usize) -> &mut T {
        unsafe { &mut *self.ptr.add(index) }
    }
}

/// The actors that wait for a worker, first in first out.
struct Ring {
    ptr: *mut u32,
    cap: usize,
    head: usize,
    len: usize,
}

impl Ring {
    const fn new() -> Ring {
        Ring {
            ptr: core::ptr::null_mut(),
            cap: 0,
            head: 0,
            len: 0,
        }
    }

    unsafe fn push(&mut self, value: u32) {
        unsafe {
            if self.len == self.cap {
                let cap = (self.cap * 2).max(1024);
                let grown = malloc(cap * 4) as *mut u32;
                if grown.is_null() {
                    fatal(b"koo: out of memory\n");
                }
                for at in 0..self.len {
                    grown.add(at).write(*self.ptr.add((self.head + at) % self.cap));
                }
                free(self.ptr as *mut u8);
                (self.ptr, self.cap, self.head) = (grown, cap, 0);
            }
            self.ptr.add((self.head + self.len) % self.cap).write(value);
        }
        self.len += 1;
    }

    unsafe fn pop(&mut self) -> u32 {
        unsafe {
            let value = *self.ptr.add(self.head);
            self.head = (self.head + 1) % self.cap;
            self.len -= 1;
            value
        }
    }
}

unsafe fn kind_name<'a>(kind: usize) -> &'a [u8] {
    unsafe {
        if kind < KIND_COUNT.load(Acquire) {
            bytes(KINDS[kind].load(Acquire))
        } else {
            b"?"
        }
    }
}

unsafe fn slot_at(index: u32) -> *mut Slot {
    unsafe {
        TABLE[index as usize >> CHUNK_BITS]
            .load(Acquire)
            .add(index as usize & ((1 << CHUNK_BITS) - 1))
    }
}

fn index_of(id: i64) -> u32 {
    (id & 0xffff_ffff) as u32
}

fn generation_of(id: i64) -> u32 {
    (id >> 32) as u32
}

/// The slot of a handle, if the actor it names is still the one there.
unsafe fn slot_of(id: i64) -> Option<*mut Slot> {
    unsafe {
        if id < 0 || index_of(id) as usize >= SLOT_COUNT.load(Acquire) {
            return None;
        }
        let slot = slot_at(index_of(id));
        ((*slot).word.load(Acquire) & ALIVE != 0 && (*slot).generation.load(Acquire) == generation_of(id))
            .then_some(slot)
    }
}

/// Ends the program as a panic does (Section 5.9), or the message, if it is guarded.
pub unsafe fn die(parts: &[&[u8]]) -> ! {
    let mut out = Buffer::new();
    out.push(b"panic: ");
    for part in parts {
        out.push(part);
    }
    let said = out.len;
    out.push(b"\n");
    unsafe {
        crate::log::said(b"error", &out.data[..out.len], &out.data[..said], &[]);
        leave()
    }
}

/// What a panic does once it has said what it has to say.
pub unsafe fn leave() -> ! {
    unsafe {
        let guard = me().guard;
        if !guard.is_null() {
            _longjmp((*guard).buffer.as_mut_ptr(), 1);
        }
        exit(101)
    }
}

/// Whether the leak check at the end of `main` can still be trusted.
pub unsafe fn leaked() -> bool {
    LEAKED.load(Relaxed)
}

/// Makes the gates usable, once.
unsafe fn gates() {
    unsafe {
        loop {
            match GATES.compare_exchange(0, 1, AcqRel, Acquire) {
                Ok(_) => {
                    QUEUE.open();
                    MONITOR.open();
                    for bench in &BENCHES {
                        bench.open();
                    }
                    #[cfg(any(target_os = "macos", target_os = "linux"))]
                    remote::open();
                    GATES.store(2, Release);
                    return;
                }
                Err(2) => return,
                Err(_) => core::hint::spin_loop(),
            }
        }
    }
}

// ---- holding an actor -------------------------------------------------------------------

unsafe fn hold(me: *mut Thread, held: Held) {
    unsafe {
        if (*me).held_len == (*me).held_cap {
            let cap = ((*me).held_cap * 2).max(16);
            (*me).held = realloc((*me).held as *mut u8, cap * core::mem::size_of::<Held>()) as *mut Held;
            if (*me).held.is_null() {
                fatal(b"koo: out of memory\n");
            }
            (*me).held_cap = cap;
        }
        (*me).held.add((*me).held_len).write(held);
        (*me).held_len += 1;
    }
}

unsafe fn top<'a>(me: *mut Thread) -> Option<&'a mut Held> {
    unsafe {
        if (*me).held_len == 0 {
            None
        } else {
            Some(&mut *(*me).held.add((*me).held_len - 1))
        }
    }
}

/// Whether waiting for `slot` would be waiting for ever: the thread that has it waits
/// for an actor, whose thread waits for another, and so round to this thread.
unsafe fn waits_for_itself(mine: u32, mut slot: *mut Slot) -> bool {
    unsafe {
        for _ in 0..=thread::MOST_THREADS {
            let holder = (*slot).holder.load(Acquire);
            if holder == mine {
                return true;
            }
            let Some(other) = thread::thread(holder) else {
                return false;
            };
            let wanted = other.waiting_for.load(Acquire);
            if wanted == 0 {
                return false;
            }
            slot = slot_at(wanted - 1);
        }
        false
    }
}

/// Locks an actor for this thread, waiting for whoever has it. `what` says why, for the
/// panic if the actor is gone or the wait could not end.
unsafe fn lock(slot: *mut Slot, id: i64, what: &[u8], me: *mut Thread) {
    unsafe {
        let word = (*slot).word.load(Relaxed);
        if word & LOCKED == 0
            && (*slot)
                .word
                .compare_exchange(word, word | LOCKED, Acquire, Relaxed)
                .is_ok()
        {
            (*slot).holder.store((*me).index, Release);
            handed(slot);
            return;
        }
        let mut looks = 0;
        loop {
            let word = (*slot).word.load(Acquire);
            if word & ALIVE == 0 || (*slot).generation.load(Acquire) != generation_of(id) {
                (*me).waiting_for.store(0, Release);
                stopped(what);
            }
            if word & LOCKED == 0 {
                if (*slot)
                    .word
                    .compare_exchange(word, word | LOCKED, Acquire, Relaxed)
                    .is_ok()
                {
                    (*me).waiting_for.store(0, Release);
                    (*slot).holder.store((*me).index, Release);
                    handed(slot);
                    return;
                }
                continue;
            }
            if (*slot).holder.load(Acquire) == (*me).index {
                cycle(slot, what);
            }
            looks += 1;
            if looks < LOOKS {
                core::hint::spin_loop();
                continue;
            }
            // Before it sleeps, the thread says what it waits for, so that the thread
            // that would close a cycle of waits finds it. That thread may be this one.
            gates();
            WAITS.lock();
            (*me).waiting_for.store(index_of(id) + 1, Release);
            let for_ever = waits_for_itself((*me).index, slot);
            if for_ever {
                (*me).waiting_for.store(0, Release);
            }
            WAITS.unlock();
            if for_ever {
                cycle(slot, what);
            }
            if (*slot)
                .word
                .compare_exchange(word, word | WAITERS, AcqRel, Relaxed)
                .is_err()
            {
                continue;
            }
            let bench = &BENCHES[index_of(id) as usize % BENCHES.len()];
            bench.lock();
            while (*slot).word.load(Acquire) & (LOCKED | WAITERS) == LOCKED | WAITERS {
                bench.wait();
            }
            bench.unlock();
        }
    }
}

unsafe fn stopped(what: &[u8]) -> ! {
    unsafe { die(&[b"`", what, b"` was asked of an actor that is stopped (ActorStopped)"]) }
}

unsafe fn cycle(slot: *mut Slot, what: &[u8]) -> ! {
    unsafe {
        die(&[
            b"`",
            kind_name((*slot).kind as usize),
            b"` is asked `",
            what,
            b"` while it waits for the answer to this very message: it would wait for ever. \
              Use `send` on one side.",
        ])
    }
}

/// Lets go of an actor. If it has mail, a worker is told.
unsafe fn unlock(slot: *mut Slot, index: u32) {
    unsafe {
        handing(slot);
        (*slot).holder.store(0, Release);
        let mut word = (*slot).word.load(Relaxed);
        let again = loop {
            let again = word & MAIL != 0 && word & QUEUED == 0;
            let mut new = word & !(LOCKED | WAITERS);
            if again {
                new |= QUEUED;
            }
            match (*slot).word.compare_exchange_weak(word, new, AcqRel, Relaxed) {
                Ok(_) => break again,
                Err(now) => word = now,
            }
        };
        if word & WAITERS != 0 {
            let bench = &BENCHES[index as usize % BENCHES.len()];
            bench.lock();
            bench.wake_all();
            bench.unlock();
        }
        if again {
            enqueue(index);
        }
    }
}

// ---- mail -------------------------------------------------------------------------------

/// The letters that were left for an actor this thread has locked, the oldest first, or
/// null if there are none.
unsafe fn fresh_mail(slot: *mut Slot) -> *mut Job {
    unsafe {
        let mut newest = (*slot).mail.swap(core::ptr::null_mut(), Acquire);
        if newest.is_null() {
            // None: say so, and look once more, for a letter left in between.
            (*slot).word.fetch_and(!MAIL, AcqRel);
            newest = (*slot).mail.swap(core::ptr::null_mut(), Acquire);
            if newest.is_null() {
                return newest;
            }
            (*slot).word.fetch_or(MAIL, AcqRel);
        }
        let mut oldest = core::ptr::null_mut();
        while !newest.is_null() {
            let next = (*newest).next;
            (*newest).next = oldest;
            oldest = newest;
            newest = next;
        }
        oldest
    }
}

/// Wakes the threads that wait for room in an actor's mail.
unsafe fn make_room(slot: *mut Slot) {
    unsafe {
        if (*slot).word.load(Acquire) & FULL != 0 {
            (*slot).word.fetch_and(!FULL, AcqRel);
            let bench = bench_of(slot);
            bench.lock();
            bench.wake_all();
            bench.unlock();
        }
    }
}

/// Where threads sleep that wait for an actor: one of a few places, by where the actor is.
unsafe fn bench_of(slot: *mut Slot) -> &'static Gate {
    &BENCHES[(slot as usize / core::mem::size_of::<Slot>()) % BENCHES.len()]
}

/// A worker is about to sleep until there is room in a mailbox. Another is started in
/// its place, so that the actor it waits for has a thread to run on however many
/// workers are waiting. Returns whether there is one: there is a most to how many
/// threads a program has.
unsafe fn stand_in() -> bool {
    unsafe {
        if WORKERS.load(SeqCst) >= thread::MOST_THREADS - 8 || !thread::start(work) {
            return false;
        }
        WORKERS.fetch_add(1, SeqCst);
        true
    }
}

/// Gives up a function value of the program's that the support library was given.
pub unsafe fn drop_function(function: *mut Header) {
    unsafe { drop_job(function as *mut Job) }
}

/// Gives up a function value of the program's.
unsafe fn drop_job(job: *mut Job) {
    unsafe {
        if release(job as *mut Header)
            && let Some(free) = (*job).free
        {
            free(job);
        }
    }
}

/// Runs the letters of an actor this thread has locked, and holds last. The state is
/// kept with the thread meanwhile, where each letter takes it from and puts it back, so
/// that the slot, which other threads write their letters to, is left alone. Stops when
/// there are no more, or after `most` if there is other work. A letter for an actor that
/// is gone is thrown away.
unsafe fn run_mail(slot: *mut Slot, me: *mut Thread, most: usize) {
    unsafe {
        // The thread's own record is reached afresh each time, and never kept as a
        // reference: a letter that is run holds actors of its own, and the list of what
        // the thread holds is moved when it grows.
        let at = (*me).held_len - 1;
        (*(*me).held.add(at)).state = core::mem::replace(&mut (*slot).state, core::ptr::null_mut());
        (*(*me).held.add(at)).told = true;
        let generation = (*slot).generation.load(Acquire);
        let alive = (*slot).word.load(Acquire) & ALIVE != 0;
        let runner = RUNNER.load(Acquire);
        let mut inbox = core::mem::replace(&mut (*slot).inbox, core::ptr::null_mut());
        let mut ran = 0;
        loop {
            if inbox.is_null() {
                inbox = fresh_mail(slot);
                if inbox.is_null() {
                    break;
                }
            }
            let job = inbox;
            inbox = (*job).next;
            handed(job);
            // A letter waits until it is run: only then is there room for another, and
            // the threads that waited for room are told.
            (*slot).waiting.fetch_sub(1, AcqRel);
            make_room(slot);
            // An actor that panicked once too often in a letter has no state to go on with.
            let wanted = alive
                && runner != 0
                && generation_of((*job).id) == generation
                && !(*(*me).held.add(at)).state.is_null();
            if wanted {
                // The program's function owns the letter from here, and lets go of it.
                let run: extern "C" fn(*mut Job) = core::mem::transmute(runner);
                run(job);
            } else {
                drop_job(job);
            }
            ran += 1;
            if ran >= most && QUEUE_LEN.load(Relaxed) > 0 {
                break;
            }
        }
        (*slot).inbox = inbox;
        (*slot).state = (*(*me).held.add(at)).state;
        (*(*me).held.add(at)).state = core::ptr::null_mut();
    }
}

/// The function of the program that runs a letter, or 0 if the program has not said.
pub fn runner() -> usize {
    RUNNER.load(Acquire)
}

/// Says which function of the program runs a letter. Called as the program starts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_actor_runner(runner: usize) {
    RUNNER.store(runner, Release);
}

/// Tells an actor: leaves it a letter, which becomes the support library's. If the
/// actor's mail is full, the thread waits until there is room, which is what keeps a
/// sender from getting ahead of an actor without end (Section 7.3). A letter for an
/// actor that is stopped is thrown away.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_actor_tell(id: i64, job: *mut Job, method: *const Header) {
    unsafe {
        leave_letter(id, job, Some(bytes(method)));
    }
}

/// Tells an actor if there is room in its mail, and says whether there was.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_actor_try_tell(id: i64, job: *mut Job) -> bool {
    unsafe { leave_letter(id, job, None) }
}

/// Leaves a letter. With the name of the method, it waits for room; without, it gives
/// up, throws the letter away and returns false.
unsafe fn leave_letter(id: i64, job: *mut Job, wait: Option<&[u8]>) -> bool {
    unsafe {
        let Some(slot) = slot_of(id) else {
            drop_job(job);
            return wait.is_some();
        };
        while (*slot).waiting.fetch_add(1, AcqRel) >= (*slot).room {
            (*slot).waiting.fetch_sub(1, AcqRel);
            let Some(method) = wait else {
                drop_job(job);
                return false;
            };
            let me = core::ptr::from_mut(me());
            // The thread that would make room may be waiting for this one: then the
            // wait would never end, and it is a panic. An actor that tells itself is
            // the plainest case, since only it could make room.
            gates();
            WAITS.lock();
            (*me).waiting_for.store(index_of(id) + 1, Release);
            let for_ever = waits_for_itself((*me).index, slot);
            if for_ever {
                (*me).waiting_for.store(0, Release);
            }
            WAITS.unlock();
            if for_ever {
                drop_job(job);
                die(&[
                    b"`",
                    kind_name((*slot).kind as usize),
                    b"` cannot be told `",
                    method,
                    b"`: its mailbox is full, and what would make room in it is waiting for this message \
                      to end. Give the actor a larger `@mailbox`, or send less at a time.",
                ]);
            }
            // A worker that waits leaves another in its place, if the workers that
            // wait are more than the threads started for that so far: a thread for
            // each that waits at one time, and not one for every letter that waited.
            // If there can be no other, the letter is left all the same: a mailbox
            // over its limit is better than a program that stands still.
            if (*me).worker {
                let waiting = WAITING_WORKERS.fetch_add(1, SeqCst) + 1;
                let covered = waiting <= STAND_INS.load(SeqCst)
                    || (stand_in() && {
                        STAND_INS.fetch_add(1, SeqCst);
                        true
                    });
                if !covered {
                    WAITING_WORKERS.fetch_sub(1, SeqCst);
                    (*me).waiting_for.store(0, Release);
                    (*slot).waiting.fetch_add(1, AcqRel);
                    break;
                }
            }
            (*slot).word.fetch_or(FULL, AcqRel);
            let bench = bench_of(slot);
            bench.lock();
            while (*slot).waiting.load(Acquire) >= (*slot).room
                && (*slot).word.load(Acquire) & (FULL | ALIVE) == FULL | ALIVE
            {
                bench.wait();
            }
            bench.unlock();
            if (*me).worker {
                WAITING_WORKERS.fetch_sub(1, SeqCst);
            }
            (*me).waiting_for.store(0, Release);
            if slot_of(id).is_none() {
                drop_job(job);
                return true;
            }
        }
        (*job).id = id;
        handing(job);
        let mut newest = (*slot).mail.load(Relaxed);
        loop {
            (*job).next = newest;
            match (*slot).mail.compare_exchange_weak(newest, job, Release, Relaxed) {
                Ok(_) => break,
                Err(now) => newest = now,
            }
        }
        // Whoever has the actor sees the mail when it lets go. If nobody has it, and it
        // is not waiting for a worker already, it goes to the workers now.
        let mut word = (*slot).word.load(Relaxed);
        loop {
            let queue = word & (LOCKED | QUEUED) == 0;
            let new = word | MAIL | if queue { QUEUED } else { 0 };
            if new == word {
                return true;
            }
            match (*slot).word.compare_exchange_weak(word, new, AcqRel, Relaxed) {
                Ok(_) => {
                    if queue {
                        enqueue(index_of(id));
                    }
                    return true;
                }
                Err(now) => word = now,
            }
        }
    }
}

// ---- the workers ------------------------------------------------------------------------

/// Starts the workers, the first time there is something for them to do.
#[cold]
unsafe fn start_workers() -> bool {
    unsafe {
        gates();
        if POOL.compare_exchange(0, 1, AcqRel, Acquire).is_err() {
            return true;
        }
        let wanted = getenv(c"KOO_THREADS".as_ptr() as *const u8);
        let wanted = if wanted.is_null() {
            None
        } else {
            core::str::from_utf8(c_text(wanted))
                .ok()
                .and_then(|text| text.parse::<usize>().ok())
        };
        let count = wanted
            .unwrap_or_else(thread::processors)
            .clamp(1, thread::MOST_THREADS - 8);
        let mut started = 0;
        for _ in 0..count {
            if thread::start(work) {
                started += 1;
            }
        }
        if started == 0 {
            // Not one could be started, as where a container is allowed no more
            // threads: whoever tells an actor runs what it told, until one can.
            POOL.store(0, Release);
            return false;
        }
        WORKERS.store(started, SeqCst);
        // Without a monitor a method that runs for ever keeps a worker, and with enough
        // of them nothing else runs; that is all that is lost if it cannot be started.
        thread::start(monitor);
        POOL.store(2, Release);
        true
    }
}

/// Wakes a worker that sleeps.
unsafe fn wake() {
    unsafe {
        QUEUE.lock();
        QUEUE.wake_one();
        QUEUE.unlock();
    }
}

/// Wakes the monitor, if it sleeps: actors wait, and no worker is free to take them.
unsafe fn press() {
    unsafe {
        if !PRESSED.swap(true, SeqCst) {
            MONITOR.lock();
            MONITOR.wake_one();
            MONITOR.unlock();
        }
    }
}

#[allow(static_mut_refs)]
unsafe fn enqueue(index: u32) {
    unsafe {
        let pool = POOL.load(Acquire) == 2 || start_workers();
        RUNNING.lock();
        RUN.push(index);
        let len = RUN.len;
        QUEUE_LEN.store(len, SeqCst);
        RUNNING.unlock();
        if !pool {
            run_what_waits();
            return;
        }
        if LOOKING.load(SeqCst) > 0 {
            // A worker that is looking will find it.
            return;
        }
        if SLEEPERS.load(SeqCst) == 0 {
            // Every worker is busy. The monitor sees to it that the actor does not
            // wait for long on one that is busy for long.
            press();
            return;
        }
        // Waking a worker costs more than most letters do. One letter left by a worker
        // is most often run by that worker, as soon as it is done with the one it is
        // running: nobody is woken for it, so long as a sleeper is watching in case the
        // worker is not done soon.
        if len == 1 && me().worker && WATCHED.load(SeqCst) {
            return;
        }
        wake();
    }
}

/// The next actor with mail. A worker looks a while before it sleeps: a letter is often
/// answered by another, and waking a thread costs more than the letter does.
#[allow(static_mut_refs)]
unsafe fn dequeue() -> u32 {
    unsafe {
        let mut looks = 0;
        // Whether this worker has just stopped being the one that watches: another is
        // woken to watch in its place.
        let mut watched = false;
        LOOKING.fetch_add(1, SeqCst);
        loop {
            if QUEUE_LEN.load(Relaxed) > 0 {
                RUNNING.lock();
                if RUN.len > 0 {
                    let index = RUN.pop();
                    let more = RUN.len;
                    QUEUE_LEN.store(more, SeqCst);
                    RUNNING.unlock();
                    TAKEN.fetch_add(1, Relaxed);
                    let looking = LOOKING.fetch_sub(1, SeqCst) - 1;
                    if SLEEPERS.load(SeqCst) > 0 && ((more > 0 && looking == 0) || watched) {
                        wake();
                    } else if more > 0 && looking == 0 {
                        // This worker was the last that was free, and actors still wait.
                        press();
                    }
                    return index;
                }
                RUNNING.unlock();
            }
            looks += 1;
            if looks < LOOKS {
                core::hint::spin_loop();
                continue;
            }
            // Nothing: sleep. One sleeper watches while any worker is awake: it wakes
            // every millisecond to see whether an actor has waited in the queue all that
            // time, as happens when the worker that would have run it is busy for long.
            LOOKING.fetch_sub(1, SeqCst);
            QUEUE.lock();
            SLEEPERS.fetch_add(1, SeqCst);
            let mut watching = false;
            loop {
                let waiting = QUEUE_LEN.load(SeqCst);
                if waiting > 0 && !watching {
                    break;
                }
                let idle = waiting == 0 && SLEEPERS.load(SeqCst) == WORKERS.load(SeqCst);
                if idle && watching {
                    WATCHED.store(false, SeqCst);
                    watching = false;
                }
                if !idle && !watching {
                    watching = WATCHED.compare_exchange(false, true, SeqCst, SeqCst).is_ok();
                }
                if !watching {
                    QUEUE.wait();
                    continue;
                }
                let seen = TAKEN.load(Relaxed);
                let woken = !QUEUE.wait_for(1);
                if QUEUE_LEN.load(SeqCst) > 0 && (woken || TAKEN.load(Relaxed) == seen) {
                    break;
                }
            }
            if watching {
                WATCHED.store(false, SeqCst);
                watched = true;
            }
            SLEEPERS.fetch_sub(1, SeqCst);
            QUEUE.unlock();
            LOOKING.fetch_add(1, SeqCst);
            looks = 0;
        }
    }
}

/// The monitor: while actors wait with every worker busy, it looks every ten
/// milliseconds, and if in that time no worker has come free to take one, it starts
/// another worker. So a method that computes for long, or for ever, costs the program a
/// thread and not its other actors (Section 13.5).
extern "C" fn monitor(_: *mut u8) -> *mut u8 {
    unsafe {
        // It takes its place among the threads, which is counted on.
        let _ = me();
        loop {
            MONITOR.lock();
            while !PRESSED.load(SeqCst) {
                MONITOR.wait();
            }
            MONITOR.unlock();
            let mut seen = TAKEN.load(Relaxed);
            loop {
                let (wanted, mut left) = ([0i64, 10_000_000], [0i64; 2]);
                nanosleep(&wanted, &mut left);
                if QUEUE_LEN.load(SeqCst) == 0 {
                    PRESSED.store(false, SeqCst);
                    // An actor may have come to wait just as the monitor decided to sleep.
                    if QUEUE_LEN.load(SeqCst) == 0 || PRESSED.swap(true, SeqCst) {
                        break;
                    }
                }
                let taken = TAKEN.load(Relaxed);
                if taken == seen && SLEEPERS.load(SeqCst) == 0 && LOOKING.load(SeqCst) == 0 {
                    stand_in();
                }
                seen = taken;
            }
        }
    }
}

/// Gives the calling thread a stack to come back by when its own is exhausted in the
/// middle of a message: the message then ends as it does for any panic.
unsafe fn landing_stack() {
    unsafe {
        const SIZE: usize = 64 << 10;
        let base = malloc(SIZE);
        if !base.is_null() {
            me().landing = (base as usize + SIZE) & !15;
        }
    }
}

/// Where a thread that ran out of stack in a message to an actor goes on, if it has a
/// stack for it and a place a panic comes back to: the function to go on in, and the
/// top of the stack to run it on.
pub unsafe fn landing() -> Option<(usize, usize)> {
    unsafe {
        let me = core::ptr::from_mut(me());
        if (*me).landing == 0 || (*me).guard.is_null() {
            return None;
        }
        Some((landed as extern "C" fn() -> ! as usize, (*me).landing))
    }
}

/// The panic has been said; what is left is what any panic in a message does.
extern "C" fn landed() -> ! {
    unsafe { leave() }
}

/// A worker: takes an actor with mail, runs it, and lets go.
extern "C" fn work(_: *mut u8) -> *mut u8 {
    unsafe {
        let me = core::ptr::from_mut(me());
        (*me).worker = true;
        thread::watch_stack();
        landing_stack();
        loop {
            let index = dequeue();
            serve(index, me);
        }
    }
}

/// Whether the thread is running what waited for a thread that could not be started.
static CATCHING_UP: AtomicUsize = AtomicUsize::new(0);

/// Where no thread could be started for what actors are told, runs what is waiting on
/// the calling thread, if it is one that may: it holds no actor, which a letter might
/// ask and find held by its own thread, and it is not already here, further up. A
/// thread that may not leaves the letters where they are, for the next that may: the
/// process that watches a server comes by ten times a second, and a program comes by
/// when its `main` has returned. So many at a time and no more, so that an actor that
/// tells itself without end does not keep the thread for ever.
#[allow(static_mut_refs)]
pub unsafe fn run_what_waits() {
    unsafe {
        let me: *mut Thread = me();
        if POOL.load(Acquire) == 2 || (*me).held_len > 0 || CATCHING_UP.load(Acquire) == (*me).index as usize {
            return;
        }
        if CATCHING_UP
            .compare_exchange(0, (*me).index as usize, AcqRel, Acquire)
            .is_err()
        {
            return;
        }
        // (With a stack to come back by, as the threads this one stands in for have:
        // a told method that calls itself without end is then a panic in its actor.)
        if (*me).landing == 0 {
            landing_stack();
        }
        for _ in 0..4096 {
            RUNNING.lock();
            if RUN.len == 0 {
                RUNNING.unlock();
                break;
            }
            let waiting = RUN.pop();
            QUEUE_LEN.store(RUN.len, SeqCst);
            RUNNING.unlock();
            serve(waiting, me);
        }
        CATCHING_UP.store(0, Release);
    }
}

/// Runs what waits for a thread, where none could be started and something waits.
/// Asked where a program would otherwise wait in vain: before it asks an actor, and
/// before it sleeps.
#[inline]
pub unsafe fn waiting_mail() {
    unsafe {
        if POOL.load(Relaxed) == 0 && QUEUE_LEN.load(Relaxed) > 0 {
            run_what_waits();
        }
    }
}

/// Runs the mail of an actor that was waiting for a thread, if nobody has the actor.
unsafe fn serve(index: u32, me: *mut Thread) {
    unsafe {
        let slot = slot_at(index);
        let mut word = (*slot).word.fetch_and(!QUEUED, AcqRel) & !QUEUED;
        // Whoever has the actor runs its mail, or sends it back here when it lets go.
        let mine = loop {
            if word & LOCKED != 0 {
                break false;
            }
            match (*slot)
                .word
                .compare_exchange_weak(word, word | LOCKED, Acquire, Relaxed)
            {
                Ok(_) => break true,
                Err(now) => word = now,
            }
        };
        if !mine {
            return;
        }
        (*slot).holder.store((*me).index, Release);
        handed(slot);
        hold(
            &mut *me,
            Held {
                slot: index,
                taken: false,
                told: true,
                state: core::ptr::null_mut(),
            },
        );
        run_mail(slot, me, BATCH);
        (*me).held_len -= 1;
        unlock(slot, index);
    }
}

/// Waits until every letter has been run: nothing waits for a worker, and every worker
/// sleeps. `main` has returned, so nothing new is told but by the letters themselves.
unsafe fn settle() {
    unsafe {
        if POOL.load(Acquire) == 0 {
            // No thread ever ran what actors were told: what is left is run here.
            loop {
                RUNNING.lock();
                let left = RUN.len;
                RUNNING.unlock();
                if left == 0 {
                    return;
                }
                run_what_waits();
            }
        }
        loop {
            QUEUE.lock();
            let still = QUEUE_LEN.load(SeqCst) == 0 && SLEEPERS.load(SeqCst) == WORKERS.load(SeqCst);
            QUEUE.unlock();
            if still {
                return;
            }
            let (wanted, mut left) = ([0i64, 50_000], [0i64; 2]);
            nanosleep(&wanted, &mut left);
        }
    }
}

// ---- what the program asks for ----------------------------------------------------------

/// Makes an actor of a kind from the state it starts with, and returns its number.
#[unsafe(no_mangle)]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn koo_actor_new(kind: *const Header, initial: *mut Header, mailbox: i64) -> i64 {
    unsafe {
        let name = bytes(kind);
        MAKING.lock();
        let kinds = KIND_COUNT.load(Relaxed);
        let mut known = (0..kinds).find(|index| bytes(KINDS[*index].load(Relaxed)) == name);
        if known.is_none() {
            if kinds == MOST_KINDS {
                fatal(b"koo: too many kinds of actor\n");
            }
            let kept = new_str(name);
            // The name is the support library's own from here on.
            uncounted();
            (*kept).rc = IMMORTAL;
            KINDS[kinds].store(kept, Release);
            KIND_COUNT.store(kinds + 1, Release);
            known = Some(kinds);
        }
        let index = if FREE.len > 0 {
            FREE.len -= 1;
            *FREE.at(FREE.len)
        } else {
            let index = SLOT_COUNT.load(Relaxed);
            let chunk = index >> CHUNK_BITS;
            if chunk == CHUNKS {
                fatal(b"koo: too many actors\n");
            }
            if TABLE[chunk].load(Relaxed).is_null() {
                let slots = calloc(1 << CHUNK_BITS, core::mem::size_of::<Slot>()) as *mut Slot;
                if slots.is_null() {
                    fatal(b"koo: out of memory\n");
                }
                TABLE[chunk].store(slots, Release);
            }
            SLOT_COUNT.store(index + 1, Release);
            index as u32
        };
        MAKING.unlock();
        let slot = slot_at(index);
        // A slot that held another actor may be in a worker's hands still, which is
        // throwing away what was told to that one.
        loop {
            let word = (*slot).word.load(Relaxed);
            if word & LOCKED == 0
                && (*slot)
                    .word
                    .compare_exchange_weak(word, word | LOCKED, Acquire, Relaxed)
                    .is_ok()
            {
                break;
            }
            core::hint::spin_loop();
        }
        handed(slot);
        // Once as the state, once as what to start again from.
        retain(initial);
        retain(initial);
        (*slot).state = initial;
        (*slot).initial = initial;
        (*slot).kind = known.unwrap_or(0) as u32;
        (*slot).room = if mailbox > 0 {
            mailbox.min(u32::MAX as i64 / 2) as u32
        } else {
            MAILBOX
        };
        (*slot).word.fetch_or(ALIVE, AcqRel);
        unlock(slot, index);
        (((*slot).generation.load(Relaxed) as i64) << 32) | index as i64
    }
}

/// Takes an actor's state out, to answer a message with: the actor is this thread's
/// until the state is put back. What was told to it before runs first.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_actor_take(id: i64, method: *const Header) -> *mut Header {
    unsafe {
        // What was told is run before an actor is asked: where there is no thread to
        // have run it, it is run now.
        waiting_mail();
        let me = core::ptr::from_mut(me());
        let index = index_of(id);
        // A letter being run: its actor is this thread's already, and its state is here.
        if let Some(top) = top(me)
            && top.slot == index
            && top.told
            && !top.taken
        {
            top.taken = true;
            return core::mem::replace(&mut top.state, core::ptr::null_mut());
        }
        let Some(slot) = slot_of(id) else {
            stopped(bytes(method))
        };
        lock(slot, id, bytes(method), me);
        hold(
            me,
            Held {
                slot: index,
                taken: false,
                told: false,
                state: core::ptr::null_mut(),
            },
        );
        if (*slot).word.load(Acquire) & MAIL != 0 {
            run_mail(slot, me, usize::MAX);
        }
        // The actor may have stopped while this thread waited for it, or have panicked
        // once too often in what it was told.
        if (*slot).word.load(Acquire) & ALIVE == 0 || (*slot).generation.load(Acquire) != generation_of(id) {
            (*me).held_len -= 1;
            unlock(slot, index);
            stopped(bytes(method));
        }
        if let Some(top) = top(me) {
            (top.taken, top.told) = (true, false);
        }
        core::mem::replace(&mut (*slot).state, core::ptr::null_mut())
    }
}

/// Puts an actor's state back, the message answered. The state becomes the slot's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_actor_put(id: i64, state: *mut Header) {
    unsafe {
        let me = core::ptr::from_mut(me());
        let index = index_of(id);
        let Some(top) = top(me) else {
            fatal(b"koo runtime: a state was put back that was not out\n")
        };
        if top.slot != index || !top.taken {
            fatal(b"koo runtime: a state was put back that was not out\n");
        }
        top.taken = false;
        if top.told {
            top.state = state;
            return;
        }
        let slot = slot_at(index);
        (*slot).state = state;
        (*me).held_len -= 1;
        // Nobody waits and nothing was told, most often: one exchange lets go.
        handing(slot);
        (*slot).holder.store(0, Release);
        let word = (*slot).word.load(Relaxed);
        if word & (MAIL | WAITERS) == 0
            && (*slot)
                .word
                .compare_exchange(word, word & !LOCKED, Release, Relaxed)
                .is_ok()
        {
            return;
        }
        unlock(slot, index);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_actor_alive(id: i64) -> bool {
    unsafe { slot_of(id).is_some() }
}

/// Stops an actor, in two steps: its state is handed over (`which` 0), then the state
/// it would have started again from (`which` 1), each for the program to free. What was
/// told to it and has not run is forgotten.
#[unsafe(no_mangle)]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn koo_actor_end(id: i64, which: i64) -> *mut Header {
    unsafe {
        let me = core::ptr::from_mut(me());
        let index = index_of(id);
        if which == 0 {
            let Some(slot) = slot_of(id) else {
                die(&[b"an actor was stopped twice"]);
            };
            if (*slot).holder.load(Acquire) == (*me).index {
                die(&[
                    b"`",
                    kind_name((*slot).kind as usize),
                    b"` was stopped from inside one of its own messages",
                ]);
            }
            lock(slot, id, b"stop", me);
            if (*slot).word.load(Acquire) & ALIVE == 0 || (*slot).generation.load(Acquire) != generation_of(id) {
                unlock(slot, index);
                die(&[b"an actor was stopped twice"]);
            }
            hold(
                me,
                Held {
                    slot: index,
                    taken: false,
                    told: false,
                    state: core::ptr::null_mut(),
                },
            );
            let mut inbox = core::mem::replace(&mut (*slot).inbox, core::ptr::null_mut());
            loop {
                if inbox.is_null() {
                    inbox = fresh_mail(slot);
                    if inbox.is_null() {
                        break;
                    }
                }
                let job = inbox;
                inbox = (*job).next;
                // A letter that is thrown away waits no more than one that was run:
                // the next actor to be kept here starts with all its room.
                (*slot).waiting.fetch_sub(1, AcqRel);
                drop_job(job);
            }
            return core::mem::replace(&mut (*slot).state, core::ptr::null_mut());
        }
        let slot = slot_at(index);
        let initial = core::mem::replace(&mut (*slot).initial, core::ptr::null_mut());
        (*slot).generation.fetch_add(1, AcqRel);
        (*slot).word.fetch_and(!ALIVE, AcqRel);
        make_room(slot);
        MAKING.lock();
        FREE.push(index);
        MAKING.unlock();
        (*me).held_len -= 1;
        unlock(slot, index);
        initial
    }
}

/// Begins a walk over the actors, the last started first. `main` has returned, and the
/// walk is to stop them: first, what was told is run.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_actor_scan() {
    unsafe {
        settle();
        SCAN = SLOT_COUNT.load(Acquire);
    }
}

/// The next actor of a kind in the walk, or -1 when there is no more.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_actor_any(kind: *const Header) -> i64 {
    unsafe {
        let name = bytes(kind);
        while SCAN > 0 {
            SCAN -= 1;
            let slot = slot_at(SCAN as u32);
            let word = (*slot).word.load(Acquire);
            if word & ALIVE != 0 && word & LOCKED == 0 && kind_name((*slot).kind as usize) == name {
                return (((*slot).generation.load(Acquire) as i64) << 32) | SCAN as i64;
            }
        }
        -1
    }
}

// ---- panics that end a message, not the program -------------------------------------

/// Marks a place a panic comes back to, and returns where the mark is kept: the program
/// calls `_setjmp` on it, since what it marks is a place in the program's own code.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_guard_enter() -> *mut u64 {
    unsafe {
        let me = core::ptr::from_mut(me());
        let guard = if (*me).spare_guard.is_null() {
            let guard = malloc(core::mem::size_of::<Guard>()) as *mut Guard;
            if guard.is_null() {
                fatal(b"koo: out of memory\n");
            }
            guard
        } else {
            let guard = (*me).spare_guard;
            (*me).spare_guard = (*guard).below;
            guard
        };
        (*guard).mark = (*me).held_len;
        (*guard).below = (*me).guard;
        (*me).guard = guard;
        (*guard).buffer.as_mut_ptr()
    }
}

/// Forgets the last mark: what it guarded ended without a panic.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_guard_done() {
    unsafe {
        let me = core::ptr::from_mut(me());
        let guard = (*me).guard;
        if guard.is_null() {
            return;
        }
        (*me).guard = (*guard).below;
        (*guard).below = (*me).spare_guard;
        (*me).spare_guard = guard;
    }
}

/// An actor that panicked starts again from the state it started with, which is
/// returned. One that has done so too often stays stopped, and there is none.
#[allow(static_mut_refs)]
unsafe fn restart(slot: *mut Slot, index: u32) -> *mut Header {
    unsafe {
        let id = (((*slot).generation.load(Acquire) as i64) << 32) | index as i64;
        let mut time = [0i64; 2];
        clock_gettime(0, &mut time);
        MAKING.lock();
        let mut known = (0..FAILED_ACTORS.len).find(|entry| FAILED_ACTORS.at(*entry).0 == id);
        if known.is_none() {
            FAILED_ACTORS.push((id, [0; RESTARTS]));
            known = Some(FAILED_ACTORS.len - 1);
        }
        let times = &mut FAILED_ACTORS.at(known.unwrap_or(0)).1;
        let often = times.iter().all(|at| time[0] - at < 60);
        times.rotate_left(1);
        times[RESTARTS - 1] = time[0];
        MAKING.unlock();
        let mut out = Buffer::new();
        out.push(b"koo: the actor `");
        out.push(kind_name((*slot).kind as usize));
        let state = if often {
            // What it started with stays where it is, for the end of the program to forget.
            (*slot).generation.fetch_add(1, AcqRel);
            (*slot).word.fetch_and(!ALIVE, AcqRel);
            make_room(slot);
            out.push(b"` panicked too often, and stays stopped\n");
            core::ptr::null_mut()
        } else {
            retain((*slot).initial);
            out.push(b"` panicked, and starts again\n");
            (*slot).initial
        };
        // Without the `koo: ` it begins with and the newline it ends with, for a log
        // that is JSON.
        let message = &out.data[5..out.len - 1];
        let fields = [(&b"actor"[..], crate::log::Field::Text(kind_name((*slot).kind as usize)))];
        crate::log::said(b"error", &out.data[..out.len], message, &fields);
        state
    }
}

/// After a panic has come back to the last mark: the actors that the thread took since
/// the mark start again, and are let go of.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn koo_actor_recover() {
    unsafe {
        LEAKED.store(true, Relaxed);
        let me = core::ptr::from_mut(me());
        (*me).waiting_for.store(0, Release);
        (*me).json_wire = false;
        (*me).json_base = 0;
        crate::pg::abandon();
        let mark = if (*me).guard.is_null() { 0 } else { (*(*me).guard).mark };
        koo_guard_done();
        while (*me).held_len > mark {
            (*me).held_len -= 1;
            let held = *(*me).held.add((*me).held_len);
            let slot = slot_at(held.slot);
            if held.taken {
                (*slot).state = restart(slot, held.slot);
            } else if held.told {
                (*slot).state = held.state;
            }
            unlock(slot, held.slot);
        }
        // The letter that panicked was of the actor below the mark, which whoever runs
        // its mail goes on holding.
        if let Some(top) = top(me)
            && top.told
            && top.taken
        {
            top.state = restart(slot_at(top.slot), top.slot);
            top.taken = false;
        }
    }
}

/// In a process just forked: the threads of its parent are not there, and what they
/// held is nobody's.
#[allow(static_mut_refs)]
unsafe fn alone_after_fork() {
    unsafe {
        MAKING.unlock();
        WAITS.unlock();
        GATES.store(0, Release);
        RUNNING.unlock();
        POOL.store(0, Release);
        WORKERS.store(0, SeqCst);
        LOOKING.store(0, SeqCst);
        SLEEPERS.store(0, SeqCst);
        WATCHED.store(false, SeqCst);
        PRESSED.store(false, SeqCst);
        RUN.len = 0;
        RUN.head = 0;
        QUEUE_LEN.store(0, Release);
        threads_after_fork();
    }
}

// ---- other processes ----------------------------------------------------------------

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub use alone::*;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use remote::*;

/// On a system where processes do not talk yet, every actor is in the one process.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod alone {
    use super::*;

    pub unsafe fn after_fork() {}
    pub unsafe fn pending(_: i32) -> bool {
        false
    }
    pub unsafe fn at_exit() {}
    pub unsafe fn let_go_of_old() {}
    pub extern "C" fn remove_socket() {}
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_home() -> *mut Header {
        unsafe { new_str(b"here") }
    }
    pub unsafe fn before_fork() {}
    pub fn quiet() -> bool {
        true
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_parent() -> *mut Header {
        unsafe { new_str(b"") }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_elsewhere() -> bool {
        false
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_local(_: *const Header) -> bool {
        true
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_message() -> *mut Header {
        unsafe { new_str(b"") }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_body() -> *mut Header {
        unsafe { new_str(b"") }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_reply(_: *const Header, _: *const Header) {}
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_hang_up() {}

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_answerer(_: usize) {}

    pub unsafe fn forking(_: bool) {}
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_ask(_: *const Header, _: *const Header, _: *const Header) -> *mut Header {
        unsafe { die(&[b"an actor of another process was asked, on a system where processes do not talk yet"]) }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_kind() -> *mut Header {
        unsafe { new_str(b"") }
    }
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_send(_: *const Header, _: *const Header, _: *const Header) {}
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod remote {
    use super::*;
    use core::fmt::Write;

    unsafe extern "C" {
        fn socket(domain: i32, kind: i32, protocol: i32) -> i32;
        fn bind(fd: i32, address: *const u8, len: u32) -> i32;
        fn listen(fd: i32, backlog: i32) -> i32;
        fn accept(fd: i32, address: *mut u8, len: *mut u32) -> i32;
        fn close(fd: i32) -> i32;
        fn getpid() -> i32;
        fn poll(fds: *mut PollFd, count: usize, milliseconds: i32) -> i32;
        fn unlink(path: *const u8) -> i32;
        fn signal(number: i32, handler: usize) -> usize;
        fn recv(fd: i32, data: *mut u8, len: usize, flags: i32) -> isize;
        fn shutdown(fd: i32, how: i32) -> i32;
        fn fcntl(fd: i32, command: i32, ...) -> i32;
        fn pipe(fds: *mut i32) -> i32;
        fn read(fd: i32, data: *mut u8, len: usize) -> isize;
    }

    const MSG_PEEK: i32 = 2;
    const MSG_DONTWAIT: i32 = if cfg!(target_os = "macos") { 0x80 } else { 0x40 };
    const EAGAIN: i32 = if cfg!(target_os = "macos") { 35 } else { 11 };
    const AF_UNIX: i32 = 1;
    const SOCK_STREAM: i32 = 1;
    const POLLIN: i16 = 0x1;
    const POLLOUT: i16 = 0x4;
    const EINTR: i32 = 4;
    const SIGPIPE: i32 = 13;
    const ASK: u8 = 0;
    const SEND: u8 = 1;
    /// The most processes that may talk to this one at a time, and the most this one
    /// may talk to.
    const MOST_CLIENTS: usize = 4096;
    const MOST_HOMES: usize = 256;
    const NAME: usize = 104;

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    #[cfg(target_os = "macos")]
    #[repr(C)]
    struct Address {
        len: u8,
        family: u8,
        path: [u8; 104],
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct Address {
        family: u16,
        path: [u8; 108],
    }

    /// The address of a process's socket for the system. On Linux the name is one that
    /// exists in no directory, so that nothing needs a place to write; on macOS it is a
    /// file where temporary files go.
    #[cfg(target_os = "macos")]
    fn address(name: &[u8]) -> (Address, u32) {
        let mut address = Address {
            len: 106,
            family: AF_UNIX as u8,
            path: [0; 104],
        };
        let len = name.len().min(103);
        address.path[..len].copy_from_slice(&name[..len]);
        (address, 106)
    }

    #[cfg(target_os = "linux")]
    fn address(name: &[u8]) -> (Address, u32) {
        let mut address = Address {
            family: AF_UNIX as u16,
            path: [0; 108],
        };
        let len = name.len().min(106);
        address.path[1..1 + len].copy_from_slice(&name[..len]);
        (address, 3 + len as u32)
    }

    #[derive(Clone, Copy)]
    struct Name {
        bytes: [u8; NAME],
        len: usize,
    }

    const NO_NAME: Name = Name {
        bytes: [0; NAME],
        len: 0,
    };

    impl Name {
        fn of(text: &[u8]) -> Name {
            let mut name = NO_NAME;
            name.len = text.len().min(NAME);
            name.bytes[..name.len].copy_from_slice(&text[..name.len]);
            name
        }

        fn text(&self) -> &[u8] {
            &self.bytes[..self.len]
        }
    }

    // What this process is to others: where it is reached, and who is talking to it.
    static HOME: AtomicPtr<Header> = AtomicPtr::new(core::ptr::null_mut());
    static LISTENER: core::sync::atomic::AtomicI32 = core::sync::atomic::AtomicI32::new(-1);

    /// What is being done on a connection from another process: nothing;
    const IDLE: u8 = 0;
    /// a thread is answering the message that came on it;
    const BUSY: u8 = 1;
    /// its message was read, and waits until its actor has answered another;
    const HELD_UP: u8 = 2;
    /// or the other end is gone, and the thread that answers on it has yet to let go;
    const GONE: u8 = 3;
    /// or its message is answered and the answer is still going out, to an asker that
    /// takes it slowly: `out` is what is left of it. Its actor has gone on to the next.
    const SENDING: u8 = 4;

    #[derive(Clone, Copy)]
    struct Conn {
        fd: i32,
        doing: u8,
        /// The actor its message asks, or `NO_ACTOR` for a message that keeps none.
        actor: i64,
        /// Its place among the messages that are held up.
        turn: u64,
        /// The message: while it is held up, and before that while it is still
        /// arriving, when `got` says how much of it, and of the four bytes before it
        /// that say its length, is here.
        message: *mut u8,
        len: usize,
        got: usize,
        head: [u8; 4],
        /// Whether more arrived on it while its message was being answered or held
        /// up. It is not watched again until that message is done with: what is there
        /// stays there, and to be told of it again and again would be to spin.
        early: bool,
        /// What is left of an answer that is going out, how long it is, and how much of
        /// it has gone.
        out: *mut u8,
        out_len: usize,
        out_sent: usize,
    }

    const NO_CONN: Conn = Conn {
        fd: -1,
        doing: IDLE,
        actor: NO_ACTOR,
        turn: 0,
        message: core::ptr::null_mut(),
        len: 0,
        got: 0,
        head: [0; 4],
        early: false,
        out: core::ptr::null_mut(),
        out_len: 0,
        out_sent: 0,
    };

    /// The connections, and whose turn it is. `SERVED` is held to read or change them:
    /// for a few instructions, and never while a thread waits for anything.
    static SERVED: Lock = Lock::new();
    static mut CONNS: [Conn; MOST_CLIENTS] = [NO_CONN; MOST_CLIENTS];
    static mut CONN_COUNT: usize = 0;
    static mut TURN: usize = 0;
    static mut TURNS: u64 = 0;
    /// The two ends of a pipe: the thread that waits for messages watches the first,
    /// and a thread that has changed what is to be watched writes to the second. The
    /// one that waits made its list before the change, and would otherwise wait out
    /// its time before it made another.
    static mut WAKE: [i32; 2] = [-1; 2];

    /// Has the thread that waits for messages look again at what it waits for.
    #[allow(static_mut_refs)]
    unsafe fn wake() {
        unsafe {
            if WAKE[1] >= 0 {
                write(WAKE[1], b"w".as_ptr(), 1);
            }
        }
    }

    // ---- looking the tables over ---------------------------------------------------------
    //
    // A program started with `KOO_CHECK` set looks its tables over each time it lets go
    // of the lock on one, and ends at once, saying what it found, if a table is not as
    // every piece of this file takes it to be. The tests run with it set. What is looked
    // for is what has gone wrong before, or would be a message never answered, an actor
    // answering two at once, or memory let go of twice, if it did.

    /// 0 until it is known, then 1 for a program that does not look and 2 for one that does.
    static CHECKING: AtomicU32 = AtomicU32::new(0);

    fn checking() -> bool {
        unsafe extern "C" {
            fn getenv(name: *const u8) -> *const u8;
        }
        match CHECKING.load(Relaxed) {
            0 => {
                let set = unsafe { !getenv(c"KOO_CHECK".as_ptr() as *const u8).is_null() };
                CHECKING.store(if set { 2 } else { 1 }, Relaxed);
                set
            }
            known => known == 2,
        }
    }

    #[cold]
    unsafe fn unsound(table: &[u8], what: &[u8], at: usize) -> ! {
        unsafe extern "C" {
            fn abort() -> !;
        }
        unsafe {
            let mut out = Buffer::new();
            out.push(b"koo: KOO_CHECK: the table of ");
            out.push(table);
            out.push(b" is not as it should be: ");
            out.push(what);
            let _ = writeln!(out, " (entry {at})");
            out.flush(2);
            abort()
        }
    }

    /// Lets go of `SERVED`, having looked the connections over if the program looks.
    #[inline]
    #[allow(static_mut_refs)]
    unsafe fn served_unlock() {
        unsafe {
            if checking() {
                connections_sound();
            }
            SERVED.unlock();
        }
    }

    #[cold]
    #[allow(static_mut_refs)]
    unsafe fn connections_sound() {
        unsafe {
            static mut SEEN: [u64; 1024] = [0; 1024];
            let wrong = |what: &[u8], at: usize| -> ! { unsound(b"connections", what, at) };
            if CONN_COUNT > MOST_CLIENTS {
                wrong(b"more connections than there is room for", CONN_COUNT);
            }
            SEEN = [0; 1024];
            for (at, conn) in CONNS[..CONN_COUNT].iter().enumerate() {
                if conn.fd < 0 {
                    wrong(b"a connection that is closed", at);
                }
                if (conn.fd as usize) < 65536 {
                    let (word, bit) = (conn.fd as usize / 64, 1u64 << (conn.fd % 64));
                    if SEEN[word] & bit != 0 {
                        wrong(b"one connection twice", at);
                    }
                    SEEN[word] |= bit;
                }
                if !conn.message.is_null() && conn.len < 3 {
                    wrong(b"a message shorter than a message is", at);
                }
                if !conn.out.is_null() && conn.doing != SENDING && conn.doing != BUSY && conn.doing != GONE {
                    wrong(b"an answer put by on a connection that is not sending one", at);
                }
                match conn.doing {
                    IDLE => {
                        if conn.actor != NO_ACTOR {
                            wrong(b"an idle connection that keeps an actor", at);
                        }
                        if conn.message.is_null() != (conn.got < 4) {
                            wrong(b"an idle connection whose message and count of it disagree", at);
                        }
                        if !conn.message.is_null() && conn.got > 4 + conn.len {
                            wrong(b"more of a message than the message has", at);
                        }
                    }
                    BUSY | GONE => {
                        if !conn.message.is_null() {
                            wrong(b"a message being answered that is still with its connection", at);
                        }
                        let twice = conn.actor != NO_ACTOR
                            && CONNS[..at]
                                .iter()
                                .any(|other| other.actor == conn.actor && matches!(other.doing, BUSY | GONE));
                        if twice {
                            wrong(b"an actor that answers two messages at once", at);
                        }
                    }
                    HELD_UP => {
                        if conn.message.is_null() || conn.actor == NO_ACTOR {
                            wrong(b"a message held up that is not there, or is held up for no actor", at);
                        }
                        let answering = CONNS[..CONN_COUNT]
                            .iter()
                            .any(|other| other.actor == conn.actor && matches!(other.doing, BUSY | GONE));
                        if !answering {
                            wrong(b"a message held up for an actor that nobody is answering for", at);
                        }
                    }
                    SENDING => {
                        if conn.out.is_null() || conn.out_sent >= conn.out_len {
                            wrong(b"a connection sending an answer that has none left to send", at);
                        }
                        if conn.actor != NO_ACTOR || !conn.message.is_null() {
                            wrong(b"a connection sending an answer that keeps an actor or a message", at);
                        }
                    }
                    _ => wrong(b"a connection in no state there is", at),
                }
            }
        }
    }

    /// Lets go of `KEPT`, having looked over the connections kept by if the program looks.
    #[inline]
    #[allow(static_mut_refs)]
    unsafe fn kept_unlock() {
        unsafe {
            if checking() {
                if HOME_COUNT > MOST_HOMES {
                    unsound(b"connections kept by", b"more than there is room for", HOME_COUNT);
                }
                for (at, (_, fd)) in HOMES[..HOME_COUNT].iter().enumerate() {
                    if *fd < 0 || HOMES[..at].iter().any(|(_, other)| other == fd) {
                        unsound(b"connections kept by", b"one that is closed, or is there twice", at);
                    }
                }
            }
            KEPT.unlock();
        }
    }

    /// The threads that answer other processes. One of them at a time waits for a
    /// message, holding `LEADING`; when it has one, it lets another wait and answers.
    /// So an actor that takes its time keeps one thread, and the others are answered
    /// meanwhile. `SPARE` is how many wait to be the one that waits.
    static LEADING: Gate = Gate::new();
    static ANSWERERS: AtomicUsize = AtomicUsize::new(0);
    static SPARE: AtomicUsize = AtomicUsize::new(0);
    const MOST_ANSWERERS: usize = 128;
    /// The function of the program that answers a message, or 0 if it has not said.
    static ANSWERER: AtomicUsize = AtomicUsize::new(0);
    /// A message that was answered: it is not answered twice.
    const ANSWERED: u8 = 2;

    /// Whether the thread that asks holds nothing of the support library's that others
    /// wait for: it is not in the middle of talking to another process, and not in the
    /// middle of making an actor's home. Asked where a signal is handled, of the thread
    /// the signal interrupted, to know whether the thread may be sent elsewhere.
    pub fn quiet() -> bool {
        if unsafe { me().talking } {
            return false;
        }
        let free = MAKING.try_lock();
        if free {
            MAKING.unlock();
        }
        free
    }

    // What this process knows of others: connections to them that nothing is using. A
    // thread that asks takes one, or makes one, and gives it back with the answer, so
    // that handlers of a server that wait for answers side by side each have their own.
    static KEPT: Lock = Lock::new();
    static mut HOMES: [(Name, i32); MOST_HOMES] = [(NO_NAME, -1); MOST_HOMES];
    /// When each was put by, on the clock that only goes forward: one that nothing
    /// has used for ten seconds is closed, so that the process at the other end is
    /// not left holding, for every worker, as many as the worker once needed at once.
    static mut KEPT_AT: [i64; MOST_HOMES] = [0; MOST_HOMES];
    const KEPT_FOR: i64 = 10_000_000_000;
    static mut HOME_COUNT: usize = 0;
    static mut HOMES_OF: i32 = 0;
    /// How many connections are in use. A handler that asks has one of its own for as
    /// long as it waits, so that none waits for another's.
    static OUT: AtomicUsize = AtomicUsize::new(0);
    /// The most connections to one process that are kept when nothing uses them.
    const MOST_KEPT: usize = 128;

    pub unsafe fn open() {
        unsafe { LEADING.open() };
    }

    unsafe fn error() -> i32 {
        unsafe { *errno_location() }
    }

    /// Writes as much as the other end has room for, waiting for room until `until`
    /// and no longer (0 for not at all). Returns how much went out, or nothing if the
    /// other end is gone.
    unsafe fn write_some(fd: i32, data: &[u8], until: i64) -> Option<usize> {
        unsafe {
            const F_GETFL: i32 = 3;
            const F_SETFL: i32 = 4;
            let non_blocking = if cfg!(target_os = "macos") { 0x4 } else { 0x800 };
            let flags = fcntl(fd, F_GETFL);
            fcntl(fd, F_SETFL, flags | non_blocking);
            let (mut sent, mut there) = (0, true);
            while sent < data.len() {
                let n = write(fd, data.as_ptr().add(sent), data.len() - sent);
                if n > 0 {
                    sent += n as usize;
                    continue;
                }
                let code = error();
                if code == EINTR {
                    continue;
                }
                if code != EAGAIN {
                    there = false;
                    break;
                }
                let left = (until - crate::koo_now_nanos()) / 1_000_000;
                if until == 0 || left <= 0 {
                    break;
                }
                let mut room = PollFd {
                    fd,
                    events: POLLOUT,
                    revents: 0,
                };
                poll(&mut room, 1, left as i32);
            }
            fcntl(fd, F_SETFL, flags);
            there.then_some(sent)
        }
    }

    unsafe fn write_full(fd: i32, mut data: &[u8]) -> bool {
        while !data.is_empty() {
            let n = unsafe { write(fd, data.as_ptr(), data.len()) };
            if n > 0 {
                data = &data[n as usize..];
            } else if unsafe { error() } != EINTR {
                return false;
            }
        }
        true
    }

    /// The path of the socket's file, ended by a zero, and the process that made it: a
    /// process forked from it must not remove what is its parent's.
    static mut SOCKET_PATH: [u8; 128] = [0; 128];
    static mut SOCKET_OWNER: i32 = 0;

    unsafe extern "C" {
        fn atexit(function: extern "C" fn()) -> i32;
    }

    /// Removes the socket's file. It allocates nothing, so that it can be called from
    /// where a signal is handled.
    #[allow(static_mut_refs)]
    pub extern "C" fn remove_socket() {
        unsafe {
            if SOCKET_OWNER != 0 && SOCKET_OWNER == getpid() && SOCKET_PATH[0] != 0 {
                unlink(SOCKET_PATH.as_ptr());
            }
        }
    }

    /// Where other processes reach the actors of this one. The socket is made the first
    /// time it is asked for.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_actor_home() -> *mut Header {
        unsafe {
            let home = HOME.load(Acquire);
            if !home.is_null() {
                return home;
            }
            MAKING.lock();
            if HOME.load(Acquire).is_null() {
                let mut name = Buffer::new();
                if cfg!(target_os = "macos") {
                    let directory = getenv(c"TMPDIR".as_ptr() as *const u8);
                    let directory = if directory.is_null() {
                        b"/tmp" as &[u8]
                    } else {
                        c_text(directory)
                    };
                    // A socket's path is short: one that would not fit goes where it does.
                    let directory = if directory.len() > 60 { b"/tmp" } else { directory };
                    name.push(directory);
                    if !directory.ends_with(b"/") {
                        name.push(b"/");
                    }
                }
                let mut time = [0i64; 2];
                clock_gettime(0, &mut time);
                let _ = write!(name, "koo-{}-{}", getpid(), time[1]);
                if cfg!(target_os = "macos") {
                    name.push(b".sock");
                }
                let text = &name.data[..name.len];
                let (address, len) = address(text);
                let fd = socket(AF_UNIX, SOCK_STREAM, 0);
                if fd < 0 || bind(fd, &address as *const Address as *const u8, len) != 0 || listen(fd, 1024) != 0 {
                    MAKING.unlock();
                    die(&[b"actors could not be started: ", c_text(strerror(error()))]);
                }
                LISTENER.store(fd, Release);
                // A pipe by which the thread that waits for messages is told to look
                // again, by one that has changed what it should be looking at.
                if pipe(WAKE.as_mut_ptr()) == 0 {
                    for end in WAKE {
                        fcntl(
                            end,
                            4,
                            fcntl(end, 3) | if cfg!(target_os = "macos") { 0x4 } else { 0x800 },
                        );
                    }
                } else {
                    WAKE = [-1; 2];
                }
                // The socket has a file on macOS, which is removed however the process
                // ends, short of being killed: by `main` returning, a panic, or `exit`.
                if cfg!(target_os = "macos") {
                    let kept = text.len().min(SOCKET_PATH.len() - 1);
                    SOCKET_PATH[..kept].copy_from_slice(&text[..kept]);
                    SOCKET_OWNER = getpid();
                    atexit(remove_socket);
                }
                // The address is the support library's own, and every handle of an actor
                // of this process has it: a constant, which no thread counts.
                let home = new_str(text);
                uncounted();
                (*home).rc = IMMORTAL;
                HOME.store(home, Release);
            }
            MAKING.unlock();
            HOME.load(Acquire)
        }
    }

    /// Whether a handle's actor lives in this process.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_local(home: *const Header) -> bool {
        unsafe {
            let mine = HOME.load(Relaxed);
            core::ptr::eq(home, mine) || (!mine.is_null() && bytes(home) == bytes(mine))
        }
    }

    /// Where the actors of a server are kept, in a process that is one of its workers:
    /// the home of the process that started it.
    static PARENT: AtomicPtr<Header> = AtomicPtr::new(core::ptr::null_mut());

    /// In a process about to start workers: if the program has actors at all, this
    /// process has a home, where its workers find them and start more.
    pub unsafe fn before_fork() {
        unsafe {
            if super::runner() != 0 {
                koo_actor_home();
            }
        }
    }

    /// The home of the process whose worker this one is, or nothing.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_parent() -> *mut Header {
        unsafe {
            let parent = PARENT.load(Acquire);
            if parent.is_null() { new_str(b"") } else { parent }
        }
    }

    /// Whether the actors a program starts here live in another process: this one is
    /// a worker of a server, and an actor is of the whole server.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_elsewhere() -> bool {
        !PARENT.load(Acquire).is_null()
    }

    /// Before a process forks, and after, in the process that forked: the connections are
    /// not changed meanwhile, so the new process finds them whole.
    pub unsafe fn forking(begins: bool) {
        if begins {
            SERVED.lock();
        } else {
            unsafe { served_unlock() };
        }
    }

    /// In a process just forked: the actors are its parent's, and so is their socket.
    #[allow(static_mut_refs)]
    pub unsafe fn after_fork() {
        unsafe {
            PARENT.store(HOME.load(Acquire), Release);
            if LISTENER.load(Acquire) >= 0 {
                close(LISTENER.load(Acquire));
                LISTENER.store(-1, Release);
            }
            for end in WAKE {
                if end >= 0 {
                    close(end);
                }
            }
            WAKE = [-1; 2];
            for conn in &CONNS[..CONN_COUNT] {
                close(conn.fd);
            }
            CONN_COUNT = 0;
            // What a thread of the parent held is let go of; the threads are not here.
            served_unlock();
            kept_unlock();
            ANSWERERS.store(0, Release);
            SPARE.store(0, Release);
            OUT.store(0, Release);
            // The address is kept, and no longer matched: a handle that names it is
            // reached through the socket, like any other.
            HOME.store(core::ptr::null_mut(), Release);
            alone_after_fork();
        }
    }

    /// Whether there was no room for another connection, and none that could be let
    /// go of, when one was last waiting to be taken. It is so until one ends or is done.
    static CROWDED: AtomicBool = AtomicBool::new(false);

    /// Lets go of a connection: closes it and takes it out of the list. With `SERVED`.
    #[allow(static_mut_refs)]
    unsafe fn drop_conn(index: usize) {
        unsafe {
            // (There is room again, if there was none: the thread that waits for
            // messages left the listener alone meanwhile, and is told to look again.)
            if CROWDED.swap(false, Relaxed) {
                wake();
            }
            // Whoever waits at the other end learns at once, whoever else has the socket.
            shutdown(CONNS[index].fd, 2);
            close(CONNS[index].fd);
            free(CONNS[index].message);
            free(CONNS[index].out);
            CONN_COUNT -= 1;
            CONNS[index] = CONNS[CONN_COUNT];
        }
    }

    /// Says which function of the program answers a message from another process. Called
    /// as the program starts.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_answerer(answerer: usize) {
        ANSWERER.store(answerer, Release);
    }

    /// Starts another thread that answers, if there may be more. Returns whether it did.
    unsafe fn another_answerer() -> bool {
        unsafe {
            // No more than the table of threads has room for, beside the threads that
            // run what actors are told and those that stand in for them while they wait.
            static PROCESSORS: AtomicUsize = AtomicUsize::new(0);
            if PROCESSORS.load(Relaxed) == 0 {
                PROCESSORS.store(thread::processors(), Relaxed);
            }
            let others = WORKERS.load(SeqCst).max(PROCESSORS.load(Relaxed)) * 2 + 8;
            let most = thread::MOST_THREADS.saturating_sub(others).clamp(1, MOST_ANSWERERS);
            if ANSWERERS.fetch_add(1, AcqRel) < most && thread::start(answer) {
                return true;
            }
            ANSWERERS.fetch_sub(1, AcqRel);
            false
        }
    }

    /// What a thread that answers does: it waits its turn to wait for a message, and
    /// when it has one, sees that another thread waits in its place, and answers.
    extern "C" fn answer(_: *mut u8) -> *mut u8 {
        unsafe {
            thread::watch_stack();
            super::landing_stack();
            loop {
                SPARE.fetch_add(1, AcqRel);
                LEADING.lock();
                SPARE.fetch_sub(1, AcqRel);
                let taken = take(1000);
                if taken && SPARE.load(Acquire) == 0 {
                    another_answerer();
                }
                LEADING.unlock();
                if taken {
                    answer_taken();
                }
            }
        }
    }

    /// In the process that watches the workers of a server, from time to time: sees that
    /// what the workers ask of the actors is answered, and waits up to some milliseconds.
    /// The answering is done by threads of its own, so that an actor that takes its time
    /// holds up neither the watching nor the other actors; where no thread can be
    /// started, the calling thread answers a message itself.
    pub unsafe fn pending(milliseconds: i32) -> bool {
        unsafe {
            super::run_what_waits();
            if LISTENER.load(Acquire) >= 0 {
                gates();
                if ANSWERERS.load(Acquire) == 0 && !another_answerer() {
                    LEADING.lock();
                    let taken = take(milliseconds);
                    LEADING.unlock();
                    if taken {
                        // (With a stack to come back by, as the threads that answer
                        // have: a method that runs out of stack is then a panic in the
                        // actor here too, and not the end of the server.)
                        if me().landing == 0 {
                            super::landing_stack();
                        }
                        answer_taken();
                    }
                    return false;
                }
            }
            if milliseconds > 0 {
                let (wanted, mut left) = ([0i64, milliseconds as i64 * 1_000_000], [0i64; 2]);
                nanosleep(&wanted, &mut left);
            }
            false
        }
    }

    /// What stands for no actor: the first actor of a program has the number 0.
    const NO_ACTOR: i64 = -1;

    /// The actor a message keeps while it is answered, or `NO_ACTOR`: an ask of a method, or to
    /// stop, is answered with the actor held, and only one such at a time is taken up
    /// for an actor. To start one, and to try to tell one, keeps none.
    fn kept_actor(kind: u8, method: &[u8]) -> i64 {
        let mut parts = method.split(|byte| *byte == b' ');
        let (_, id, called) = (parts.next(), parts.next(), parts.next().unwrap_or(b""));
        if kind != ASK || called.starts_with(b"try.") || called.starts_with(b".") {
            return NO_ACTOR;
        }
        id.and_then(|id| core::str::from_utf8(id).ok())
            .and_then(|id| id.parse::<i64>().ok())
            .unwrap_or(NO_ACTOR)
    }

    /// Waits up to some milliseconds for a message from another process, with `LEADING`
    /// held. Returns whether there is one for the calling thread to answer: it is then
    /// the thread's, with its connection, until `answer_taken` has let go of them.
    #[allow(static_mut_refs)]
    unsafe fn take(milliseconds: i32) -> bool {
        unsafe {
            let mut fds: [PollFd; MOST_CLIENTS + 2] = core::array::from_fn(|_| PollFd {
                fd: -1,
                events: POLLIN,
                revents: 0,
            });
            let mut count = 1;
            SERVED.lock();
            // (Not while there is no room for another connection and none to let go of:
            // it would say without end that one is waiting.)
            fds[0].fd = if CROWDED.load(Relaxed) {
                -1
            } else {
                LISTENER.load(Acquire)
            };
            // A connection that is being answered, or whose message waits its turn, is
            // watched too: its next message may arrive before this thread looks again,
            // and would otherwise lie there until something else woke the thread.
            for conn in &CONNS[..CONN_COUNT] {
                if conn.doing == SENDING {
                    // One whose answer is going out is watched for room to write in.
                    fds[count].fd = conn.fd;
                    fds[count].events = POLLOUT;
                    count += 1;
                } else if conn.doing != GONE && !conn.early {
                    fds[count].fd = conn.fd;
                    count += 1;
                }
            }
            served_unlock();
            // And the pipe by which it is told that the list has changed, last.
            fds[count].fd = WAKE[0];
            if poll(fds.as_mut_ptr(), count + 1, milliseconds) <= 0 {
                return false;
            }
            if fds[count].revents != 0 {
                let mut told = [0u8; 64];
                while read(WAKE[0], told.as_mut_ptr(), told.len()) == told.len() as isize {}
            }
            // Every connection that waits to be taken is taken, and not one at a time:
            // a worker whose handlers all ask at once makes as many.
            let mut waiting = fds[0].revents != 0;
            while waiting {
                // Where there is no room for another connection, in the table or among
                // the files the process may have open, one that nothing is using gives
                // way: a worker keeps connections by for the next that asks, and will
                // make another when it finds this one gone. If every one is in use,
                // those that wait to be taken wait until one is done.
                let made_room = |full: bool| -> bool {
                    SERVED.lock();
                    // The one that has lain unused the longest: one that was taken a
                    // moment ago has an asker about to write to it.
                    let mut idle: Option<usize> = None;
                    for (index, conn) in CONNS[..CONN_COUNT].iter().enumerate() {
                        let mut byte = [0u8; 1];
                        if conn.doing == IDLE
                            && conn.got == 0
                            && idle.is_none_or(|idle| conn.turn < CONNS[idle].turn)
                            && recv(conn.fd, byte.as_mut_ptr(), 1, MSG_PEEK | MSG_DONTWAIT) < 0
                            && error() == EAGAIN
                        {
                            idle = Some(index);
                        }
                    }
                    if let Some(idle) = idle {
                        drop_conn(idle);
                    } else if full {
                        // Said with the lock, so that a thread which makes room a
                        // moment later is not unsaid by this one.
                        CROWDED.store(true, Relaxed);
                    }
                    served_unlock();
                    idle.is_some()
                };
                // (The count is read as it is changed, with the lock: the threads that
                // answer let go of connections while this one looks.)
                SERVED.lock();
                let full = CONN_COUNT >= MOST_CLIENTS;
                served_unlock();
                if full && !made_room(true) {
                    break;
                }
                let mut fd = accept(LISTENER.load(Acquire), core::ptr::null_mut(), core::ptr::null_mut());
                // 23 and 24 are the system's words for no more files.
                if fd < 0 && matches!(error(), 23 | 24) && made_room(false) {
                    fd = accept(LISTENER.load(Acquire), core::ptr::null_mut(), core::ptr::null_mut());
                }
                let mut room = false;
                if fd >= 0 {
                    SERVED.lock();
                    CONNS[CONN_COUNT] = Conn {
                        fd,
                        turn: TURNS,
                        ..NO_CONN
                    };
                    TURNS += 1;
                    CONN_COUNT += 1;
                    room = true;
                    served_unlock();
                } else if error() != EINTR && error() != EAGAIN {
                    // Every connection is in use: a moment is let pass, so that the
                    // asking again is not without end.
                    let (wanted, mut left) = ([0i64, 10_000_000], [0i64; 2]);
                    nanosleep(&wanted, &mut left);
                }
                let mut more = PollFd {
                    fd: LISTENER.load(Acquire),
                    events: POLLIN,
                    revents: 0,
                };
                waiting = room && poll(&mut more, 1, 0) > 0;
            }
            // Each connection in its turn, so that no sender waits behind a busy one.
            let mut taken = false;
            for step in 0..count - 1 {
                let slot = &fds[1 + (TURN + step) % (count - 1)];
                if slot.revents == 0 {
                    continue;
                }
                let fd = slot.fd;
                SERVED.lock();
                let Some(index) = CONNS[..CONN_COUNT].iter().position(|conn| conn.fd == fd) else {
                    served_unlock();
                    continue;
                };
                if CONNS[index].doing == SENDING {
                    // Room for more of an answer that is going out: as much as there
                    // is room for goes, and when all has gone the connection is free
                    // for its next message.
                    let conn = &mut CONNS[index];
                    let rest = core::slice::from_raw_parts(conn.out.add(conn.out_sent), conn.out_len - conn.out_sent);
                    match write_some(fd, rest, 0) {
                        None => drop_conn(index),
                        Some(sent) => {
                            conn.out_sent += sent;
                            if conn.out_sent == conn.out_len {
                                free(conn.out);
                                conn.out = core::ptr::null_mut();
                                conn.out_len = 0;
                                conn.out_sent = 0;
                                conn.doing = IDLE;
                                conn.early = false;
                                conn.turn = TURNS;
                                TURNS += 1;
                                CROWDED.store(false, Relaxed);
                            }
                        }
                    }
                    served_unlock();
                    continue;
                }
                if CONNS[index].doing != IDLE {
                    // Something on a connection that is being answered: the other end
                    // is gone, or its next message is early.
                    let mut byte = [0u8; 1];
                    let there = recv(fd, byte.as_mut_ptr(), 1, MSG_PEEK | MSG_DONTWAIT);
                    if there == 0 || (there < 0 && error() != EAGAIN && error() != EINTR) {
                        match CONNS[index].doing {
                            BUSY => CONNS[index].doing = GONE,
                            // Nobody is answering it yet, and nobody is there to be
                            // answered: its message is let go of with it.
                            HELD_UP => drop_conn(index),
                            _ => {}
                        }
                    } else {
                        CONNS[index].early = true;
                    }
                    served_unlock();
                    continue;
                }
                if taken {
                    // This thread has its message. Another that is here whole, and is
                    // for an actor that is answering, this thread's among them, is set
                    // to wait its turn now: whoever answers that actor then goes from
                    // one message to the next without another thread being woken.
                    let mut whole = [0u8; 256];
                    // (Not one of which a part was read before: what is here of that is
                    // the rest of it, and it is read when this thread's turn is over.)
                    let here = if CONNS[index].got == 0 {
                        recv(fd, whole.as_mut_ptr(), whole.len(), MSG_PEEK | MSG_DONTWAIT)
                    } else {
                        0
                    };
                    let size = u32::from_le_bytes([whole[0], whole[1], whole[2], whole[3]]) as usize;
                    let method_end = 3 + u16::from_le_bytes([whole[5], whole[6]]) as usize;
                    if here >= 7 && size >= 3 && 4 + size <= here as usize && method_end <= size {
                        let text = &whole[4..4 + size];
                        let actor = kept_actor(text[0], &text[3..method_end]);
                        let answering = actor != NO_ACTOR
                            && CONNS[..CONN_COUNT]
                                .iter()
                                .any(|conn| conn.fd != fd && conn.actor == actor && conn.doing != IDLE);
                        let message = if answering { malloc(size) } else { core::ptr::null_mut() };
                        let mut again = [0u8; 256];
                        if !message.is_null()
                            && recv(fd, again.as_mut_ptr(), 4 + size, MSG_DONTWAIT) == 4 + size as isize
                        {
                            core::ptr::copy_nonoverlapping(again.as_ptr().add(4), message, size);
                            CONNS[index].doing = HELD_UP;
                            CONNS[index].actor = actor;
                            CONNS[index].turn = TURNS;
                            TURNS += 1;
                            CONNS[index].message = message;
                            CONNS[index].len = size;
                        } else {
                            free(message);
                        }
                    }
                    served_unlock();
                    continue;
                }
                // What has come of its message is read, and what has not is not waited
                // for: a sender that stops in the middle of a long message, as a worker
                // does that has other things to do, holds up nobody but itself. Only
                // the thread that waits for messages reads a connection that is idle.
                let (mut got, mut message, mut size, mut head) = (
                    CONNS[index].got,
                    CONNS[index].message,
                    CONNS[index].len,
                    CONNS[index].head,
                );
                served_unlock();
                let mut gone = false;
                loop {
                    if got >= 4 && message.is_null() {
                        size = u32::from_le_bytes(head) as usize;
                        message = if size >= 3 { malloc(size) } else { core::ptr::null_mut() };
                        if message.is_null() {
                            gone = true;
                            break;
                        }
                    }
                    if got >= 4 && got == 4 + size {
                        break;
                    }
                    let n = if got < 4 {
                        recv(fd, head.as_mut_ptr().add(got), 4 - got, MSG_DONTWAIT)
                    } else {
                        recv(fd, message.add(got - 4), 4 + size - got, MSG_DONTWAIT)
                    };
                    if n > 0 {
                        got += n as usize;
                        continue;
                    }
                    let why = error();
                    if n < 0 && why == EINTR {
                        continue;
                    }
                    gone = n == 0 || why != EAGAIN;
                    break;
                }
                let all = !gone && got >= 4 && got == 4 + size;
                let whole = all && 3 + u16::from_le_bytes([*message.add(1), *message.add(2)]) as usize <= size;
                SERVED.lock();
                // Nothing has moved the connection but the leaving of others, so it is
                // looked for again.
                let Some(index) = CONNS[..CONN_COUNT].iter().position(|conn| conn.fd == fd) else {
                    free(message);
                    served_unlock();
                    continue;
                };
                (
                    CONNS[index].got,
                    CONNS[index].message,
                    CONNS[index].len,
                    CONNS[index].head,
                ) = (got, message, size, head);
                if gone || (all && !whole) {
                    drop_conn(index);
                    served_unlock();
                    continue;
                }
                if !whole {
                    // The rest is read when it comes.
                    served_unlock();
                    continue;
                }
                (CONNS[index].got, CONNS[index].message) = (0, core::ptr::null_mut());
                CONNS[index].doing = BUSY;
                CONNS[index].actor = NO_ACTOR;
                let text = core::slice::from_raw_parts(message, size);
                let method_end = 3 + u16::from_le_bytes([text[1], text[2]]) as usize;
                let actor = kept_actor(text[0], &text[3..method_end]);
                let held_up = actor != NO_ACTOR
                    && CONNS[..CONN_COUNT]
                        .iter()
                        .any(|conn| conn.fd != fd && conn.actor == actor && conn.doing != IDLE);
                CONNS[index].actor = actor;
                if held_up {
                    // Its actor is answering another: it waits, and whoever answers
                    // that one takes this one up next.
                    CONNS[index].doing = HELD_UP;
                    CONNS[index].turn = TURNS;
                    TURNS += 1;
                    CONNS[index].message = message;
                    CONNS[index].len = size;
                    served_unlock();
                    continue;
                }
                TURN = (TURN + step + 1) % (count - 1);
                served_unlock();
                let me = core::ptr::from_mut(me());
                (*me).asked_on = fd + 1;
                (*me).asked_kind = text[0];
                (*me).asked = message;
                (*me).asked_len = size;
                (*me).asked_method = method_end;
                taken = true;
            }
            taken
        }
    }

    /// Whether a message, `Kind id method`, names its actor by the kind the actor is of.
    /// One that names no actor (`Kind 0 .start`), or one that is no longer there, is for
    /// the program to answer as it does.
    unsafe fn of_its_kind(message: &[u8]) -> bool {
        unsafe {
            let mut parts = message.split(|byte| *byte == b' ');
            let (Some(kind), Some(id), Some(called)) = (parts.next(), parts.next(), parts.next()) else {
                return true;
            };
            // What is asked of a kind and of no actor, to start one, has a number that
            // names nothing.
            if called.starts_with(b".") {
                return true;
            }
            let Some(id) = core::str::from_utf8(id).ok().and_then(|id| id.parse::<i64>().ok()) else {
                return true;
            };
            match slot_of(id) {
                Some(slot) => kind_name((*slot).kind as usize) == kind,
                None => true,
            }
        }
    }

    /// Answers the message the thread has taken, and after it those that were held up
    /// for the same actor, the oldest first; then lets go of the connection.
    #[allow(static_mut_refs)]
    unsafe fn answer_taken() {
        unsafe {
            loop {
                let me = core::ptr::from_mut(me());
                let fd = (*me).asked_on - 1;
                // The asker is told that its message is the one being answered.
                let there = (*me).asked_kind != ASK || write_full(fd, b"A");
                let answerer = ANSWERER.load(Acquire);
                // A message that names an actor by a kind the actor is not of comes
                // from no worker of this program: it is not run, and whoever sent it
                // is hung up on.
                let message = core::slice::from_raw_parts((*me).asked.add(3), (*me).asked_method - 3);
                let there = there && of_its_kind(message);
                if there && answerer != 0 {
                    let answer: extern "C" fn() = core::mem::transmute(answerer);
                    answer();
                }
                let me = core::ptr::from_mut(crate::thread::me());
                // A connection that was hung up on has its number turned about.
                let leave = !there || answerer == 0 || (*me).asked_on < 0;
                free((*me).asked);
                (*me).asked = core::ptr::null_mut();
                (*me).asked_on = 0;
                SERVED.lock();
                let index = CONNS[..CONN_COUNT].iter().position(|conn| conn.fd == fd);
                let actor = index.map_or(NO_ACTOR, |index| CONNS[index].actor);
                if let Some(index) = index {
                    if leave || CONNS[index].doing == GONE {
                        drop_conn(index);
                    } else if !CONNS[index].out.is_null() {
                        // Its answer is still going out, and whoever waits for a
                        // message sends the rest: the actor is not kept for that.
                        CONNS[index].doing = SENDING;
                        CONNS[index].actor = NO_ACTOR;
                        wake();
                    } else {
                        // Its next message may be here already, sent the moment the
                        // answer arrived and before this was reached: whoever waits
                        // for messages left it alone then, and is told to look again.
                        if CONNS[index].early {
                            wake();
                        }
                        CONNS[index].doing = IDLE;
                        CONNS[index].actor = NO_ACTOR;
                        CONNS[index].early = false;
                        CONNS[index].turn = TURNS;
                        TURNS += 1;
                        if CROWDED.swap(false, Relaxed) {
                            wake();
                        }
                    }
                }
                let mut next: Option<usize> = None;
                if actor != NO_ACTOR {
                    for (at, conn) in CONNS[..CONN_COUNT].iter().enumerate() {
                        let older = next.is_none_or(|next| conn.turn < CONNS[next].turn);
                        if conn.doing == HELD_UP && conn.actor == actor && older {
                            next = Some(at);
                        }
                    }
                }
                let Some(next) = next else {
                    served_unlock();
                    return;
                };
                let conn = &mut CONNS[next];
                conn.doing = BUSY;
                (*me).asked_on = conn.fd + 1;
                (*me).asked = core::mem::replace(&mut conn.message, core::ptr::null_mut());
                (*me).asked_len = conn.len;
                (*me).asked_kind = *(*me).asked;
                (*me).asked_method = 3 + u16::from_le_bytes([*(*me).asked.add(1), *(*me).asked.add(2)]) as usize;
                served_unlock();
            }
        }
    }

    /// What the message from another process asks: `Kind id method`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_message() -> *mut Header {
        unsafe {
            let me = core::ptr::from_mut(me());
            if (*me).asked.is_null() {
                return new_str(b"");
            }
            new_str(core::slice::from_raw_parts((*me).asked.add(3), (*me).asked_method - 3))
        }
    }

    /// What the message was sent with.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_body() -> *mut Header {
        unsafe {
            let me = core::ptr::from_mut(me());
            if (*me).asked.is_null() {
                return new_str(b"");
            }
            new_str(core::slice::from_raw_parts(
                (*me).asked.add((*me).asked_method),
                (*me).asked_len - (*me).asked_method,
            ))
        }
    }

    /// Answers the message: `kind` is empty for a value, and names the error for one.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_reply(kind: *const Header, body: *const Header) {
        unsafe {
            koo_flush();
            let me = core::ptr::from_mut(me());
            if (*me).asked_on <= 0 || (*me).asked_kind != ASK {
                return;
            }
            (*me).asked_kind = ANSWERED;
            let fd = (*me).asked_on - 1;
            let (kind, body) = (bytes(kind), bytes(body));
            let size = (2 + kind.len() + body.len()) as u32;
            let mut head = [0u8; 6];
            head[..4].copy_from_slice(&size.to_le_bytes());
            head[4..].copy_from_slice(&(kind.len() as u16).to_le_bytes());
            // An asker that is gone is not answered, and nothing else is the matter.
            // One that is there and takes its answer slowly, a worker in the middle of
            // a long computation for one, is waited for a moment and no longer: what is
            // left of the answer is put by with the connection, to go out as there is
            // room, and the actor goes on to its next message meanwhile.
            let until = crate::koo_now_nanos() + 50_000_000;
            let parts: [&[u8]; 3] = [&head, kind, body];
            let total = head.len() + kind.len() + body.len();
            let mut done = 0;
            for part in parts {
                match write_some(fd, part, until) {
                    Some(sent) => {
                        done += sent;
                        if sent < part.len() {
                            break;
                        }
                    }
                    None => {
                        (*me).asked_on = -(*me).asked_on;
                        return;
                    }
                }
            }
            if done == total {
                return;
            }
            let rest = malloc(total - done);
            let (mut skip, mut at) = (done, 0);
            if !rest.is_null() {
                for part in parts {
                    let from = skip.min(part.len());
                    skip -= from;
                    core::ptr::copy_nonoverlapping(part.as_ptr().add(from), rest.add(at), part.len() - from);
                    at += part.len() - from;
                }
            }
            SERVED.lock();
            let index = CONNS[..CONN_COUNT].iter().position(|conn| conn.fd == fd);
            match index {
                Some(index) if !rest.is_null() && CONNS[index].doing == BUSY => {
                    CONNS[index].out = rest;
                    CONNS[index].out_len = total - done;
                    CONNS[index].out_sent = 0;
                }
                _ => {
                    free(rest);
                    (*me).asked_on = -(*me).asked_on;
                }
            }
            served_unlock();
        }
    }

    /// Ends the connection of the message being answered without an answer: its asker
    /// learns that the message panicked.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_hang_up() {
        unsafe {
            let me = core::ptr::from_mut(me());
            if (*me).asked_on > 0 {
                (*me).asked_on = -(*me).asked_on;
            }
        }
    }

    /// Does other work, or nothing, for so many nanoseconds.
    unsafe fn nap(nanos: i64) {
        unsafe {
            if !crate::fiber::pause(nanos) {
                let (wanted, mut left) = ([0i64, nanos], [0i64; 2]);
                nanosleep(&wanted, &mut left);
            }
        }
    }

    /// What `connection` returns for a process that is there and cannot take a
    /// connection this moment.
    const BUSY_THERE: i32 = -2;

    /// A connection to another process that nothing else is using: one that was kept,
    /// or a new one when there is none, or when the ones kept are no longer to be
    /// trusted (`fresh`). Returns -1 if the process is not there.
    #[allow(static_mut_refs)]
    unsafe fn connection(name: &[u8], fresh: bool) -> i32 {
        unsafe {
            OUT.fetch_add(1, AcqRel);
            KEPT.lock();
            // A connection is one process's: what a fork inherited is its parent's.
            if HOMES_OF != getpid() {
                for (_, fd) in &HOMES[..HOME_COUNT] {
                    close(*fd);
                }
                HOME_COUNT = 0;
                HOMES_OF = getpid();
                signal(SIGPIPE, 1);
            }
            let mut kept = -1;
            let mut index = 0;
            let now = crate::koo_now_nanos();
            while index < HOME_COUNT {
                let old = now - KEPT_AT[index] > KEPT_FOR;
                let mine = HOMES[index].0.text() == name;
                // One is taken; one that has lain too long, or leads where the last
                // one led nowhere, is closed; and the rest stay for the next to ask.
                let taken = mine && !fresh && !old && kept < 0;
                if !(taken || old || mine && fresh) {
                    index += 1;
                    continue;
                }
                let fd = HOMES[index].1;
                HOME_COUNT -= 1;
                HOMES[index] = HOMES[HOME_COUNT];
                KEPT_AT[index] = KEPT_AT[HOME_COUNT];
                if taken {
                    kept = fd;
                } else {
                    close(fd);
                }
            }
            kept_unlock();
            if kept >= 0 {
                return kept;
            }
            let (address, len) = address(name);
            let fd = socket(AF_UNIX, SOCK_STREAM, 0);
            // Without waiting: where as many as the other process lets wait are
            // waiting already, the answer is that there is no room, and it is asked
            // again in a moment, by a handler that is left where it is meanwhile.
            if fd < 0 || !crate::pg::connect_within(fd, &address as *const Address as *const u8, len, 2000) {
                // The process is there and has no room for another connection this
                // moment, which Linux says in so many words; or it is not there.
                // (Or this process has no file left to make a connection with, which
                // 23 and 24 say: one is free when another of its handlers is done.)
                let busy = if fd < 0 {
                    matches!(error(), 23 | 24)
                } else {
                    cfg!(target_os = "linux") && error() == EAGAIN
                };
                if fd >= 0 {
                    close(fd);
                }
                OUT.fetch_sub(1, AcqRel);
                return if busy { BUSY_THERE } else { -1 };
            }
            fd
        }
    }

    /// Closes the connections that have been kept longer than they are kept for. A
    /// server asks it once a second, so that a worker which was busy once and is quiet
    /// now does not go on holding, in the process that has the actors, the places its
    /// busy moment took.
    #[allow(static_mut_refs)]
    pub unsafe fn let_go_of_old() {
        unsafe {
            if HOME_COUNT == 0 || HOMES_OF != getpid() || !KEPT.try_lock() {
                return;
            }
            let now = crate::koo_now_nanos();
            let mut index = 0;
            while index < HOME_COUNT {
                if now - KEPT_AT[index] > KEPT_FOR {
                    close(HOMES[index].1);
                    HOME_COUNT -= 1;
                    HOMES[index] = HOMES[HOME_COUNT];
                    KEPT_AT[index] = KEPT_AT[HOME_COUNT];
                } else {
                    index += 1;
                }
            }
            kept_unlock();
        }
    }

    /// Gives a connection back: it is kept for the next that asks, if it is still good
    /// and not too many are kept already, and closed otherwise.
    #[allow(static_mut_refs)]
    unsafe fn give_back(name: &[u8], fd: i32, good: bool) {
        unsafe {
            KEPT.lock();
            let kept = HOMES[..HOME_COUNT]
                .iter()
                .filter(|(known, _)| known.text() == name)
                .count();
            if good && HOMES_OF == getpid() && HOME_COUNT < MOST_HOMES && kept < MOST_KEPT {
                HOMES[HOME_COUNT] = (Name::of(name), fd);
                KEPT_AT[HOME_COUNT] = crate::koo_now_nanos();
                HOME_COUNT += 1;
            } else {
                close(fd);
            }
            kept_unlock();
            OUT.fetch_sub(1, AcqRel);
        }
    }

    /// Sends a message on a connection. Returns whether it went out whole.
    unsafe fn deliver(fd: i32, kind: u8, method: &[u8], body: &[u8]) -> bool {
        unsafe {
            let size = 1 + 2 + method.len() + body.len();
            let mut head = [0u8; 7];
            head[..4].copy_from_slice(&(size as u32).to_le_bytes());
            head[4] = kind;
            head[5..].copy_from_slice(&(method.len() as u16).to_le_bytes());
            // A long message does not keep the thread while the other process has no
            // room for it: a handler of a server is left where it is meanwhile, and
            // the answers its neighbours wait for are read.
            // (And is not in the middle of anything while it is left there, as in
            // `read_waiting`.)
            me().talking = false;
            let send = |data: &[u8]| crate::pg::send_within(fd, data, 0);
            let sent = send(&head) && send(method) && send(body);
            me().talking = true;
            sent
        }
    }

    /// Reads as `read_full` does, but a handler of a server that has to wait for what it
    /// reads is left where it is, and the thread answers other requests meanwhile.
    unsafe fn read_waiting(fd: i32, data: &mut [u8]) -> bool {
        unsafe {
            let mut got = 0;
            while got < data.len() {
                let n = recv(fd, data.as_mut_ptr().add(got), data.len() - got, MSG_DONTWAIT);
                if n > 0 {
                    got += n as usize;
                    continue;
                }
                if n == 0 || (error() != EAGAIN && error() != EINTR) {
                    return false;
                }
                if error() == EINTR {
                    continue;
                }
                // The thread is not in the middle of anything while it waits here.
                me().talking = false;
                let waited = crate::fiber::wait(fd, false, 0);
                me().talking = true;
                if waited == crate::fiber::Waited::NotHere {
                    let mut ready = PollFd {
                        fd,
                        events: POLLIN,
                        revents: 0,
                    };
                    poll(&mut ready, 1, -1);
                }
            }
            true
        }
    }

    /// Asks an actor of another process, and waits for the answer, which is returned;
    /// its kind is kept for `koo_actor_kind`. A handler of a server that asks is left
    /// where it is while it waits, and its thread answers other requests.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_ask(
        home: *const Header,
        method: *const Header,
        body: *const Header,
    ) -> *mut Header {
        unsafe {
            let (name, method, body) = (bytes(home), bytes(method), bytes(body));
            let asked = method.rsplit(|b| *b == b' ').next().unwrap_or(method);
            koo_flush();
            // A process that is there and cannot take a connection this moment, because
            // as many as it lets wait are waiting, is tried for two seconds; one that
            // took the connection and let go of it, three times.
            let (mut attempt, mut refusals, mut fresh) = (0, 0, false);
            while attempt < 3 && refusals < 100 {
                // (After a connection that led nowhere the ones kept are let go of,
                // once: not again on every try while the process has no room, which
                // would close what the worker's other handlers have given back since.)
                let fd = connection(name, fresh);
                fresh = false;
                // A process that is there is waited for, however long it has no room:
                // those whose connections it has taken are waiting no less.
                if fd == BUSY_THERE {
                    nap(20_000_000);
                    continue;
                }
                me().talking = true;
                let mut taken = [0u8; 1];
                // No word that the message was taken up: the process that has the actor
                // is not there to take it.
                if fd < 0 || !deliver(fd, ASK, method, body) || !read_waiting(fd, &mut taken) {
                    me().talking = false;
                    if fd >= 0 {
                        give_back(name, fd, false);
                        attempt += 1;
                        fresh = true;
                    } else {
                        refusals += 1;
                    }
                    if attempt > 1 || refusals > 0 {
                        nap(20_000_000);
                    }
                    continue;
                }
                let mut size = [0u8; 4];
                let size_read = read_waiting(fd, &mut size);
                let size = u32::from_le_bytes(size) as usize;
                let answer = if size_read && size >= 2 {
                    malloc(size)
                } else {
                    core::ptr::null_mut()
                };
                if answer.is_null() || !read_waiting(fd, core::slice::from_raw_parts_mut(answer, size)) {
                    free(answer);
                    me().talking = false;
                    give_back(name, fd, false);
                    die(&[b"an actor panicked while it answered `", asked, b"`"]);
                }
                let answer_text = core::slice::from_raw_parts(answer, size);
                let kind_len = u16::from_le_bytes([answer_text[0], answer_text[1]]) as usize;
                let kind = &answer_text[2..2 + kind_len.min(size - 2)];
                let me = core::ptr::from_mut(me());
                let most = (*me).answer_kind.len();
                let kept = kind.len().min(most);
                (*me).answer_kind_len = kept;
                (&mut (*me).answer_kind)[..kept].copy_from_slice(&kind[..kept]);
                let text = new_str(&answer_text[(2 + kind_len).min(size)..]);
                free(answer);
                (*me).talking = false;
                give_back(name, fd, true);
                return text;
            }
            die(&[
                b"`",
                asked,
                b"` was asked of an actor whose process is gone (ActorStopped)",
            ]);
        }
    }

    /// The kind of the last answer: empty for a value, or the name of an error.
    #[unsafe(no_mangle)]
    #[allow(static_mut_refs)]
    pub unsafe extern "C" fn koo_actor_kind() -> *mut Header {
        unsafe {
            let me = me();
            new_str(&me.answer_kind[..me.answer_kind_len])
        }
    }

    /// Tells an actor of another process, and goes on. A message for a process that is
    /// gone is dropped.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn koo_actor_send(home: *const Header, method: *const Header, body: *const Header) {
        unsafe {
            let (name, method, body) = (bytes(home), bytes(method), bytes(body));
            koo_flush();
            let mut gone = true;
            for attempt in 0..2 {
                let fd = connection(name, attempt > 0);
                if fd < 0 {
                    continue;
                }
                me().talking = true;
                let sent = deliver(fd, SEND, method, body);
                me().talking = false;
                give_back(name, fd, sent);
                if sent {
                    gone = false;
                    break;
                }
            }
            if gone {
                let mut out = Buffer::new();
                out.push(b"koo: a message for an actor whose process is gone was dropped\n");
                out.flush(2);
            }
        }
    }

    /// When `main` returns: the socket is closed, and what the runtime kept is given back.
    #[allow(static_mut_refs)]
    pub unsafe fn at_exit() {
        unsafe {
            if LISTENER.load(Acquire) >= 0 {
                close(LISTENER.load(Acquire));
                LISTENER.store(-1, Release);
                if cfg!(target_os = "macos") && !HOME.load(Acquire).is_null() {
                    let path = c_string(HOME.load(Acquire));
                    unlink(path);
                    free(path);
                }
            }
            SERVED.lock();
            for conn in &CONNS[..CONN_COUNT] {
                // One that a thread is still answering on is that thread's to close.
                if conn.doing != BUSY && conn.doing != GONE {
                    close(conn.fd);
                }
            }
            CONN_COUNT = 0;
            served_unlock();
            KEPT.lock();
            for (_, fd) in &HOMES[..HOME_COUNT] {
                close(*fd);
            }
            HOME_COUNT = 0;
            kept_unlock();
        }
    }
}
