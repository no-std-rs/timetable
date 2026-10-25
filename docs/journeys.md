# Complete connections on the display

Choose **Journey planner (advanced)** as a group's mode in the USB installer.
Add one to four starting stops, with walking minutes to each, and one to four
acceptable target stops. Platforms in the same declared station are one entity.
Without a station declaration, equal full names group within about 125 metres
of a representative platform; case and whitespace are ignored, accents retained.
Stops are partitioned by exact stop-ID order with one representative per place,
so overlapping neighborhoods use the same membership in the installer, browser
search and both devices. Unrelated same-name stops stay separate. Without coordinates or a declared
station, the exact platform ID remains the endpoint. Choose the place once,
set one walk, and the planner considers its platforms at both ends.
Searching a platform code finds its whole place. The itinerary identifies the
actual boarding/alighting platform. Existing saved platform IDs expand to their
place automatically; duplicate place selections in older installer drafts merge
and keep the shortest configured walk. An optional walk after each target counts
towards ETA.
The display recommends the catchable connection with the earliest arrival,
including access walking, waiting, all bus/train legs and destination walking.
It can therefore prefer a 12-minute walk to a direct bus over a nearer stop.

After selecting two starting places, **Stops between your starting stops**
shows their positions and intermediate stops as a local dot diagram, with matching
quick-add buttons below. Tap or keyboard-activate an intermediate dot to add or
remove that starting place. The diagram uses feed coordinates without map tiles
or location services. Places within a 150-metre corridor between the first two
starts are shown, including selected intermediate starts. The extra-walk limit
defaults to ten minutes. Editable estimates add straight-line walking at 75
metres/minute to a chosen start's configured walk. Adjust for real streets and
crossings. Missing feed coordinates leave manual stop search available. This
installer helper supplies access walks, never new transfer links.

The normal setting allows **one transfer**. The minimum change defaults to
**three minutes**: exactly three is allowed; less is rejected. The GTFS minimum
can increase this margin, and prohibited transfers remain prohibited. The chosen
minimum also applies to GTFS timed-transfer guarantees; the app does not waive
your margin or assume a later departure than the published schedule. Choose
zero transfers for direct journeys, or two in the advanced device settings.
For equal arrival times, fewer rides and less access walking win. These are
scheduled ETAs; live delays and the likelihood of catching a vehicle are not
estimated.

Both Android and ESP32 calculate independently from downloaded static GTFS.
The installer supplies settings, never precomputed routes. The optional website
explorer runs locally in a worker and reports completed depths progressively,
up to four transfers. Its depth does not change the installed device setting.
The website explorer can also apply route/trip-scoped stop-transfer rules and
keeps different arriving services when those rules distinguish them. Installed
devices retain the stricter rule limits described below. There is no routing
server. The ordinary departure-board group mode remains
available.

The device builds a six-hour departure/arrival profile. As time advances, the
screen chooses the next complete connection whose first bus can still be reached
with the configured walk, without another download. It shows arrival ETA, when
to leave, walking time, each line and its boarding/alighting stops, and actual
change time. A long press in this mode opens group selection; edit journey
endpoints and margins in the installer.

Transfers use the same named place (including its platforms), platforms with a
declared common `parent_station`, or explicit `transfers.txt` links. Explicit
platform/station prohibitions and minimums override a same-name connection. No street route or nearby-stop footpath is
invented. A station transfer's configured slack must cover its walking time.
Journey stop IDs must still exist in the downloaded feed; select them again if
the operator renumbers stops. This mode does not plan walking-only journeys,
fares, accessibility constraints, frequency templates, conditional boarding or
alighting that requires arrangements, route/trip-specific
transfer rules or in-seat transfer rules. Unsupported relevant rules fail the
search explicitly. The bounded device search conservatively rejects a scoped
rule touching a selected trip’s stops; it does not evaluate route/trip predicates.
Use feeds without such rules for those stops. On ESP32, a window requiring different GTFS archives is
rejected instead of claiming a complete search across incompatible snapshots.

The planner uses fixed storage and fails visibly when a selection exceeds it:

| Bound | ESP32 | Android |
| --- | ---: | ---: |
| Relevant endpoint trips | 256 | 1,024 |
| Trip-ID / temporary place-name arena bytes | 4,096 | 16,384 |
| Endpoint trip calls in the time window | 1,024 | 2,048 |
| Stops including relevant name cohorts and interchange platforms | 128 | 384 |
| Advanced middle-ride calls | 128 | 2,048 |
| Active services | 16 | 64 |
| Relevant routes | 32 | 64 |

Both profiles retain at most 64 nondominated future connections across groups,
with 32 shared labels in a 1 KiB arena. Browser exploration separately bounds
active calls and retained prefixes at 300,000 each. Capacity exhaustion never returns a
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
