use memflow::prelude::v1::{Pod, Pointer64};

#[repr(C)]
pub struct TsListNode;

#[derive(Pod)]
#[repr(C)]
pub struct TsListHead {
    pub next: Pointer64<TsListNode>, // 0x0000
}

#[derive(Pod)]
#[repr(C)]
pub struct TsListBase {
    pub head: TsListHead, // 0x0000
}
