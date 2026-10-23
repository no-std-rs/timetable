//! Opt-in camera motion, not a claim of human detection. Luma samples never leave
//! RAM. One small grid and a debounce reject isolated sensor noise.
pub const BYTES: usize = 800;
pub fn detect(
    state: &mut [u8],
    luma: &[u8],
    width: usize,
    height: usize,
) -> Result<bool, &'static str> {
    if state.len() < BYTES
        || width < 32
        || height < 24
        || width
            .checked_mul(height)
            .is_none_or(|n| n > luma.len() || n > 4096 * 4096)
    {
        return Err("Invalid camera frame");
    }
    let previous = state[768] == 1;
    let mut changed = 0;
    for y in 0..24 {
        for x in 0..32 {
            let index = y * 32 + x;
            let value = luma[(y * height / 24) * width + x * width / 32];
            changed += usize::from(value.abs_diff(state[index]) >= 18);
            state[index] = value;
        }
    }
    state[768] = 1;
    let moving = previous && changed >= 48;
    state[769] = if moving {
        state[769].saturating_add(1)
    } else {
        0
    };
    Ok(state[769] >= 2)
}
