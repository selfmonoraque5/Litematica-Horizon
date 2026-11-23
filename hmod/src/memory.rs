//! How much memory a schematic needs to load, and whether the computer has it: no file is too
//! large in itself, but one that likely won't fit is loaded only once the player asks again.
//!
//! Linux tells the memory still free (`/proc/meminfo`). Elsewhere only the computer's whole RAM
//! is known, from the game (`Total`, given by `src/native/game_support.rs`).

use std::path::Path;
use std::sync::OnceLock;

/// The computer's RAM in bytes, where the game knows it.
pub type Total = fn() -> Option<u64>;

static TOTAL: OnceLock<Total> = OnceLock::new();

/// From the game side, once.
pub fn install(total: Total) {
    let _ = TOTAL.set(total);
}

/// What is known of the computer's memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Room {
    /// The memory still free (and so what a load may take).
    Free(u64),
    /// Only the whole RAM: the game itself and every other program hold some of it.
    Total(u64),
}

impl Room {
    pub fn now() -> Option<Room> {
        free().map(Room::Free).or_else(|| TOTAL.get()?().map(Room::Total))
    }

    /// Whether `need` bytes likely fit: all of what is free, or half the RAM where only that is
    /// known.
    pub fn fits(self, need: u64) -> bool {
        match self {
            Room::Free(free) => need <= free,
            Room::Total(total) => need <= total / 2,
        }
    }
}

/// `MemAvailable` from `/proc/meminfo`, in bytes.
#[cfg(target_os = "linux")]
fn free() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(not(target_os = "linux"))]
fn free() -> Option<u64> {
    None
}

/// About the most memory reading `path` takes at once: the file itself, its unpacked contents
/// and the block arrays read out of them (about as large again).
pub fn need(path: &Path) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = [0u8; 2];
    f.read_exact(&mut head).ok()?;
    let mut tail = [0u8; 4];
    if len >= 4 {
        f.seek(SeekFrom::End(-4)).ok()?;
        f.read_exact(&mut tail).ok()?;
    }
    let unpacked = blazematica_core::nbt::unpacked_size(&head, tail, len);
    Some(len.saturating_add(unpacked.saturating_mul(2)))
}
