/* Same region aliases and section scripts as esp-hal 1.1.1's C6 linkall.x.
 * Limit HP RAM to 384 KiB so the complete linked app, including its stack
 * and retained state, remains inside this project's 400,000 byte budget. */
MEMORY {
    RAM : ORIGIN = 0x40800000, LENGTH = 0x60000
    dram2_seg (RW) : ORIGIN = 0x4086e610, LENGTH = 0
    ROM : ORIGIN = 0x42000020, LENGTH = 0x400000 - 0x20
    RTC_FAST : ORIGIN = 0x50000000, LENGTH = 16K
}
REGION_ALIAS("ROTEXT", ROM);
REGION_ALIAS("RODATA", ROM);
REGION_ALIAS("RWTEXT", RAM);
REGION_ALIAS("RWDATA", RAM);
REGION_ALIAS("RTC_FAST_RWTEXT", RTC_FAST);
REGION_ALIAS("RTC_FAST_RWDATA", RTC_FAST);
INCLUDE "esp32c6.x"
INCLUDE "hal-defaults.x"
