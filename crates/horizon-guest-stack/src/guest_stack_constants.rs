//! How much stack a thread that may enter a guest reserves.
//!
//! A 64-bit process has room for the guest's full recursion depth. A 32-bit one does not: its
//! user address space is 2 GiB, and a game that starts a dozen worker threads would reserve
//! more than all of it at 128 MiB each (measured: Wine i686 failed to spawn the loot loader and
//! the resource loader with `OutOfMemory`). The 32-bit figure keeps every reservation small
//! enough that the game's whole thread set fits, at the cost of a shallower guest stack.

/// At the reporter's measured ceiling of about 1.8 KB per frame, 128 MiB covers about 70k plain
/// frames, including Java's deepest reported 59k case.
#[cfg(target_pointer_width = "64")]
pub const GUEST_STACK_BYTES: usize = 128 * 1024 * 1024;
/// The same 1.8 KB frames reach about 9k deep: a tenth of Java's reported depth, but it is what
/// the 32-bit address space can hold for every worker at once.
#[cfg(not(target_pointer_width = "64"))]
pub const GUEST_STACK_BYTES: usize = 16 * 1024 * 1024;

/// Room for Rust host frames below the guest and synchronous host imports above it.
#[cfg(target_pointer_width = "64")]
pub const HOST_HEADROOM: usize = 8 * 1024 * 1024;
#[cfg(not(target_pointer_width = "64"))]
pub const HOST_HEADROOM: usize = 2 * 1024 * 1024;

/// What a guest-capable thread reserves, and what the executables' link settings declare.
pub const GUEST_RESERVATION_BYTES: usize = GUEST_STACK_BYTES + HOST_HEADROOM;
