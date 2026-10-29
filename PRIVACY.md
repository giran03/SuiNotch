# Privacy

SuiNotch has no accounts, no analytics and no telemetry. Everything it keeps about you stays on your PC.

## What stays on your PC

Your settings, to-dos, water log and shelf items are saved in `%APPDATA%\SuiNotch`, along with a small log file for troubleshooting. Nothing in that folder is uploaded anywhere. Uninstalling SuiNotch and deleting that folder removes all of it.

Song details and playback state come from Windows' own media controls and are only shown in the notch.

## What leaves your PC

SuiNotch makes only two kinds of network request:

| Service | When | What is sent |
|---|---|---|
| [LRCLIB](https://lrclib.net) | Once each time a new song starts playing | The song's title, artist, album and length, to look up synced lyrics |
| [GitHub](https://github.com/giran03/SuiNotch/releases) | Checking for updates | A request for the latest release; an installer is downloaded only when a newer version exists |

Like any web request, these services can see your IP address, and their own privacy policies apply to it. No other information about you or your PC is sent.
