//! Main-task RTC samples. Match the pinned HAL's counter latch and calibration.
#[cfg(not(feature = "board-waveshare-c6"))]
use esp_hal::peripherals::LPWR;

#[cfg(not(feature = "board-waveshare-c6"))]
pub fn stamp() -> (u64, u32) {
    let regs = LPWR::regs();
    regs.time_update().write(|w| w.time_update().set_bit());
    let ticks = (u64::from(regs.time_high0().read().timer_value0_high().bits()) << 32)
        | u64::from(regs.time_low0().read().timer_value0_low().bits());
    (ticks, regs.store1().read().bits())
}

#[cfg(feature = "board-waveshare-c6")]
pub fn stamp() -> (u64, u32) {
    use esp_hal::peripherals::{LP_AON, LP_TIMER};
    let regs = LP_TIMER::regs();
    regs.update().write(|w| w.main_timer_update().set_bit());
    let ticks = (u64::from(regs.main_buf0_high().read().main_timer_buf0_high().bits()) << 32)
        | u64::from(regs.main_buf0_low().read().main_timer_buf0_low().bits());
    (ticks, LP_AON::regs().store1().read().bits())
}
