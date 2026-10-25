# Timetable USB installer

A static, responsive installer with a porcelain / ink / gold / chilli palette.
Group changes update a WebAssembly instance of the actual Rust app in round or portrait layout. The route/stop
picker and firmware share the selected network’s checked catalogue. Poznań has
an instant bundled catalogue; other networks are built in a browser worker. It supports
four named groups, four choices per group, direction filters, per-choice
walking time and optional weekday/time windows. Export/import contains group
settings only, never Wi-Fi credentials.
“Follow group schedules” controls automatic switching for all groups. Turn it
off for touch-only selection without deleting time windows; imported manual
settings can be inspected and switched back to automatic mode here.

## Journey groups

Choose **Journey planner (advanced)** to configure starting stops, walking times
and targets. The display recommends a complete connection with ETA, allowing
one transfer and at least three minutes to change by default. Advanced device
settings allow direct-only or two transfers. **Routes between your stops**
searches locally in the browser, including nearby walkable stops, and shows each
route as tappable stop dots: tap a stop to add it as a start or target. It checks
one transfer by default and up to four on request. See [journey behaviour and limits](../docs/journeys.md),
including ESP32 restart connectivity and supported transfer rules.

## Countries and transport networks

Open **Transport network**, choose a country and city/region, then search for an
operator or feed name. One feed is installed per display. The bundled public
MobilityDatabase directory contains 4,646 schedule feeds across 94 named
countries/regions plus unspecified locations, dated 2026-10-21. Regional feeds without a city
remain searchable. **Refresh network directory** reads the public CSV directly;
no API account or token is needed. Real-time and bike-sharing feeds are not static
timetables. Unavailable/inactive entries remain visible with a source link.

Loading a network downloads its public mirrored ZIP in a worker, streams CSV and
DEFLATE, and builds sorted route/stop choices. Route diagrams are reconstructed
on demand from that same ZIP, including branches, interleaved journeys and
non-boarding terminals. Public ZIPs are cached in IndexedDB for three hours.
Downloads show progress and switching networks cancels the previous worker.
Header-only optional frequency tables are accepted; actual frequency templates
remain unsupported. The device validates frequency headers within 512 bytes and
48 columns, using the existing decompression workspace.
Groups are remembered separately per feed; Wi-Fi preferences stay in the current
tab. Group exports contain `feed_id` and no credentials. Select that feed before
importing its groups. Forget/opt-out erases every saved network’s credential copy.

Prepare validates the published firmware SHA first, replaces only a reserved
1 MiB immutable catalogue/profile slot, and repairs the ESP image checksum and
appended SHA. Settings still use their original flash partition. The checked
profile stores the feed identity and IANA timezone transitions (installation year
minus two through year plus ten). Reinstall after timezone legislation changes
or before that finite coverage expires. No arbitrary source URL or credential
is embedded in download requests. The real Rust WASM demo uses the same profile.

For large networks, the installed catalogue contains only the chosen routes and
all their boarding stops; the page states this scope. Add other routes here and
reinstall. Full small catalogues remain editable on the display. Exact IDs are
never shortened. Current device limits include 32-byte route/stop/service IDs and 255-byte journey IDs, 8-byte human line labels, 64-byte stop/headsign labels, 8 KiB CSV records,
48 columns, 64 services, 1,024 active trips / 16 KiB trip IDs and 512 future
selected departures. Oversized static identities and unsupported archives fail
before installation; changing feeds or oversized active selections can still
fail visibly at refresh. Frequency templates, GTFS Flex, ZIP64 and encrypted ZIPs
are unsupported. No result is silently truncated to fit the display.

Non-Latin labels use a local browser-generated 10×10 glyph subset in flash, with
no whole font in ESP32 RAM. The preview shows the actual device lettering. It
relies on installed browser fonts; per-character glyphs do not provide complex
script shaping or right-to-left layout. These remain display limitations.

