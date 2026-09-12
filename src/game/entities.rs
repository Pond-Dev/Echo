//! Finding an entity by index.
//!
//! Entities do not sit in a flat array. The game keeps 64 chunks of 512 slots
//! each, reached through a table of chunk pointers, so getting one entity is
//! two dereferences rather than one index.
//!
//! Handles pack an index and a serial number into a single word. The serial is
//! what makes a handle safe to hold across time: reusing a slot bumps it, so a
//! handle to a freed entity stops matching. Echo does not check serials yet —
//! it re-reads everything each pass instead — but the index is masked out here
//! so the shape is right when that check arrives.
//!
//! All of this is pure address arithmetic, which is why it is testable without
//! a game.

/// Slots per chunk.
const CHUNK_SIZE: u32 = 512;
/// Chunks in the table.
const CHUNK_COUNT: u32 = 64;
/// Where the chunk pointer table starts inside the entity system.
const CHUNK_TABLE: usize = 0x10;
/// Bytes between chunk pointers.
const CHUNK_POINTER_STRIDE: usize = 0x08;
/// Bytes between entries inside a chunk.
const ENTRY_STRIDE: usize = 0x70;
/// The part of a handle that is an index; the rest is the serial.
const HANDLE_INDEX_MASK: u32 = 0x7FFF;

/// Highest entity index the table can hold.
pub const CAPACITY: u32 = CHUNK_SIZE * CHUNK_COUNT;

/// Player controllers occupy the low indices, one per connected player.
pub const CONTROLLER_INDICES: std::ops::RangeInclusive<u32> = 1..=64;

/// Address holding the pointer to the chunk that `index` lives in.
///
/// Index zero is excluded: the game uses it as "no entity", so a zero here is
/// a null handle that leaked through rather than a real slot.
pub const fn chunk_pointer(system: usize, index: u32) -> Option<usize> {
    if index == 0 || index >= CAPACITY {
        return None;
    }
    Some(system + CHUNK_TABLE + CHUNK_POINTER_STRIDE * (index / CHUNK_SIZE) as usize)
}

/// Offset of `index`'s entry within its own chunk.
pub const fn entry_offset(index: u32) -> usize {
    ENTRY_STRIDE * (index % CHUNK_SIZE) as usize
}

/// The index a handle refers to, or `None` for a handle that names nothing.
pub const fn handle_index(raw: u32) -> Option<u32> {
    if raw == 0 || raw == u32::MAX {
        return None;
    }
    let index = raw & HANDLE_INDEX_MASK;
    if index == 0 { None } else { Some(index) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSTEM: usize = 0x1000;

    #[test]
    fn the_first_chunk_holds_the_low_indices_that_player_controllers_use() {
        // Every controller index shares chunk zero, so they all read their
        // chunk pointer from the same place.
        for index in CONTROLLER_INDICES {
            assert_eq!(chunk_pointer(SYSTEM, index), Some(SYSTEM + 0x10));
        }
        assert_eq!(entry_offset(1), 0x70);
        assert_eq!(entry_offset(64), 0x70 * 64);
    }

    #[test]
    fn crossing_a_chunk_boundary_moves_to_the_next_pointer_and_restarts_the_entries() {
        assert_eq!(chunk_pointer(SYSTEM, 511), Some(SYSTEM + 0x10));
        assert_eq!(entry_offset(511), 0x70 * 511);

        assert_eq!(chunk_pointer(SYSTEM, 512), Some(SYSTEM + 0x18));
        assert_eq!(entry_offset(512), 0, "a new chunk starts at its own base");

        assert_eq!(chunk_pointer(SYSTEM, 1024), Some(SYSTEM + 0x20));
    }

    #[test]
    fn index_zero_and_anything_past_the_table_name_no_slot() {
        assert_eq!(chunk_pointer(SYSTEM, 0), None, "zero means no entity");
        assert_eq!(chunk_pointer(SYSTEM, CAPACITY), None);
        assert_eq!(chunk_pointer(SYSTEM, u32::MAX), None);
        // The last slot lives in chunk 63: the table base plus 63 pointers.
        assert_eq!(chunk_pointer(SYSTEM, CAPACITY - 1), Some(SYSTEM + 0x208));
        assert_eq!(entry_offset(CAPACITY - 1), 0x70 * 511);
    }

    #[test]
    fn a_handle_yields_its_index_and_the_empty_handles_yield_nothing() {
        // Serial in the high bits, index in the low fifteen.
        assert_eq!(handle_index(0x0001_8001), Some(1));
        assert_eq!(handle_index(0x7FFF), Some(0x7FFF));

        assert_eq!(handle_index(0), None, "a null handle");
        assert_eq!(handle_index(u32::MAX), None, "the invalid handle");
        assert_eq!(
            handle_index(0xFFFF_8000),
            None,
            "a serial with a zero index still names nothing"
        );
    }
}
