#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const MAX_FAILURE: usize = 4096;
pub fn packet(command: &[u8; 4], value: u32) -> [u8; 8] {
    let mut packet = [0; 8];
    packet[..4].copy_from_slice(command);
    packet[4..].copy_from_slice(&value.to_le_bytes());
    packet
}
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
pub fn exit_result(bytes: &[u8]) -> Result<String, String> {
    let text = String::from_utf8_lossy(bytes);
    let (body, status) = text
        .rsplit_once("\nTT_EXIT:")
        .ok_or("The device did not return a command status")?;
    let status = status
        .trim()
        .parse::<u32>()
        .map_err(|_| "Invalid device command status")?;
    if status != 0 {
        return Err(format!("Device command failed ({status}): {}", body.trim()));
    }
    Ok(body.trim().to_owned())
}
pub fn sync_result(bytes: &[u8]) -> Result<Option<()>, String> {
    if bytes.len() < 8 {
        return Ok(None);
    }
    let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if size > MAX_FAILURE || bytes.len() > 8 + MAX_FAILURE {
        return Err("Oversized ADB sync response".into());
    }
    if bytes.len() < 8 + size {
        return Ok(None);
    }
    if bytes.len() != 8 + size {
        return Err("Unexpected trailing ADB sync data".into());
    }
    match &bytes[..4] {
        b"OKAY" if size == 0 => Ok(Some(())),
        b"FAIL" => Err(format!(
            "APK transfer failed: {}",
            String::from_utf8_lossy(&bytes[8..])
        )),
        _ => Err("Invalid ADB sync response".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_status_handles_fragmentation_and_bounded_failures() {
        let okay = packet(b"OKAY", 0);
        for n in 0..8 {
            assert_eq!(sync_result(&okay[..n]), Ok(None));
        }
        assert_eq!(sync_result(&okay), Ok(Some(())));
        assert!(sync_result(&packet(b"FAIL", 5000)).is_err());
        let mut fail = packet(b"FAIL", 4).to_vec();
        fail.extend_from_slice(b"full");
        assert!(sync_result(&fail).unwrap_err().contains("full"));
    }
    #[test]
    fn shell_reports_failure_and_quotes_untrusted_values() {
        assert_eq!(exit_result(b"Success\nTT_EXIT:0\n").unwrap(), "Success");
        assert!(exit_result(b"no\nTT_EXIT:1\n").is_err());
        assert!(exit_result(b"Success").is_err());
        assert_eq!(shell_quote("a'b;$(x)"), "'a'\"'\"'b;$(x)'");
    }
}
