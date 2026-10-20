# Timetable USB installer

A static, responsive installer with a porcelain / ink / gold / chilli palette.
Group changes update the illustrative round-screen preview. The route/stop
picker uses the same checked-in official catalogue as firmware. It supports
four named groups, four choices per group, direction filters, per-choice
walking time and optional weekday/time windows. Export/import contains group
settings only, never Wi-Fi credentials.
“Follow group schedules” controls automatic switching for all groups. Turn it
off for touch-only selection without deleting time windows; imported manual
settings can be inspected and switched back to automatic mode here.

Battery mode is off by default. Enable it to sleep the screen and CPU after
30 seconds on the idle departure board, wake by touch and refresh Wi-Fi every
three hours. Cached wakes do not initialize Wi-Fi. Group imports leave this
device preference unchanged. See [battery operation and hardware limits](../docs/battery.md).

Search any stop by name or platform code in one combobox. Platforms with the
same name share one result, with all their serving lines. Choose a line and
destination to resolve the exact boarding platform; the hint shows its code,
next stop and actual destinations. Multiple platforms in one direction remain
separate choices. Polish accents are optional for search, while distinct accented
names remain distinct groups. You can also start with a line number or destination.
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
catalogue indexes are built once. Empty groups have an empty preview.

Group tabs support Left/Right, Home and End. Removing a group or choice offers
Undo until the next settings edit.
Import uses a keyboard-accessible button and offers Undo, including the previous
group switching mode. Invalid imports leave groups and any existing Undo intact.
Failed catalogue or firmware downloads have
retry controls that retain group edits. Browser history restores those edits,
restores saved settings and resets password visibility. Search Enter moves to
the choices; it does not submit an unfinished edit.
Weekday toggles retain keyboard focus after updating. The walking preview
distinguishes a possible rush from insufficient walking time, matching the
device's zero-percent message.

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

```sh
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
firmware and catalogue. Main builds publish that directory to Pages; PRs retain
it as an Actions artifact. See [firmware instructions](../firmware/README.md).

References: [ESP Web Tools](https://esphome.github.io/esp-web-tools/),
[Web Serial](https://developer.chrome.com/docs/capabilities/serial).
