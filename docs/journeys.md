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

With at least one starting place and one target, **Routes between your stops**
lists every route the timetable offers in the next six hours, including routes
that need a transfer. Each alternative is a dot diagram of every stop on every
ride, with walks, line labels, change minutes and request stops (a dashed ring:
signal the driver). The first search downloads the network once in the browser;
later edits refresh the routes automatically. The search also starts from
places within the extra-walk limit (default ten minutes) of a chosen start and
ends at places within that limit of a chosen target. This finds a further stop
with a direct bus, such as walking about 13 minutes to Kresowa for the 348
instead of taking the 911 from Maków Polnych and changing. Alternatives that use
such an estimated place say so and offer **Add these stops**. Tap or
keyboard-activate any stop dot to add it: it joins the starting stops when it is
nearer a chosen start, otherwise the targets. Tap one of your stops to remove it.
Editable estimates add straight-line walking at 75 metres/minute to the nearest
chosen stop's walk; adjust them for real streets and crossings. Without feed
coordinates, nearby places are not suggested and manual stop search remains.
This helper supplies access and egress walks, never new transfer links.

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
The installer supplies settings, never precomputed routes. The website route
search runs locally in a worker and reports completed depths progressively. It
checks one transfer by default and up to four on request. Its depth does not
change the installed device setting. It lists distinct routes within the
six-hour window and selected transfer limit, ordered by arrival including the
final walk. Each route shows its earliest arrival with the latest departure that
still achieves it, each transfer's platform and waiting time, and every ordered
stop along each ride, including calls that do not allow boarding or alighting.
Different lines, interchange choices, and intermediate stop patterns remain
separate alternatives; repeated services of an identical route share one result.
Any target counts as arrival, so a ride or change continuing past an earlier
target is not listed. Boarding the same vehicles at another nearby place, riding
away and back, and routes with an extra change but no earlier arrival or later
departure are left out. Pickup/drop-off types 2 and 3 are request stops: they
are used and marked, not rejected. Dense networks keep the earliest few prefixes
of the earliest line sequences at each stop, so very slow alternatives can be
omitted. The first eight alternatives are shown, with the rest on request.
Intermediate calls with missing times remain in the stop list and distinguish
route alternatives; missing times are labelled rather than interpolated.
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
active calls, untimed calls, retained prefixes per search round, boardings per trip, and route
alternatives at 300,000 each. Capacity exhaustion never returns a
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
