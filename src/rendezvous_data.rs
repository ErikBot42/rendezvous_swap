//! Contains [`RendezvousData`]

use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::mem::swap;
use core::pin::Pin;
use core::ptr::NonNull;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::{Acquire, Release};

use crate::padded::Padded;

/// A pointer to this will be shared for the two [`RendezvousData`]
/// Note that this has no indirection.
struct RendezvousDataShared<T: Send + Sync> {
    /// First counter
    c1: Padded<AtomicUsize>,
    /// Second counter
    c2: Padded<AtomicUsize>,
    /// First shared data (not a pointer)
    p1: Padded<UnsafeCell<T>>,
    /// Second shared data (not a pointer)
    p2: Padded<UnsafeCell<T>>,
}
// SAFETY:
// UnsafeCell needs special consideration
unsafe impl<T: Send + Sync> Sync for RendezvousDataShared<T> {}
impl<T: Send + Sync> RendezvousDataShared<T> {
    /// Constructs a new [`RendezvousDataShared`] from the provided data
    const fn new(data1: T, data2: T) -> Self {
        Self {
            c1: Padded::new(AtomicUsize::new(0)),
            c2: Padded::new(AtomicUsize::new(0)),
            p1: Padded::new(UnsafeCell::new(data1)),
            p2: Padded::new(UnsafeCell::new(data2)),
        }
    }
}

/// Synchronise execution and swap data between threads.
/// # Example: Swap thread data
/// ```rust
/// use std::thread;
/// use rendezvous_swap::RendezvousData;
///
/// let (mut my_rendezvous, mut their_rendezvous) = RendezvousData::new(0, 0);
/// let handle = thread::spawn(move || {
///     let borrow = their_rendezvous.swap();
///     *borrow = 3;
///     
///     let borrow = their_rendezvous.swap();
///     assert_eq!(7, *borrow);
/// });
/// let borrow = my_rendezvous.swap();
/// *borrow = 7;
///
/// let borrowed_data = my_rendezvous.swap();
/// assert_eq!(3, *borrowed_data);
///
/// # handle.join().unwrap();
/// ```
/// # Example: Safety
/// The following won't compile due to the limited lifetime of the references provided by [`RendezvousData::swap`], you will get the familiar lifetime errors as if you are borrowing a struct element. This crate is safe because it is not possible for both threads to have mutable references to the same memory location at the same time.
/// ```compile_fail
/// use std::thread;
/// use rendezvous_swap::RendezvousData;
///
/// let (mut my_rendezvous, mut their_rendezvous) = RendezvousData::new(0, 0);
/// let handle = thread::spawn(move || {
///     their_rendezvous.swap(); // swap return values can be ignored
///     their_rendezvous.swap();
/// });
/// let old_borrow = my_rendezvous.swap(); // first mutable borrow occurs here
///
/// let new_borrow = my_rendezvous.swap(); // second mutable borrow occurs here
///
/// *old_borrow = 3; // first borrow is later used here
///
/// # handle.join().unwrap();
/// ```
#[non_exhaustive]
pub struct RendezvousData<T: Send + Sync> {
    /// Thread local generation
    generation: usize,

    /// Atomic counter for this thread
    my_counter: NonNull<AtomicUsize>,

    /// Atomic counter for other thread
    their_counter: NonNull<AtomicUsize>,

    /// A pair of pointers to the underlying data.
    /// Needs sync to enforce correctness
    data: (NonNull<UnsafeCell<T>>, NonNull<UnsafeCell<T>>),

    /// Let Arc handle dropping shared data so that everything is alive long enough
    /// TODO: decide on cache stuff
    _handle: Pin<Arc<RendezvousDataShared<T>>>,
}
// SAFETY:
// The act of sending pointers between threads is not unsafe.
// UnsafeCell requires special consideration
unsafe impl<T: Sync + Send> Send for RendezvousData<T> {}
impl<T: Send + Sync> RendezvousData<T> {
    /// Create a linked pair of [`RendezvousData`]
    /// Arguments are the initial values for the data that will be swapped.
    #[must_use]
    #[inline]
    pub fn new(data1: T, data2: T) -> (Self, Self) {
        let a = Arc::pin(RendezvousDataShared::new(data1, data2));

        let p1: NonNull<UnsafeCell<T>> = (&*a.p1).into();
        let p2: NonNull<UnsafeCell<T>> = (&*a.p2).into();
        (
            Self {
                generation: 0,
                my_counter: (&*a.c1).into(),
                their_counter: (&*a.c2).into(),
                data: (p1, p2),
                _handle: a.clone(),
            },
            Self {
                generation: 0,
                my_counter: (&*a.c2).into(),
                their_counter: (&*a.c1).into(),
                data: (p2, p1),
                _handle: a.clone(),
            },
        )
    }
    /// Swap data with other thread and get a mutable reference to the data.
    #[allow(clippy::needless_lifetimes)] // lifetime needs to be restricted here
    #[inline]
    pub fn swap<'lock>(&'lock mut self) -> &'lock mut T {
        self.swap_inline()
    }

    /// Always inlined version of [`RendezvousData::swap`]
    #[allow(clippy::needless_lifetimes)] // lifetime needs to be restricted here
    #[allow(clippy::inline_always)]
    #[inline(always)]
    pub fn swap_inline<'lock>(&'lock mut self) -> &'lock mut T {
        // SAFETY:
        // Number of swaps must stay the same between threads
        unsafe { self.wait() };

        // Swap the **pointers** to the underlying data.
        swap(&mut self.data.0, &mut self.data.1);

        // SAFETY:
        // we know that the mutable reference in the other thread
        // is destroyed after calling wait(), and we can therefore create
        // a new mutable reference to that data without causing UB
        unsafe { &mut *(self.data.0.as_ref()).get() }
    }

    /// Synchronize execution with other thread.
    /// As a side-effect, memory is also synchronized.
    ///
    /// # SAFETY
    /// If number of swaps gets out of sync, multiple mutable references to the same
    /// memory is created
    #[allow(clippy::inline_always)]
    #[inline(always)]
    unsafe fn wait(&mut self) {
        let next_generation = self.generation.wrapping_add(1);

        // SAFETY:
        // Pointer is valid as long as the Arc is not dropped
        unsafe { self.my_counter.as_ref() }.store(next_generation, Release);
        while {
            // Signal to processor (not OS) that we are in a spinloop.
            // Performance seems to improve by a tiny bit with this.
            spin_loop();

            // SAFETY:
            // Pointer is valid as long as the Arc is not dropped
            unsafe { self.their_counter.as_ref() }.load(Acquire) == self.generation
        } {}
        self.generation = next_generation;
    }
}
