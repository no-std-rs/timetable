# Complete connections on the display

Choose **Journey planner (advanced)** as a group's mode in the USB installer.
Add one to four starting stops, with walking minutes to each, and one to four
acceptable target stops. An optional walk after each target counts towards ETA.
The display recommends the catchable connection with the earliest arrival,
including access walking, waiting, all bus/train legs and destination walking.
It can therefore prefer a 12-minute walk to a direct bus over a nearer stop.

The normal setting allows **one transfer**. The minimum change defaults to
**three minutes**: exactly three is allowed; less is rejected. The GTFS minimum
can increase this margin, and prohibited transfers remain prohibited. Choose
zero transfers for direct journeys, or two in the advanced device settings.
For equal arrival times, fewer rides and less access walking win. These are
scheduled ETAs; live delays and the likelihood of catching a vehicle are not
estimated.

Both Android and ESP32 calculate independently from downloaded static GTFS.
The installer supplies settings, never precomputed routes. The optional website
explorer runs locally in a worker and reports completed depths progressively,
up to four transfers. Its depth does not change the installed device setting.
There is no routing server. The ordinary departure-board group mode remains
available.

The device builds a six-hour departure/arrival profile. As time advances, the
screen chooses the next complete connection whose first bus can still be reached
with the configured walk, without another download. It shows arrival ETA, when
to leave, walking time, each line and its boarding/alighting stops, and actual
change time. A long press in this mode opens group selection; edit journey
endpoints and margins in the installer.

Transfers use the same stop, platforms with a declared common `parent_station`,
or explicit `transfers.txt` links. No street route or nearby-stop footpath is
invented. A station transfer's configured slack must cover its walking time.
Journey stop IDs must still exist in the downloaded feed; select them again if
the operator renumbers stops. This mode does not plan walking-only journeys,
fares, accessibility constraints, frequency templates, route/trip-specific
transfer rules or in-seat transfer rules. Unsupported relevant rules fail the
search explicitly. On ESP32, a window requiring different GTFS archives is
rejected instead of claiming a complete search across incompatible snapshots.

The planner uses fixed storage and fails visibly when a selection exceeds it:

| Bound | ESP32 | Android |
| --- | ---: | ---: |
| Relevant endpoint trips | 256 | 1,024 |
| Exact trip-ID bytes | 4,096 | 16,384 |
| Endpoint trip calls in the time window | 1,024 | 2,048 |
| Stops including interchange platforms | 128 | 384 |
| Advanced middle-ride calls | 128 | 2,048 |
| Active services | 16 | 64 |
| Relevant routes | 32 | 64 |

Both profiles retain at most 64 nondominated future connections across groups,
with 32 shared labels in a 1 KiB arena. Capacity exhaustion never returns a
silently shortened complete result. Yesterday, today and tomorrow's service
days include calendar exceptions and after-24:00 times, using the installed
network timezone. Feeds extending beyond that service-day policy are unsupported.

Android keeps the original ZIP and validated journey profile on disk. A failed
or cancelled update preserves the last validated snapshot. ESP32 keeps journey
profiles in RAM and bypasses the ordinary departure-only flash cache when any
journey group is configured. It downloads and recalculates after a restart or
C3 deep-sleep wake, so Wi-Fi is required then. It does not repeatedly write a
moving six-hour profile to flash. Ordinary departure-only configurations retain
the existing persistent-cache behaviour.

Synthetic regression schedules use the requested Maków Polnych /
Bolechowo-Os./Szkoła names; their times are test data, not actual transit advice.
