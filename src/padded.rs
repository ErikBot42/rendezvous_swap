//! Contains [`Padded`]
#![forbid(unsafe_code)]

use core::ops::Deref;

#[derive(Debug)]
#[repr(align(128))] // Alignment of 128 marginally faster on x86_64
/// Pad data so it is aligned to cache line (currently hard coded to 128 bytes)
pub struct Padded<T> {
    /// Inner data
    pub i: T,
}
impl<T> Padded<T> {
    /// Constructs a new [`Padded`]
    pub const fn new(i: T) -> Self {
        Self { i }
    }
}
impl<T> Deref for Padded<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.i
    }
}
