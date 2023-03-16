//! Contains [`Rendezvous`]
#![forbid(unsafe_code)]

use alloc::sync::Arc;
use core::hint::spin_loop;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::{Acquire, Release};
/// Synchronise execution between threads.
/// # Example: Sync thread execution
/// ```rust
/// use rendezvous_swap::Rendezvous;
/// use std::thread;
///
/// let (mut my_rendezvous, mut their_rendezvous) = Rendezvous::new();
/// thread::spawn(move || {
///     for i in 1..5 {
///         println!("{i}");
///         their_rendezvous.wait();
///     }
/// });
/// for i in 1..5 {
///     println!("{i}");
///     my_rendezvous.wait();
/// }
/// ```
/// This prints:
/// ```text
/// 1
/// 1
/// 2
/// 2
/// 3
/// 3
/// 4
/// 4
/// ```
#[non_exhaustive]
pub struct Rendezvous {
    /// Atomic counter for this thread
    my_counter: Arc<AtomicUsize>,
    /// Atomic counter for other thread
    their_counter: Arc<AtomicUsize>,
    /// Thread local generation
    generation: usize,
}
impl Rendezvous {
    /// Synchronize execution with other thread.
    ///
    /// As a side-effect, memory is also synchronized.
    #[inline]
    pub fn wait(&mut self) {
        self.wait_inline();
    }

    /// Always inlined version of [`Rendezvous::wait`]
    #[allow(clippy::inline_always)]
    #[inline(always)]
    pub fn wait_inline(&mut self) {
        let next_generation = self.generation.wrapping_add(1);
        self.my_counter.store(next_generation, Release);
        while {
            // Signal to processor (not OS) that we are in a spinloop.
            // Performance seems to improve by a tiny bit with this.
            spin_loop();
            self.their_counter.load(Acquire) == self.generation
        } {}
        self.generation = next_generation;
    }
    /// Create a linked pair of [`Rendezvous`]
    #[must_use]
    #[inline]
    pub fn new() -> (Self, Self) {
        let first = Arc::new(AtomicUsize::new(0));
        let second = Arc::new(AtomicUsize::new(0));
        (
            Self {
                my_counter: Arc::clone(&first),
                their_counter: Arc::clone(&second),
                generation: 0,
            },
            Self {
                my_counter: second,
                their_counter: first,
                generation: 0,
            },
        )
    }
}
