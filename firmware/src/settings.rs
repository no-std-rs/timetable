//! Read newest valid settings directly into provisional caller-owned storage.
use timetable_display::{Config, config};

#[inline(never)]
pub fn load_banks<E>(
    mut read: impl FnMut(u32, &mut [u8]) -> Result<(), E>,
    scratch: &mut [u8; config::IMAGE_SIZE],
    output: &mut Config,
) -> Option<(u32, u32)> {
    let mut banks = [0, config::BANK_SIZE as u32];
    let generations = banks.map(|bank| {
        let mut bytes = [0; 4];
        if read(config::FLASH_OFFSET + bank + 8, &mut bytes).is_ok() {
            Some(u32::from_le_bytes(bytes))
        } else if read(config::FLASH_OFFSET + bank, scratch).is_ok() {
            Some(u32::from_le_bytes(scratch[8..12].try_into().unwrap()))
        } else {
            None
        }
    });
    if generations[1]
        .is_some_and(|second| generations[0].is_none_or(|first| config::newer(second, first)))
    {
        banks.swap(0, 1);
    }
    // Header generations only order the attempts. Full CRC, schema and settings
    // validation still decide which bank is published; a broken newer bank
    // falls back to the other one without retaining a second Config on stack.
    for bank in banks {
        if read(config::FLASH_OFFSET + bank, scratch).is_ok()
            && let Ok(generation) = config::decode_into(scratch, output)
        {
            return Some((generation, bank));
        }
    }
    None
}
