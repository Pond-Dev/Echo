use std::collections::HashSet;

use anyhow::{Result, ensure};
use memflow::prelude::v1::*;

use super::UtlMemoryPool;

// A generous discovery ceiling, not a claimed live schema count. It bounds
// signed remote counts, allocation, and node visits even when data is null.
const MAX_HASH_ELEMENTS: usize = 65_536;

#[repr(C)]
pub struct UtlTsHashAllocatedBlob<D> {
    pub next: Pointer64<UtlTsHashAllocatedBlob<D>>, // 0x0000
    pad_0: [u8; 0x8],                               // 0x0008
    pub data: Pointer64<D>,                         // 0x0010
    pad_1: [u8; 0x18],                              // 0x0018
}

unsafe impl<D: 'static> Pod for UtlTsHashAllocatedBlob<D> {}

#[repr(C)]
pub struct UtlTsHashFixedData<D> {
    pub ui_key: u64,                            // 0x0000
    pub next: Pointer64<UtlTsHashFixedData<D>>, // 0x0008
    pub data: Pointer64<D>,                     // 0x0010
}

// SAFETY: The fixed u64 wire key and both remote pointers have no padding or
// restricted bit patterns. Other key widths are not supported by this layout.
unsafe impl<D: 'static> Pod for UtlTsHashFixedData<D> {}

#[repr(C)]
pub struct UtlTsHashBucket<D> {
    pub add_lock: u64,                                       // 0x0000
    pub first: Pointer64<UtlTsHashFixedData<D>>,             // 0x0008
    pub first_uncommitted: Pointer64<UtlTsHashFixedData<D>>, // 0x0010
}

// SAFETY: The fixed-width lock and integer-backed remote pointers have no
// restricted bit patterns or implicit padding.
unsafe impl<D: 'static> Pod for UtlTsHashBucket<D> {}

#[repr(C)]
pub struct UtlTsHash<D, const C: usize = 256> {
    pub entry_mem: UtlMemoryPool,         // 0x0000
    pub buckets: [UtlTsHashBucket<D>; C], // 0x0060
    pub needs_commit: u8,                 // 0x1860: raw remote boolean
    pad_0: [u8; 0x3],                     // 0x1861
    pub contention_check: i32,            // 0x1864
    pad_1: [u8; 0x8],                     // 0x1868
}

impl<D: Pod, const C: usize> UtlTsHash<D, C> {
    pub fn elements(&self, mem: &mut impl MemoryView) -> Result<Vec<Pointer64<D>>> {
        ensure!(self.needs_commit <= 1, "invalid hash commit boolean");
        ensure!(
            self.entry_mem.grow_mode <= 2,
            "invalid memory pool grow mode"
        );
        let used_count = checked_count(self.entry_mem.blocks_allocated)?;
        let free_count = checked_count(self.entry_mem.peak_allocated)?;
        let mut result = Vec::new();
        result.try_reserve(used_count + free_count)?;
        let mut visits = 0;

        for bucket in &self.buckets {
            append_chain(
                bucket.first_uncommitted.address().to_umem(),
                &mut result,
                &mut visits,
                |address| {
                    let node: UtlTsHashFixedData<D> = read_node(mem, address)?;
                    Ok((node.next.address().to_umem(), node.data))
                },
            )?;
        }
        append_chain(
            self.entry_mem.free_blocks.head.next.address().to_umem(),
            &mut result,
            &mut visits,
            |address| {
                let blob: UtlTsHashAllocatedBlob<D> = read_node(mem, address)?;
                Ok((blob.next.address().to_umem(), blob.data))
            },
        )?;

        // Remove duplicate pointers that exist in both lists.
        let mut seen = HashSet::with_capacity(result.len());
        result.retain(|ptr| seen.insert(ptr.address().to_umem()));
        Ok(result)
    }
}

fn read_node<T: Pod>(mem: &mut impl MemoryView, address: umem) -> Result<T> {
    let mut node = memflow::dataview::zeroed();
    // memflow's read/read_ptr helpers map partial reads to zero-filled values.
    // read_into preserves completion status, so missing bytes cannot end a
    // chain successfully or manufacture a null payload.
    mem.read_into(address.into(), &mut node).data()?;
    Ok(node)
}

fn checked_count(count: i32) -> Result<usize> {
    ensure!(
        (0..=MAX_HASH_ELEMENTS as i32).contains(&count),
        "hash element count is negative or exceeds the discovery cap"
    );
    Ok(count as usize)
}

fn append_chain<D>(
    mut address: umem,
    elements: &mut Vec<Pointer64<D>>,
    visits: &mut usize,
    mut read: impl FnMut(umem) -> Result<(umem, Pointer64<D>)>,
) -> Result<()> {
    let mut seen = HashSet::new();
    while address != 0 {
        ensure!(seen.insert(address), "cyclic hash node chain");
        ensure!(
            *visits < MAX_HASH_ELEMENTS,
            "hash node visits exceed the discovery cap"
        );
        *visits += 1;
        let (next, data) = read(address)?;
        if !data.is_null() {
            elements.push(data);
        }
        address = next;
    }
    Ok(())
}

