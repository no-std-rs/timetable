//! Main-task RTC samples. Match the pinned HAL's counter latch and calibration.
use esp_hal::peripherals::LPWR;

pub fn stamp() -> (u64, u32) {
    let regs = LPWR::regs();
    regs.time_update().write(|w| w.time_update().set_bit());
    let ticks = (u64::from(regs.time_high0().read().timer_value0_high().bits()) << 32)
        | u64::from(regs.time_low0().read().timer_value0_low().bits());
    (ticks, regs.store1().read().bits())
}
