#![warn(missing_docs)]
#![allow(clippy::inline_always)]
//! A rendezvous is an execution barrier between a pair of threads, but this crate also provides the option of
//! swapping data at the synchronisation point.
//! (Terminology is from [The Little Book of Semaphores](https://greenteapress.com/wp/semaphores/))
//!
//! This is mainly intended for situations where threads sync frequently.
//! Unlike a normal spinlock, it does not use any CAS instructions, just [`Acquire`] loads and [`Release`] stores which means it
//! can compile to just a handful of non atomic instructions on `x86_64`.
//!
//! Data is internally swapped with pointers, so large structures are not costly to swap and
//! therefore does not need to be boxed.
//!
//! In microbenchmarks on my machine, it takes less than `200 ns` to swap data and less than `100 ns` to sync
//! execution.
//!
//! # Usage
//! ## Sync thread execution
//! ```rust
//! use rendezvous_swap::Rendezvous;
//! use std::thread;
//!
//!    let (mut my_rendezvous, mut their_rendezvous) = Rendezvous::new();
//!    thread::spawn(move || {
//!        for i in 1..5 {
//!            println!("{i}");
//!            their_rendezvous.wait();
//!        }
//!    });
//!    for i in 1..5 {
//!        println!("{i}");
//!        my_rendezvous.wait();
//!    }
//! ```
//! prints:
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
//! ## Swap thread data
//!```rust
//!  use std::thread;
//!  use rendezvous_swap::RendezvousData;
//!
//!  let (mut my_rendezvous, mut their_rendezvous) = RendezvousData::new(0, 0);
//!  let handle = thread::spawn(move || {
//!      let borrow = their_rendezvous.swap();
//!      *borrow = 3;
//!      
//!      let borrow = their_rendezvous.swap();
//!      assert_eq!(7, *borrow);
//!  });
//!  let borrow = my_rendezvous.swap();
//!  *borrow = 7;
//!
//!  let borrowed_data = my_rendezvous.swap();
//!  assert_eq!(3, *borrowed_data);
//!
//!  # handle.join().unwrap();
//!```
//!
#[test]
fn equivalent_doctest() {
    use std::thread;
    let (mut my_rendezvous, mut their_rendezvous) = Rendezvous::new();
    thread::spawn(move || {
        for i in 1..5 {
            println!("{i} their thead");
            their_rendezvous.wait();
        }
    });
    for i in 1..5 {
        println!("{i} my thead");
        my_rendezvous.wait();
    }
}

#[test]
fn equivalent_doctest2() {
    use std::thread;

    let (mut my_rendezvous, mut their_rendezvous) = RendezvousData::new(0, 0);
    let handle = thread::spawn(move || {
        let borrow = their_rendezvous.swap();
        *borrow = 3;

        let borrow = their_rendezvous.swap();
        assert_eq!(7, *borrow);
    });
    let borrow = my_rendezvous.swap();
    *borrow = 7;

    let borrowed_data = my_rendezvous.swap();
    assert_eq!(3, *borrowed_data);

    handle.join().unwrap();
}

use std::cell::UnsafeCell;
use std::mem::transmute;
use std::ops::Deref;
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{Acquire, Release};
use std::sync::Arc;

#[derive(Debug)]
#[repr(align(128))] // marginally faster
struct Padded<T> {
    pub i: T,
}
impl<T> Padded<T> {
    fn new(i: T) -> Self {
        Self { i }
    }
}
impl<T> Deref for Padded<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.i
    }
}

/// Synchronise execution and swap data between threads.
#[non_exhaustive]
pub struct RendezvousData<T: Sync> {
    // thead local
    generation: usize,

    // synced using atomics
    my_counter: NonNull<AtomicUsize>,
    their_counter: NonNull<AtomicUsize>,

    // unsafe mutable, need sync to enforce correctness
    data: (NonNull<UnsafeCell<T>>, NonNull<UnsafeCell<T>>),

