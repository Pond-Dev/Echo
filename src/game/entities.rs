//! Entity address arithmetic using the layout scanned from the running game.

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

pub const CONTROLLER_INDICES: std::ops::RangeInclusive<u32> = 1..=64;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Layout {
    pub chunk_pointer_base: usize,
    pub chunk_pointer_stride: usize,
    pub entry_stride: usize,
    pub chunk_size: u32,
    pub chunk_count: u32,
    pub handle_index_mask: u32,
}

impl Layout {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.chunk_size.is_power_of_two() && self.chunk_count > 0,
            "invalid entity chunk dimensions"
        );
        let capacity = self
            .chunk_size
            .checked_mul(self.chunk_count)
            .context("entity capacity overflow")?;
        ensure!(
            capacity > 64 && self.handle_index_mask == capacity - 1,
            "entity handle mask does not match capacity"
        );
        ensure!(
            self.chunk_pointer_stride == 8
                && self.entry_stride >= 8
                && self.entry_stride <= 0x1000
                && self.chunk_pointer_base <= 0x1000,
            "invalid entity pointer/entry layout"
        );
        Ok(())
    }

    pub fn chunk_pointer(self, system: usize, index: u32) -> Option<usize> {
        if index == 0 || index >= self.chunk_size.checked_mul(self.chunk_count)? {
            return None;
        }
        system.checked_add(self.chunk_pointer_base)?.checked_add(
            self.chunk_pointer_stride
                .checked_mul((index / self.chunk_size) as usize)?,
        )
    }

    pub fn entry_offset(self, index: u32) -> usize {
        self.entry_stride * (index % self.chunk_size) as usize
    }

    pub fn handle_index(self, raw: u32) -> Option<u32> {
        if raw == 0 || raw == u32::MAX {
            return None;
        }
        let index = raw & self.handle_index_mask;
        (index != 0).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: Layout = Layout {
        chunk_pointer_base: 0x10,
        chunk_pointer_stride: 8,
        entry_stride: 0x70,
        chunk_size: 512,
        chunk_count: 64,
        handle_index_mask: 0x7fff,
    };
    const CAPACITY: u32 = 512 * 64;
    const SYSTEM: usize = 0x1000;

    #[test]
    fn the_first_chunk_holds_the_low_indices_that_player_controllers_use() {
        // Every controller index shares chunk zero, so they all read their
        // chunk pointer from the same place.
        for index in CONTROLLER_INDICES {
            assert_eq!(LAYOUT.chunk_pointer(SYSTEM, index), Some(SYSTEM + 0x10));
        }
        assert_eq!(LAYOUT.entry_offset(1), 0x70);
        assert_eq!(LAYOUT.entry_offset(64), 0x70 * 64);
    }

    #[test]
    fn crossing_a_chunk_boundary_moves_to_the_next_pointer_and_restarts_the_entries() {
        assert_eq!(LAYOUT.chunk_pointer(SYSTEM, 511), Some(SYSTEM + 0x10));
        assert_eq!(LAYOUT.entry_offset(511), 0x70 * 511);

        assert_eq!(LAYOUT.chunk_pointer(SYSTEM, 512), Some(SYSTEM + 0x18));
        assert_eq!(
            LAYOUT.entry_offset(512),
            0,
            "a new chunk starts at its own base"
        );

        assert_eq!(LAYOUT.chunk_pointer(SYSTEM, 1024), Some(SYSTEM + 0x20));
    }

    #[test]
    fn index_zero_and_anything_past_the_table_name_no_slot() {
        assert_eq!(
            LAYOUT.chunk_pointer(SYSTEM, 0),
            None,
            "zero means no entity"
        );
        assert_eq!(LAYOUT.chunk_pointer(SYSTEM, CAPACITY), None);
        assert_eq!(LAYOUT.chunk_pointer(SYSTEM, u32::MAX), None);
        // The last slot lives in chunk 63: the table base plus 63 pointers.
        assert_eq!(
            LAYOUT.chunk_pointer(SYSTEM, CAPACITY - 1),
            Some(SYSTEM + 0x208)
        );
        assert_eq!(LAYOUT.entry_offset(CAPACITY - 1), 0x70 * 511);
    }

    #[test]
    fn a_handle_yields_its_index_and_the_empty_handles_yield_nothing() {
        // Serial in the high bits, index in the low fifteen.
        assert_eq!(LAYOUT.handle_index(0x0001_8001), Some(1));
        assert_eq!(LAYOUT.handle_index(0x7FFF), Some(0x7FFF));

        assert_eq!(LAYOUT.handle_index(0), None, "a null handle");
        assert_eq!(LAYOUT.handle_index(u32::MAX), None, "the invalid handle");
        assert_eq!(
            LAYOUT.handle_index(0xFFFF_8000),
            None,
            "a serial with a zero index still names nothing"
        );
    }
}
