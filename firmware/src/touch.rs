//! Controller packets are decoded separately from I2C so host tests exercise
//! the exact orientation and reject off-panel coordinates before dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    Released,
    Pressed { x: u16, y: u16 },
}
pub fn cst816(data: &[u8; 5]) -> Option<Sample> {
    if data[0] & 15 == 0 {
        return Some(Sample::Released);
    }
    let x = (u16::from(data[1] & 15) << 8) | u16::from(data[2]);
    let y = (u16::from(data[3] & 15) << 8) | u16::from(data[4]);
    (x < 240 && y < 240).then_some(Sample::Pressed { x, y })
}
/// Read from register 0x01 at address 0x63, with a STOP between write/read,
/// as in Waveshare's AXS5106L reference. Portrait orientation mirrors X.
pub fn axs5106(data: &[u8; 14]) -> Option<Sample> {
    if data[1] & 15 == 0 {
        return Some(Sample::Released);
    }
    let x = (u16::from(data[2] & 15) << 8) | u16::from(data[3]);
    let y = (u16::from(data[4] & 15) << 8) | u16::from(data[5]);
    if x >= 172 || y >= 320 {
        return None;
    }
    Some(Sample::Pressed { x: 171 - x, y })
}

/// Waveshare's 90-degree orientation swaps the native touch axes without
/// mirrors. Reuse portrait packet validation, then undo its X mirror.
pub fn axs5106_landscape(data: &[u8; 14]) -> Option<Sample> {
    axs5106(data).map(|sample| match sample {
        Sample::Pressed { x, y } => Sample::Pressed { x: y, y: 171 - x },
        Sample::Released => Sample::Released,
    })
}
