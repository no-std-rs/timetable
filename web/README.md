# Timetable USB installer

A static, responsive installer with a porcelain / ink / gold / chilli palette.
Group changes update the illustrative round-screen preview. The route/stop
picker uses the same checked-in official catalogue as firmware. It supports
four named groups, four choices per group, direction filters, per-choice
walking time and optional weekday/time windows. Export/import contains group
settings only, never Wi-Fi credentials.

Search by line number or destination, then by stop name or platform code; Polish
accents are optional. Results stay within the chosen line and direction. A line
and boarding stop must be selected explicitly. Saved choices reopen with their
selection intact, and outdated catalogue identities ask for an updated choice.
The catalogue search index is built once, so typing scans only the chosen
route's stop links. Empty groups have an empty preview rather than sample trips.

Group tabs support Left/Right, Home and End. Removing a group or choice offers
Undo until the next settings edit. Failed catalogue or firmware downloads have
retry controls that retain group edits. Browser history restores those edits,
clears Wi-Fi credentials and resets password visibility. Search Enter moves to
the choices; it does not submit an unfinished edit.

The browser downloads `latest.json` and `timetable-display.bin`, validates board,
settings format, size and SHA-256, then creates local Blob URLs for the factory
image, CRC-protected settings and ESP Web Tools manifest. The settings image
replaces both flash banks, so a previous device-side generation cannot override
a fresh installation. Any edit invalidates the prepared installation.
No local storage, analytics or third-party runtime
scripts are used; ESP Web Tools 10.4.0 is pinned and bundled locally. Credentials
are cleared on page navigation and sent only over the user-selected USB port.

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
