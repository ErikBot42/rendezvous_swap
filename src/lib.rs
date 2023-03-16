#![no_std]
#![warn(missing_docs)]
#![allow(clippy::implicit_return)]
#![allow(clippy::semicolon_inside_block)]
#![allow(clippy::blanket_clippy_restriction_lints)]
//! A rendezvous is an execution barrier between a pair of threads, but this crate also provides the option of swapping data at the synchronisation point. (Terminology is from [The Little Book of Semaphores](https://greenteapress.com/wp/semaphores/))
//!
//! This is mainly intended for situations where threads sync frequently. Unlike a normal spinlock, it does not use any CAS instructions, just [`Acquire`] loads and [`Release`] stores which means it can compile to just a handful of non atomic instructions on `x86_64`.
//!
//! Data is internally swapped with pointers, so large structures are not costly to swap and therefore does not need to be boxed.
//!
//! In microbenchmarks on my machine, it takes less than `200 ns` to swap data and less than `100 ns` to sync execution.
//!
//!
//! # Example: Sync thread execution
//! ```rust
//! use rendezvous_swap::Rendezvous;
//! use std::thread;
//!
//! let (mut my_rendezvous, mut their_rendezvous) = Rendezvous::new();
//! thread::spawn(move || {
//!     for i in 1..5 {
//!         println!("{i}");
//!         their_rendezvous.wait();
//!     }
//! });
//! for i in 1..5 {
//!     println!("{i}");
//!     my_rendezvous.wait();
//! }
//! ```
//! this prints:
//! ```text
//! 1
//! 1
//! 2
//! 2
//! 3
//! 3
//! 4
//! 4
//! ```
//! # Example: Swap thread data
//! ```rust
//! use std::thread;
//! use rendezvous_swap::RendezvousData;
//!
//! let (mut my_rendezvous, mut their_rendezvous) = RendezvousData::new(0, 0);
//! let handle = thread::spawn(move || {
//!     let borrow = their_rendezvous.swap();
//!     *borrow = 3;
//!     
//!     let borrow = their_rendezvous.swap();
//!     assert_eq!(7, *borrow);
//! });
//! let borrow = my_rendezvous.swap();
//! *borrow = 7;
//!
//! let borrowed_data = my_rendezvous.swap();
//! assert_eq!(3, *borrowed_data);
//!
//! # handle.join().unwrap();
//! ```
//! # Example: Safety
//! The following won't compile due to the limited lifetime of the references provided by [`RendezvousData::swap`], you will get the familiar lifetime errors as if you are borrowing a struct element. This crate is safe because it's impossible for both threads to have mutabeĺe references to the same memory location at the same time. 
//! ```compile_fail
//! use std::thread;
//! use rendezvous_swap::RendezvousData;
//!
//! let (mut my_rendezvous, mut their_rendezvous) = RendezvousData::new(0, 0);
//! let handle = thread::spawn(move || {
//!     their_rendezvous.swap(); // swap return values can be ignored
//!     their_rendezvous.swap();
//! });
//! let old_borrow = my_rendezvous.swap(); // first mutable borrow occurs here
//!
//! let new_borrow = my_rendezvous.swap(); // second mutable borrow occurs here
//! 
//! *old_borrow = 3; // first borrow is later used here
//!
//! # handle.join().unwrap();
//! ```

extern crate alloc;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::mem::swap;
use core::ops::Deref;
use core::pin::Pin;
use core::ptr::NonNull;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::{Acquire, Release};

#[derive(Debug)]
#[repr(align(128))] // Alignment of 128 marginally faster on x86_64
/// Pad data so it is aligned to cache line (currently hard coded to 128 bytes)
struct Padded<T> {
    /// Inner data
    pub i: T,
}
impl<T> Padded<T> {
    /// Constructs a new [`Padded`]
    const fn new(i: T) -> Self {
        Self { i }
    }
}
impl<T> Deref for Padded<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.i
    }
}

/// A pointer to this will be shared for the two [`RendezvousData`]
/// Note that this has no indirection.
struct RendezvousDataShared<T: Sync> {
    /// First counter
    c1: Padded<AtomicUsize>,
    /// Second counter
    c2: Padded<AtomicUsize>,
    /// First shared data (not a pointer)
    p1: Padded<UnsafeCell<T>>,
    /// Second shared data (not a pointer)
    p2: Padded<UnsafeCell<T>>,
}
impl<T: Sync> RendezvousDataShared<T> {
    /// Constructs a new [`RendezvousDataShared`] from the provided data
    fn new(data1: T, data2: T) -> Self {
        Self {
            c1: Padded::new(AtomicUsize::new(0)),
            c2: Padded::new(AtomicUsize::new(0)),
            p1: Padded::new(UnsafeCell::new(data1)),
            p2: Padded::new(UnsafeCell::new(data2)),
        }
    }
}

/// Synchronise execution and swap data between threads.
#[non_exhaustive]
pub struct RendezvousData<T: Sync> {
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
impl<T: Sync> RendezvousData<T> {
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

    /// Allways inlined version of [`RendezvousData::swap`]
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
        // is destryed after calling wait(), and we can therefore create
        // a new mutable reference to that data without causing UB
        unsafe { &mut *(self.data.0.as_ref()).get() }
    }

    /// Synchronize execution with other thread.
    /// As a side-effect, memory is also synchronized.
    ///
    /// # SAFETY
    /// If number of swaps gets out of sync, muliple mutable references to the same
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
// SAFETY:
// The act of sending pointers between threads is not unsafe.
// UnsafeCell requires special consideration
unsafe impl<T: Sync> Send for RendezvousData<T> {}
// where Pin<Arc<RendezvousDataShared<T>>>: Send {}

/// Synchronise execution between threads.
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

    /// Allways inlined version of [`Rendezvous::wait`]
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
