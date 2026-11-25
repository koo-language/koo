//! Stacks to wait on, for the handlers of a server (`http.rs`).
//!
//! A worker of a server is one thread, and a handler that waits, for a database or for
//! another server, would make every other request of that worker wait with it. So a
//! handler that is about to wait is left where it is, on a stack of its own, and the
//! thread goes on with another request on another stack; when what the handler waited
//! for has come, the thread comes back to it. Nothing runs at the same time as anything
//! else, so nothing needs to be locked or counted differently for it.
//!
//! This file has the switch from one stack to another, the making of a stack, and the
//! two questions that the rest of the support library asks before it waits. The server
//! answers them; where there is no server, or the thread is not the server's, the
//! answer is "wait as you always did".

unsafe extern "C" {
    fn mmap(address: *mut u8, length: usize, protection: i32, flags: i32, fd: i32, offset: i64) -> *mut u8;
    fn munmap(address: *mut u8, length: usize) -> i32;
    fn mprotect(address: *mut u8, length: usize, protection: i32) -> i32;
}

/// The size of a stack, and of the part at its low end that cannot be touched, so that
/// a handler that calls itself without end is stopped and does not write over another.
pub const STACK: usize = 8 << 20;
pub const GUARD: usize = 1 << 16;

/// Leaves the stack that is running for another: what a function must find unchanged
/// when a call returns is put on the stack, the place of the stack is written at
/// `from`, and the same is taken back from the stack at `to`. The call returns on the
/// other stack, in whatever left it by a call of this function.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub unsafe extern "C" fn switch(from: *mut usize, to: usize) {
    core::arch::naked_asm!(
        "sub sp, sp, #176",
        "stp x19, x20, [sp, #0]",
        "stp x21, x22, [sp, #16]",
        "stp x23, x24, [sp, #32]",
        "stp x25, x26, [sp, #48]",
        "stp x27, x28, [sp, #64]",
        "stp x29, x30, [sp, #80]",
        "stp d8, d9, [sp, #96]",
        "stp d10, d11, [sp, #112]",
        "stp d12, d13, [sp, #128]",
        "stp d14, d15, [sp, #144]",
        "mov x9, sp",
        "str x9, [x0]",
        "mov sp, x1",
        "ldp x19, x20, [sp, #0]",
        "ldp x21, x22, [sp, #16]",
        "ldp x23, x24, [sp, #32]",
        "ldp x25, x26, [sp, #48]",
        "ldp x27, x28, [sp, #64]",
        "ldp x29, x30, [sp, #80]",
        "ldp d8, d9, [sp, #96]",
        "ldp d10, d11, [sp, #112]",
        "ldp d12, d13, [sp, #128]",
        "ldp d14, d15, [sp, #144]",
        "add sp, sp, #176",
        "ret",
    )
}

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub unsafe extern "C" fn switch(from: *mut usize, to: usize) {
    core::arch::naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}

/// Makes a stack on which `entry` will run when the stack is first switched to. Returns
/// where the stack's memory begins and what to switch to.
pub unsafe fn new_stack(entry: extern "C" fn() -> !) -> Option<(*mut u8, usize)> {
    unsafe {
        // Private, of no file, and on Linux not counted against what the system has
        // promised until it is touched: a stack is as large as a thread's own, so that
        // a handler has the same room wherever it runs, and uses a small part of it.
        let anonymous = if cfg!(target_os = "macos") { 0x1002 } else { 0x4022 };
        let base = mmap(core::ptr::null_mut(), STACK, 3, anonymous, -1, 0);
        if base.is_null() || base as isize == -1 {
            return None;
        }
        mprotect(base, GUARD, 0);
        let top = (base as usize + STACK) & !15;
        // What `switch` takes from a stack, as if `entry` had just called it: every
        // register nothing, and the place to return to the beginning of `entry`.
        let (frame, returns_at) = if cfg!(target_arch = "aarch64") {
            (176, 88)
        } else {
            // Six registers and the address; and one word more, so that the stack is
            // where a call leaves it when `entry` begins.
            (64, 48)
        };
        let sp = top - frame;
        core::ptr::write_bytes(sp as *mut u8, 0, frame);
        *((sp + returns_at) as *mut usize) = entry as usize;
        Some((base, sp))
    }
}

