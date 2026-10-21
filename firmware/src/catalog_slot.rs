//! Compiler-opaque, installer-customizable immutable ROM. No RAM copy.
use timetable_display::{
    catalog::Catalog,
    profile::{self, Profile},
};
core::arch::global_asm!(include_str!(concat!(env!("OUT_DIR"), "/catalog-slot.S")));
unsafe extern "C" {
    static timetable_catalog_slot: [u8; profile::SLOT_BYTES];
}
fn bytes() -> &'static [u8] {
    // SAFETY: the assembler symbol defines exactly SLOT_BYTES immutable ROM
    // bytes. The installer changes them before execution, never while borrowed.
    unsafe { &timetable_catalog_slot }
}
pub fn catalog() -> Option<Catalog<'static>> {
    Catalog::from_bundle(bytes())
}
pub fn profile() -> Profile<'static> {
    // Boot validates the full immutable envelope before settings/cache access.
    profile::metadata(bytes()).expect("validated catalogue profile")
}
pub fn tag(tag: u32) -> u32 {
    tag ^ timetable_display::config::crc32(profile().identity())
}
