# Privacy

SuiNotch has no accounts, no analytics and no telemetry. Everything it keeps about you stays on your PC.

## What stays on your PC

Your settings, to-dos, water log, stats, lyrics timing tweaks and shelf items are saved in `%APPDATA%\SuiNotch`, along with a small log file for troubleshooting. If you add a calendar link, the link and a copy of the calendar are kept there too. Nothing in that folder is uploaded anywhere. Uninstalling SuiNotch and deleting that folder removes all of it.

Clipboard history is kept in memory while SuiNotch runs; copied images are written to `%APPDATA%\SuiNotch\clips` so they can be dragged out, and that folder is emptied each time SuiNotch starts. Copies that password managers and other apps mark as private are never kept. You can turn clipboard history off in Settings > Shelf.

Song details and playback state come from Windows' own media controls and are only shown in the notch.

## What leaves your PC

SuiNotch makes only these network requests:

| Service | When | What is sent |
|---|---|---|
| [LRCLIB](https://lrclib.net) | Once each time a new song starts playing, and when you press Search again | The song's title, artist, album and length, to look up synced lyrics |
| [GitHub](https://github.com/giran03/SuiNotch/releases) | Checking for updates | A request for the latest release; an installer is downloaded only when a newer version exists |
| Your calendar provider | Only if you add a calendar link, every 15 minutes | A request for that calendar, to the address you pasted |

Like any web request, these services can see your IP address, and their own privacy policies apply to it. No other information about you or your PC is sent.
