# rendezvous_swap

A rendezvous is an execution barrier between a pair of threads, but this crate also provides the option of swapping data at the synchronisation point. (Terminology is from [The Little Book of Semaphores](https://greenteapress.com/wp/semaphores/))

This is mainly intended for situations where threads sync frequently. Unlike a normal spinlock, it does not use any CAS instructions, just [`Acquire`] loads and [`Release`] stores which means it can compile to just a handful of non atomic instructions on `x86_64`. Because the crate uses atomics for synchronisation, it is also `no_std`.

Data is internally swapped with pointers, so large structures are not costly to swap and therefore do not need to be boxed.

In microbenchmarks on a `i5-7200U` CPU, it takes less than `100 ns` to swap data.

## Safety
[`RendezvousData`] contains `unsafe` but all tests pass when running with Miri.

## Example: Sync thread execution
```rust
use rendezvous_swap::Rendezvous;
use std::thread;

let (mut my_rendezvous, mut their_rendezvous) = Rendezvous::new();
thread::spawn(move || {
    for i in 1..5 {
        println!("{i}");
        their_rendezvous.wait();
    }
});
for i in 1..5 {
    println!("{i}");
    my_rendezvous.wait();
}
```
This prints:
```
1
1
2
2
3
3
4
4
```
## Example: Swap thread data
```rust
use std::thread;
use rendezvous_swap::RendezvousData;

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

```
## Example: Safety
The following won't compile due to the limited lifetime of the references provided by [`RendezvousData::swap`], you will get the familiar lifetime errors as if you are borrowing a struct element. This crate is safe because it is not possible for both threads to have mutable references to the same memory location at the same time.
```rust
use std::thread;
use rendezvous_swap::RendezvousData;

let (mut my_rendezvous, mut their_rendezvous) = RendezvousData::new(0, 0);
let handle = thread::spawn(move || {
    their_rendezvous.swap(); // swap return values can be ignored
    their_rendezvous.swap();
});
let old_borrow = my_rendezvous.swap(); // first mutable borrow occurs here

let new_borrow = my_rendezvous.swap(); // second mutable borrow occurs here

*old_borrow = 3; // first borrow is later used here

```

## Current version: 0.1.0

## License: GPL-3.0
