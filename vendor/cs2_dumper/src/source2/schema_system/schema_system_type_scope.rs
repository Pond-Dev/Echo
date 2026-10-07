use std::ffi::CStr;

use memflow::prelude::v1::*;

use super::{SchemaClassBinding, SchemaEnumBinding};

use crate::source2::UtlTsHash;

#[derive(Pod)]
#[repr(C)]
pub struct SchemaSystemTypeScope {
    pad_0: [u8; 0x8],                                   // 0x0000
    pub name: [u8; 256],                                // 0x0008
    pub global_scope: Pointer64<SchemaSystemTypeScope>, // 0x0108
    pad_1: [u8; 0x450],                                 // 0x0110
    pub class_bindings: UtlTsHash<SchemaClassBinding>,  // 0x0560
    pub enum_bindings: UtlTsHash<SchemaEnumBinding>,    // 0x1DD0
}

impl SchemaSystemTypeScope {
    pub fn module_name(&self) -> anyhow::Result<&CStr> {
        CStr::from_bytes_until_nul(&self.name)
            .map_err(|_| anyhow::anyhow!("type scope module name is not NUL-terminated"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_is_bounded_to_the_wire_name_buffer() {
        let mut scope: SchemaSystemTypeScope = memflow::dataview::zeroed();
        scope.name.fill(u8::MAX);
        assert!(scope.module_name().is_err());
        scope.name[..11].copy_from_slice(b"client.dll\0");
        assert_eq!(scope.module_name().unwrap().to_bytes(), b"client.dll");
        assert_eq!(
            std::mem::offset_of!(SchemaSystemTypeScope, class_bindings),
            0x560
        );
        assert_eq!(
            std::mem::offset_of!(SchemaSystemTypeScope, enum_bindings),
            0x1dd0
        );
    }
}