// SAFETY: The header uses raw integers, explicit padding, a Pod memory pool,
// and Pod buckets; no remote byte is constructed as a Rust bool or enum.
unsafe impl<D: 'static, const C: usize> Pod for UtlTsHash<D, C> {}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use memflow::connector::FileIoMemory;

    use super::*;

    type Hash = UtlTsHash<u64, 2>;

    fn memory(bytes: Vec<u8>) -> impl MemoryView {
        let size = bytes.len() as umem;
        FileIoMemory::with_size(Cursor::new(bytes), size)
            .unwrap()
            .into_mem_view()
    }

    fn node(bytes: &mut [u8], address: usize, next: u64, data: u64) {
        bytes[address + 8..address + 16].copy_from_slice(&next.to_le_bytes());
        bytes[address + 16..address + 24].copy_from_slice(&data.to_le_bytes());
    }

    #[test]
    fn cycles_with_null_data_fail_in_both_chains() {
        let mut hash: Hash = memflow::dataview::zeroed();
        hash.entry_mem.blocks_allocated = 1;
        hash.entry_mem.peak_allocated = 1;
        hash.buckets[0].first_uncommitted = Pointer64::from(0x40);
        let mut bytes = vec![0; 0x100];
        node(&mut bytes, 0x40, 0x40, 0);
        assert!(hash.elements(&mut memory(bytes)).is_err());

        hash.buckets[0].first_uncommitted = Pointer64::from(0);
        hash.entry_mem.free_blocks.head.next = Pointer64::from(0x40);
        let mut bytes = vec![0; 0x100];
        bytes[0x40..0x48].copy_from_slice(&0x40_u64.to_le_bytes());
        assert!(hash.elements(&mut memory(bytes)).is_err());
    }

    #[test]
    fn signed_counts_and_raw_header_discriminants_fail_before_reads() {
        let mut hash: Hash = memflow::dataview::zeroed();
        for invalid in [-1, MAX_HASH_ELEMENTS as i32 + 1, i32::MAX] {
            hash.entry_mem.blocks_allocated = invalid;
            assert!(hash.elements(&mut memory(vec![0; 1])).is_err());
            hash.entry_mem.blocks_allocated = 0;
            hash.entry_mem.peak_allocated = invalid;
            assert!(hash.elements(&mut memory(vec![0; 1])).is_err());
            hash.entry_mem.peak_allocated = 0;
        }
        for invalid in [2, u8::MAX] {
            hash.needs_commit = invalid;
            assert!(hash.elements(&mut memory(vec![0; 1])).is_err());
        }
        hash.needs_commit = 0;
        for invalid in [3, u32::MAX] {
            hash.entry_mem.grow_mode = invalid;
            assert!(hash.elements(&mut memory(vec![0; 1])).is_err());
        }
        assert_eq!(std::mem::size_of::<UtlMemoryPool>(), 0x60);
        assert_eq!(std::mem::offset_of!(UtlMemoryPool, free_blocks), 0x20);
        assert_eq!(std::mem::offset_of!(UtlTsHash<u64>, needs_commit), 0x1860);
        assert_eq!(std::mem::size_of::<UtlTsHash<u64>>(), 0x1870);
        assert_eq!(std::mem::size_of::<UtlTsHashFixedData<u64>>(), 0x18);
        assert_eq!(std::mem::offset_of!(UtlTsHashFixedData<u64>, next), 0x8);
        assert_eq!(std::mem::size_of::<UtlTsHashAllocatedBlob<u64>>(), 0x30);
        assert_eq!(
            std::mem::offset_of!(UtlTsHashAllocatedBlob<u64>, data),
            0x10
        );
    }

    #[test]
    fn shared_tails_and_repeated_payloads_preserve_first_seen_order() {
        let mut hash: Hash = memflow::dataview::zeroed();
        hash.entry_mem.blocks_allocated = 3;
        hash.entry_mem.peak_allocated = 2;
        hash.buckets[0].first_uncommitted = Pointer64::from(0x40);
        hash.buckets[1].first_uncommitted = Pointer64::from(0x60);
        hash.entry_mem.free_blocks.head.next = Pointer64::from(0x80);
        let mut bytes = vec![0; 0x100];
        node(&mut bytes, 0x40, 0x60, 0x900);
        node(&mut bytes, 0x60, 0, 0x800);
        bytes[0x80..0x88].copy_from_slice(&0xb0_u64.to_le_bytes());
        bytes[0x90..0x98].copy_from_slice(&0x900_u64.to_le_bytes());
        bytes[0xc0..0xc8].copy_from_slice(&0x700_u64.to_le_bytes());
        let result = hash.elements(&mut memory(bytes)).unwrap();
        assert_eq!(
            result
                .iter()
                .map(|ptr| ptr.address().to_umem())
                .collect::<Vec<_>>(),
            [0x900, 0x800, 0x700]
        );
    }

    #[test]
    fn unreadable_chains_and_excess_null_node_visits_fail_closed() {
        let mut hash: Hash = memflow::dataview::zeroed();
        hash.buckets[0].first_uncommitted = Pointer64::from(0x1000);
        assert!(hash.elements(&mut memory(vec![0; 1])).is_err());
        hash.buckets[0].first_uncommitted = Pointer64::from(0x40);
        assert!(hash.elements(&mut memory(vec![0; 0x50])).is_err());

        let mut result = Vec::<Pointer64<u64>>::new();
        let mut visits = 0;
        assert!(
            append_chain(1, &mut result, &mut visits, |address| {
                Ok((address + 1, Pointer64::from(0)))
            })
            .is_err()
        );
        assert_eq!(visits, MAX_HASH_ELEMENTS);
        assert!(result.is_empty());
    }
}
