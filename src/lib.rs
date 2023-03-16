#![no_std]
#![warn(missing_docs)]
#![allow(clippy::implicit_return)]
#![allow(clippy::semicolon_inside_block)]
#![allow(clippy::blanket_clippy_restriction_lints)]
#![allow(clippy::pub_use)]
//! A rendezvous is an execution barrier between a pair of threads, but this crate also provides the option of swapping data at the synchronisation point. (Terminology is from [The Little Book of Semaphores](https://greenteapress.com/wp/semaphores/))
//!
//! This is mainly intended for situations where threads sync frequently. Unlike a normal spinlock, it does not use any CAS instructions, just [`Acquire`] loads and [`Release`] stores which means it can compile to just a handful of non atomic instructions on `x86_64`. Because the crate uses atomics for synchronisation, it is also `no_std`.
//!
//! Data is internally swapped with pointers, so large structures are not costly to swap and therefore do not need to be boxed.
//!
//! In microbenchmarks on a `i5-7200U` CPU, it takes less than `100 ns` to swap data.
//!
//! # Safety
//! [`RendezvousData`] contains `unsafe` but all tests pass when running with Miri.
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
//! This prints:
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
//! The following won't compile due to the limited lifetime of the references provided by [`RendezvousData::swap`], you will get the familiar lifetime errors as if you are borrowing a struct element. This crate is safe because it is not possible for both threads to have mutable references to the same memory location at the same time.
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
mod rendezvous_data;
mod rendezvous;

pub use rendezvous_data::RendezvousData;
pub use rendezvous::Rendezvous;
