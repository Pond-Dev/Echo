use memflow::prelude::v1::*;

use super::{SchemaClassBinding, SchemaEnumBinding, SchemaSystemTypeScope};

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct SchemaArrayT {
    pub array_size: u32,                // 0x0000
    pad_0: [u8; 0x4],                   // 0x0004
    pub element: Pointer64<SchemaType>, // 0x0008
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct SchemaAtomicI {
    pad_0: [u8; 0x10], // 0x0000
    pub value: u64,    // 0x0010
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct SchemaAtomicT {
    pub element: Pointer64<SchemaType>,  // 0x0000
    pad_0: [u8; 0x8],                    // 0x0008
    pub template: Pointer64<SchemaType>, // 0x0010
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct SchemaAtomicTT {
    pad_0: [u8; 0x10],                         // 0x0000
    pub templates: [Pointer64<SchemaType>; 2], // 0x0010
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct SchemaAtomicTF {
    pad_0: [u8; 0x10],                   // 0x0000
    pub template: Pointer64<SchemaType>, // 0x0010
    pub size: i32,                       // 0x0018
    pad_1: [u8; 0x4],                    // 0x001C
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct SchemaAtomicTTF {
    pad_0: [u8; 0x10],                         // 0x0000
    pub templates: [Pointer64<SchemaType>; 2], // 0x0010
    pub size: i32,                             // 0x0020
    pad_1: [u8; 0x4],                          // 0x0024
}

#[derive(Pod)]
#[repr(C)]
pub struct SchemaType {
    pad_0: [u8; 0x8],                                 // 0x0000
    pub name: Pointer64<ReprCString>,                 // 0x0008
    pub type_scope: Pointer64<SchemaSystemTypeScope>, // 0x0010
    pub type_category: u8,                            // 0x0018: checked before use
    pub atomic_category: u8,                          // 0x0019: checked before use
    pad_1: [u8; 0x6],                                 // 0x001A
    pub value: SchemaTypeUnion,                       // 0x0020
}

impl SchemaType {
    pub fn validate_categories(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.type_category <= 7, "invalid schema type category");
        anyhow::ensure!(self.atomic_category <= 7, "invalid schema atomic category");
        Ok(())
    }
}

#[repr(C)]
pub union SchemaTypeUnion {
    pub r#type: Pointer64<SchemaType>,
    pub class_binding: Pointer64<SchemaClassBinding>,
    pub enum_binding: Pointer64<SchemaEnumBinding>,
    pub array: SchemaArrayT,
    pub atomic: SchemaAtomicT,
    pub atomic_tt: SchemaAtomicTT,
    pub atomic_tf: SchemaAtomicTF,
    pub atomic_ttf: SchemaAtomicTTF,
    pub atomic_i: SchemaAtomicI,
}

// SAFETY: Every union member consists solely of raw integers, explicit padding,
// and integer-backed remote pointers, so every byte pattern is valid.
unsafe impl Pod for SchemaTypeUnion {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_schema_categories_are_validated_without_constructing_enums() {
        let mut value: SchemaType = memflow::dataview::zeroed();
        for category in 0..=7 {
            value.type_category = category;
            value.atomic_category = category;
            assert!(value.validate_categories().is_ok());
        }
        for invalid in [8, u8::MAX] {
            value.type_category = invalid;
            assert!(value.validate_categories().is_err());
            value.type_category = 0;
            value.atomic_category = invalid;
            assert!(value.validate_categories().is_err());
            value.atomic_category = 0;
        }
        assert_eq!(std::mem::offset_of!(SchemaType, value), 0x20);
    }
}
