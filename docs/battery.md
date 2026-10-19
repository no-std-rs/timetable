# Battery operation

Enable **Battery mode** in the USB installer. It is off by default, so existing
installations keep their always-visible board. Both modes use the same
selective mirror queries, official fallback, selection limits and validated
six-hour departure cache. Always-on mode also releases Wi-Fi between three-hour
updates and runs the CPU at 80 MHz, while keeping the backlight, screen and
touch active. Every network session corrects UTC with NTP; an available network
therefore permits correction every three hours, including after sleep.

After 30 seconds of inactivity on the board, the app switches off the backlight,
sends the LCD display-off and sleep-in commands, stops Wi-Fi and enters CPU
deep sleep. Editors, unsaved drafts, failed settings writes, an active touch and
an unfinished query keep the CPU awake. During a query the idle backlight can
still turn off. A tap wakes the screen; the wake gesture is consumed through
release so it cannot accidentally change a group or selection.

A fresh cached wake does not initialize the radio. The display restores saved
departures, destination labels, selected group and the two-hour manual override.
It filters departed events using the restored clock. A timer wakes the device
for a Wi-Fi refresh after three hours. Timer refreshes keep the backlight off
unless touched. Retry and line/stop changes can start Wi-Fi on a cached wake;
the refresh deadline is also honored if an editor remains open. Failed refreshes
retain validated data and schedule a five-minute battery sleep retry. An initial
connection attempt is bounded to 30 seconds and clock synchronization to 15.

## Retention and time

A 4,096-byte RTC array contains encoded data, with at most 3,184 bytes used at
the full 512-event/32-heading/1,024-byte heading-pool limits. It contains no
Wi-Fi credentials. Magic/version, CRC, settings identity, lengths, selections,
headsign references, UTF-8, time ranges and deadlines are checked before staging
and publishing restored data. Changing installed settings invalidates an older
snapshot. Cold/invalid retention falls back to normal Wi-Fi bootstrap.

The normal uptime counter stops during sleep. Instead, the app retains UTC in
microseconds, raw RTC ticks and their calibration period. Elapsed time is
calculated from the raw tick difference using the saved period; subtracting two
absolute RTC microsecond readings across boots would be incorrect because HAL
recalibrates on boot. Subsecond time survives repeated short sleeps. The RTC
oscillator can drift between SNTP synchronizations.

The minimum-reserve regression account is **298,612 bytes**, including RTC,
the unchanged 64 KiB heap and 24 KiB minimum stack reserve. It remains below
300,000 bytes but is not a runtime peak. Static RAM plus the complete
50,568-byte available stack is **324,604 bytes**, below 400,000. TLS uses more
than the minimum reserve; a separate compiled-stack audit checks known paths
with headroom. See [the linked report](../benchmarks/device-throughput-ram.json),
[boot-stack audit](boot-recovery.md) and [performance notes](../PERFORMANCE.md).

## Board support and verification

The [manufacturer schematic](https://europe1.discourse-cdn.com/arduino/original/4X/6/b/6/6b630e7d70dcf2de063691c62978c80fc7ee7397.png)
for ESP32-2424S012-V1.0 identifies **GPIO0 as TP_INT**, GPIO1 as touch reset,
GPIO10 as LCD chip select and GPIO3 as backlight. Earlier inherited notes that
TP_INT shared GPIO10 were incorrect. GPIO0 supplies the deep-sleep wake input;
touch reset is held high and the backlight is held low by its external pulldown.

The touch controller stays in automatic low-power sensing with touch interrupts
enabled. It does not enter reset-only standby. Before sleeping the firmware
verifies the touch registers and an inactive wake line. Preparation failure
keeps the CPU awake and shows **SLEEP FAILED**. Shutdown prevents new queries
and waits for radio-owner acknowledgement before CPU sleep. A delayed radio
shutdown shows the same message and remains pending until acknowledged.

No physical current, wake reliability or clock-drift measurement has been made.
Battery life cannot be inferred from the ESP32 chip's sleep-current rating:
the LCD, touch controller and board power circuit remain part of the load.
Verify a complete board with a current meter: cold bootstrap; cached tap wakes
without Wi-Fi; retained group/clock across repeated sleeps; a timer refresh with
the backlight dark; unavailable Wi-Fi and its retry; and USB recovery. Compare
the restored clock with SNTP after a full interval and at operating temperatures.

Sources: [ESP32-C3 sleep modes](https://docs.espressif.com/projects/esp-idf/en/stable/esp32c3/api-reference/system/sleep_modes.html),
[CST816 controller datasheet](https://cdn.static.spotpear.com/uploads/picture/learn/common-lcd/lcd/1.28inch-round-lcd/CST816S%20Data%20Sheet%20V1.4.pdf),
[GC9A01 datasheet](https://cdn-learn.adafruit.com/assets/assets/000/135/078/original/GC9A01A.pdf).

## Installer compatibility

Old settings omit `battery` and decode as `false`. New settings support is
advertised as `config_version: 2`; the installer refuses older firmware metadata
so a stale cached binary cannot receive an unsupported settings field. The
flash-bank header and group export format remain readable. Quotes, backslashes
and JSON Unicode escapes are decoded using a bounded 64-byte unescape buffer,
including Wi-Fi credentials and full-length stop labels.