    /// Let Arc handle dropping shared data so that everything is alive long enough
    /// TODO: decide on cache stuff
    _handle: Pin<
        Arc<(
            Padded<AtomicUsize>,
            Padded<AtomicUsize>,
            Padded<UnsafeCell<T>>,
            Padded<UnsafeCell<T>>,
        )>,
    >,
}
impl<T: Sync> RendezvousData<T> {
    #[must_use]
    /// Create a linked pair of [`RendezvousData`]
    /// Arguments are the initial values for the data that will be swapped.
    pub fn new(data1: T, data2: T) -> (Self, Self) {
        let a = Arc::pin((
            Padded::new(AtomicUsize::new(0)),
            Padded::new(AtomicUsize::new(0)),
            Padded::new(UnsafeCell::new(data1)),
            Padded::new(UnsafeCell::new(data2)),
        ));

        //let b: *mut T = addr_of_mut!(a.2.i);

        let p1: NonNull<UnsafeCell<T>> = (&a.2.i).into();
        let p2: NonNull<UnsafeCell<T>> = (&a.3.i).into();

        (
            Self {
                generation: 0,
                my_counter: (&a.0.i).into(),
                their_counter: (&a.1.i).into(),
                data: (p1, p2),
                _handle: a.clone(),
            },
            Self {
                generation: 0,
                my_counter: (&a.1.i).into(),
                their_counter: (&a.0.i).into(),
                data: (p2, p1),
                _handle: a.clone(),
            },
        )
    }
    /// Swap data with other thread and get a mutable reference to the data.
    pub fn swap<'a>(&'a mut self) -> &'a mut T {
        self.wait();

        // Swap the **pointers** to the underlying data.
        std::mem::swap(&mut self.data.0, &mut self.data.1);

        // SAFETY: we know that the mutable reference int the other thread
        // is destryed after calling wait(), and we can therefore create
        // a new mutable reference to that data without causing UB
        unsafe { transmute((self.data.0.as_ref()).get()) }
    }
    #[inline(always)]
    fn wait(&mut self) {
        let next_generation = self.generation.wrapping_add(1);
        unsafe { self.my_counter.as_ref() }.store(next_generation, Release);
        while {
            // Signal to processor (not OS) that we are in a spinloop.
            // Performance seems to improve by a tiny bit with this.
            std::hint::spin_loop();
            unsafe { self.their_counter.as_ref() }.load(Acquire) == self.generation
        } {}
        self.generation = next_generation;
    }
}
unsafe impl<T: Sync> Send for RendezvousData<T> {}

/// Synchronise execution between threads.
#[non_exhaustive]
pub struct Rendezvous {
    my_counter: Arc<AtomicUsize>,
    their_counter: Arc<AtomicUsize>,
    generation: usize,
}
impl Rendezvous {
    /// Synchronize execution with other thread.
    ///
    /// As a side-effect, memory is also synchronized.
    #[inline(never)]
    pub fn wait(&mut self) {
        self.wait_inline();
    }

    /// Inlined version of [`Rendezvous::wait`]
    #[inline(always)]
    pub fn wait_inline(&mut self) {
        let next_generation = self.generation.wrapping_add(1);
        self.my_counter.store(next_generation, Release);
        while {
            // Signal to processor (not OS) that we are in a spinloop.
            // Performance seems to improve by a tiny bit with this.
            std::hint::spin_loop();
            self.their_counter.load(Acquire) == self.generation
        } {}
        self.generation = next_generation;
    }
    /// Create a linked pair of [`Rendezvous`]
    pub fn new() -> (Self, Self) {
        let first = Arc::new(AtomicUsize::new(0));
        let second = Arc::new(AtomicUsize::new(0));
        (
            Rendezvous {
                my_counter: first.clone(),
                their_counter: second.clone(),
                generation: 0,
            },
            Rendezvous {
                my_counter: second,
                their_counter: first,
                generation: 0,
            },
        )
    }
}