pub unsafe fn free_stack(base: *mut u8) {
    unsafe { munmap(base, STACK) };
}

/// The server's answers to the two questions, when a server runs handlers side by side.
pub static mut WAIT: Option<unsafe fn(i32, bool, i64) -> Waited> = None;
pub static mut PAUSE: Option<unsafe fn(i64) -> bool> = None;

/// How a wait ended.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Waited {
    /// What was waited for has come.
    Came,
    /// The time ran out first.
    TimeUp,
    /// There was no waiting here: the caller waits as it does when no server runs.
    NotHere,
}

/// Asked before a read (or, with `write`, a write) that may have to wait: the thread
/// does other work until `fd` has something to read (or room to write), or until the
/// time `until`, on the clock that only goes forward, in nanoseconds; 0 for no limit.
pub unsafe fn wait(fd: i32, write: bool, until: i64) -> Waited {
    unsafe {
        match WAIT {
            Some(wait) => wait(fd, write, until),
            None => Waited::NotHere,
        }
    }
}

/// Asked before a read that may have to wait. Returns false only if the time ran out;
/// when it returns true the caller reads, which may still wait, as when no server runs.
pub unsafe fn wait_readable(fd: i32, until: i64) -> bool {
    unsafe { wait(fd, false, until) != Waited::TimeUp }
}

/// Asked in place of a sleep: the thread does other work for so many nanoseconds.
/// Returns whether it did; if not, the caller sleeps as it always did.
pub unsafe fn pause(nanos: i64) -> bool {
    unsafe {
        match PAUSE {
            Some(pause) => pause(nanos),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static mut HOME: usize = 0;
    static mut AWAY: usize = 0;
    static mut SEEN: [u64; 8] = [0; 8];
    static mut ROUNDS: usize = 0;

    /// What runs on the other stack: it keeps a number and a fraction from one visit to
    /// the next, which the switch must give back to it as it left them.
    extern "C" fn away() -> ! {
        unsafe {
            let (mut total, mut part) = (1u64, 0.5f64);
            loop {
                total = total.wrapping_mul(31).wrapping_add(ROUNDS as u64);
                part *= 1.5;
                SEEN[ROUNDS] = total ^ part.to_bits();
                ROUNDS += 1;
                switch(&raw mut AWAY, HOME);
            }
        }
    }

    #[test]
    fn a_stack_is_left_and_come_back_to() {
        unsafe {
            let (base, sp) = new_stack(away).expect("a stack");
            AWAY = sp;
            let (mut kept, mut ratio) = (7u64, 1.0f64);
            for round in 0..8u64 {
                switch(&raw mut HOME, AWAY);
                kept = kept * 3 + round;
                ratio *= 1.25;
            }
            let (rounds, seen) = (ROUNDS, SEEN);
            assert_eq!(rounds, 8);
            let (mut total, mut part) = (1u64, 0.5f64);
            for (round, seen) in seen.iter().enumerate() {
                total = total.wrapping_mul(31).wrapping_add(round as u64);
                part *= 1.5;
                assert_eq!(
                    *seen,
                    total ^ part.to_bits(),
                    "what the other stack kept, at visit {round}"
                );
            }
            let (mut expected, mut expected_ratio) = (7u64, 1.0f64);
            for round in 0..8u64 {
                expected = expected * 3 + round;
                expected_ratio *= 1.25;
            }
            assert_eq!((kept, ratio), (expected, expected_ratio), "what this stack kept");
            free_stack(base);
        }
    }
}
