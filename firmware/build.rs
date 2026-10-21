use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-changed=../catalog/catalog.bin");
    if env::var_os("CARGO_FEATURE_FIRMWARE").is_none() {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let catalog = fs::read("../catalog/catalog.bin").unwrap();
    let mut payload = (catalog.len() as u32).to_le_bytes().to_vec();
    payload.extend_from_slice(&catalog);
    let mut slot = b"TTBND001".to_vec();
    slot.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    slot.extend_from_slice(&crc32(&payload).to_le_bytes());
    slot.extend_from_slice(&payload);
    assert!(slot.len() <= 1024 * 1024);
    slot.resize(1024 * 1024, 0xff);
    let file = out.join("catalog-slot.bin");
    fs::write(&file, slot).unwrap();
    // Assembler-defined memory prevents Rust from folding template contents.
    fs::write(out.join("catalog-slot.S"), format!(
        ".section .rodata.timetable_catalog,\"a\",@progbits\n.balign 4\n.global timetable_catalog_slot\n.type timetable_catalog_slot,@object\ntimetable_catalog_slot:\n.incbin \"{}\"\n.size timetable_catalog_slot, .-timetable_catalog_slot\n", file.display())).unwrap();
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