The device discovers immutable snapshots for the installed feed and uses the
existing bounded range-query/cache pipeline. Cache and RTC identities include
the feed and timezone rules, so reused route/stop IDs cannot mix networks. Only
Poznań has a configured direct official-source fallback. Other networks keep the
last validated board if the mirror is unavailable, and never query Poznań.

Directory source: [MobilityDatabase public CSV](https://files.mobilitydatabase.org/feeds_v2.csv)
and [published schema](https://github.com/MobilityData/mobility-feed-api/blob/main/docs/SpreadsheetSchemaV2.md).

Battery mode is off by default. On the GUITION, enable it to sleep the screen and CPU after
30 seconds on the idle departure board, wake by touch and refresh Wi-Fi every
three hours. Cached wakes do not initialize Wi-Fi. Group imports leave this
device preference unchanged. See [battery operation and hardware limits](../docs/battery.md).

Search any stop by name or platform code in one combobox. Platforms with the
same name share one result, with all their serving lines. Choose a line and
destination to resolve the exact boarding platform; the hint shows its code,
next stop and actual destinations. Multiple platforms in one direction remain
separate choices. Polish accents are optional for search, while distinct accented
names remain distinct groups. You can also start with a line number or destination.
Stop lists and destination choices use the feed language’s alphabetical order
(Polish for Poznań) in either page language; line labels sort naturally (2, 10, 100, T2, T10). Filtering preserves
that order. Route diagrams and their stop lists retain actual travel order.
Arrow keys browse; Enter commits the highlighted stop; Escape closes the list.
Results include actual destinations and the next stop, so same-name platforms
can be distinguished by their route rather than a guessed odd/even platform
code. Choosing a line reveals a diagram of its ordered paths, including forks,
rejoins and repeated stops. Preview another path, then choose a platform from
the diagram or its keyboard-accessible stop list. Non-boarding terminals remain
visible but disabled. Picking a call sets its exact platform and direction;
previewing a path alone does not change the choice. Saved timetables include
all paths serving that platform/direction. Paths describe the catalogue
snapshot, not a claim of current service or a geographical street map.
A line and boarding stop must be selected explicitly. Saved choices reopen
intact; outdated identities ask for an updated choice. Forward and reverse
catalogue indexes are built once. Empty groups show the device’s real empty state.

Group tabs support Left/Right, Home and End. Removing a group or choice offers
Undo until the next settings edit.
Import uses a keyboard-accessible button and offers Undo, including the previous
group switching mode. Invalid imports leave groups and any existing Undo intact.
Failed catalogue or firmware downloads have
retry controls that retain group edits. Browser history restores those edits,
restores saved settings and resets password visibility. Search Enter moves to
the choices; it does not submit an unfinished edit.
Weekday toggles retain keyboard focus after updating. The Rust preview prioritizes catchable departures and shows the device’s
zero-percent message when no displayed departure allows enough walking time.

The browser downloads `latest.json` and `timetable-display.bin`, validates board,
settings format, size and SHA-256, then creates local Blob URLs for the factory
image, CRC-protected settings and ESP Web Tools manifest. The settings image
replaces both flash banks, so a previous device-side generation cannot override
a fresh installation. Any edit invalidates the prepared installation.
Settings are remembered in this browser by default: groups, schedules, walking
preferences, power mode and Wi-Fi credentials. The page explains this beside an
opt-out checkbox and a Forget button; disable it on a shared computer. Forget
erases the stored snapshot while leaving the current tab editable. Storage
failures show a message and keep editing usable. Passwords are hidden after
navigation. Group exports still exclude credentials. No analytics or remote
settings service is used; credentials reach the device only over the selected
USB port. ESP Web Tools 10.4.0 is pinned and bundled locally.

PL/EN controls translate the page, picker, validation and diagnostics without
changing editable names, stop names or prepared settings. The browser language
sets the initial choice; an explicit choice is saved locally. The USB flashing
dialog is provided by ESP Web Tools and remains in English.

“Read device logs over USB” opens a passive 115200-baud serial monitor in desktop
Chrome or Edge on HTTPS/localhost. It never writes flash or changes reset/control
lines. Close the flasher/other serial monitors first; press the device's RESET
button to capture startup. Disconnect logs before installing. The view keeps the
latest 65,536 characters, supports Clear and Download, and releases the port on
disconnect. Logs are not saved to browser storage or uploaded. Reconnect if a
reset or USB removal changes the port.

The little screen runs the same model, touch handlers, catalogue and renderer
as firmware, with sample departures. Hold to edit, tap lists and swipe groups.
**Demo hardware** exposes simulated Wi-Fi/NTP/update/offline states; browser time
stands in for NTP. Use **Use demo changes** to copy committed demo edits to the
installer, or **Reset demo** to discard them. Actual Wi-Fi credentials stay out
of WASM. The device UI remains English; the page and demo controls support PL/EN.
See [adapter details and tests](../web-preview/README.md).

```sh
rustup target add wasm32-unknown-unknown --toolchain 1.95.0
cargo +1.95.0 install wasm-bindgen-cli --version 0.2.129 --locked
npm --prefix web ci
npm --prefix web run build
npm --prefix web test
# Requires a built firmware artifact for a real install; previews work without it.
node web/tests/server.mjs
# In web/, after installing Chromium:
npx playwright install chromium
npm run test:browser
```

The six-departure preview uses illustrative times. The installation note explains
that saved device times appear before Wi-Fi, with countdowns and walking advice
waiting for clock synchronization. Both messages are available in PL and EN.

The dev server uses <http://127.0.0.1:4197/>. Tests cover desktop and mobile group
editing, exact USB settings payloads, absence of credentials from network
requests, invalidation after edits and refusal of corrupt firmware. They mock
public release responses, so no USB device or live transit service is needed.
The headless browser needs working system fonts/fontconfig. On the development
host an isolated temporary `FONTCONFIG_FILE` was used to avoid a shared font
cache issue; CI installs Chromium's normal dependencies.

`tools/assemble-installer.py` produces `dist/installer/`, including the exact
firmware, catalogue, Android APK and browser ADB transport. For local Android
setup, [build the APK](../android/README.md) first. The Android panel shares the
network and groups above and uses the tablet’s existing Wi-Fi; its power, Home
and device-owner options are separate from ESP32 installation. Main builds publish that directory to Pages; PRs retain
it as an Actions artifact. See [firmware instructions](../firmware/README.md).

References: [ESP Web Tools](https://esphome.github.io/esp-web-tools/),
[Web Serial](https://developer.chrome.com/docs/capabilities/serial).


## Display models

Choose GUITION ESP32-C3 (240×240) or Waveshare ESP32-C6-**Touch**-LCD-1.47
(172×320). The browser remembers the model alongside the current draft. Changing
models invalidates a prepared install and downloads that model's verified
release; settings, selected transport network and routes stay available. The
preview switches to the same native Rust layout and hit regions as that board.
The C6 firmware lives under `devices/waveshare-c6/`; default C3 URLs remain
compatible. Chip family, board identity, flash size and application header must
agree, including when a catalogue is inserted into C6's unified ROM segment.
Both builds keep the settings/cache partitions at the same addresses. C6 screen
standby retains live touch polling and CPU idle sleep rather than deep sleep.

The C6 Orientation selector offers portrait (172×320) and landscape (320×172).
It changes the native shared Rust layout and downloads the corresponding release,
without losing route choices. Saved model IDs from older installers still select
portrait. Orientation is part of release validation and browser persistence; it
is installed with the app, so changing a device's orientation requires reinstalling.
Landscape uses two large route/stop rows per page, 2×2 group/choice cards and
three wide walking controls. See the [rendered examples](../docs/waveshare-c6.md#landscape-alternative).
