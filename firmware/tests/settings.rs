use timetable_display::{Config, config};
use timetable_firmware::settings::load_banks;

fn image(c: &Config, generation: u32) -> [u8; config::IMAGE_SIZE] {
    let mut image = [0; config::IMAGE_SIZE];
    config::encode(c, generation, &mut image).unwrap();
    image
}

#[test]
fn newest_valid_bank_wins_and_corrupt_newer_bank_falls_back_without_stale_fields() {
    let mut source = *include_bytes!("../../display/tests/settings.bin");
    let old = config::decode(&mut source).unwrap().0;
    let mut newer = old.clone();
    newer.battery = true;
    newer.wifi_ssid = "new-network".try_into().unwrap();
    let first = image(&old, u32::MAX);
    let mut second = image(&newer, 0);
    let mut scratch = [0; config::IMAGE_SIZE];
    let mut output = Config::empty();
    let load = |second: &[u8; config::IMAGE_SIZE], output: &mut Config, scratch: &mut _| {
        load_banks(
            |offset, bytes| {
                let offset = (offset - config::FLASH_OFFSET) as usize;
                let image = if offset >= config::BANK_SIZE {
                    second
                } else {
                    &first
                };
                let start = offset % config::BANK_SIZE;
                bytes.copy_from_slice(&image[start..start + bytes.len()]);
                Ok::<_, ()>(())
            },
            scratch,
            output,
        )
    };
    assert_eq!(
        load(&second, &mut output, &mut scratch),
        Some((0, config::BANK_SIZE as u32))
    );
    assert!(output == newer);
    // Preserve a correct CRC while introducing a late schema error, after
    // battery and Wi-Fi were parsed into provisional storage.
    let len = u32::from_le_bytes(second[12..16].try_into().unwrap()) as usize;
    let payload = &mut second[config::HEADER_SIZE..config::HEADER_SIZE + len];
    let start = payload.windows(8).position(|w| w == b"\"groups\"").unwrap();
    payload[start + 1] = b'X';
    second[16..20].fill(0);
    let crc = config::crc32(&second[..config::HEADER_SIZE + len]);
    second[16..20].copy_from_slice(&crc.to_le_bytes());
    assert_eq!(
        load(&second, &mut output, &mut scratch),
        Some((u32::MAX, 0))
    );
    assert!(output == old);
    assert!(!output.battery);
}

#[test]
fn interrupted_bank_and_header_reads_never_publish_an_invalid_config() {
    let mut scratch = [0; config::IMAGE_SIZE];
    let mut output = Config::empty();
    assert_eq!(load_banks(|_, _| Err(()), &mut scratch, &mut output), None);
    assert_eq!(
        load_banks(
            |_, bytes| {
                bytes.fill(0xff);
                Ok::<_, ()>(())
            },
            &mut scratch,
            &mut output
        ),
        None
    );
    let mut original = *include_bytes!("../../display/tests/settings.bin");
    let mut newest = config::decode(&mut original).unwrap().0;
    let first = image(&newest, 4);
    newest.wifi_ssid = "newest-network".try_into().unwrap();
    let second = image(&newest, 5);
    let selected = load_banks(
        |offset, bytes| {
            if bytes.len() == 4 {
                return Err(());
            }
            let bank = if offset == config::FLASH_OFFSET {
                &first
            } else {
                &second
            };
            bytes.copy_from_slice(bank);
            Ok(())
        },
        &mut scratch,
        &mut output,
    );
    assert_eq!(selected, Some((5, config::BANK_SIZE as u32)));
    assert!(output == newest);
}
