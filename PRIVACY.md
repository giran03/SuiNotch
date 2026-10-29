# Privacy

SuiNotch has no accounts, no analytics and no telemetry. Everything it keeps about you stays on your PC.

## What stays on your PC

Your settings, to-dos, water log, stats, lyrics timing tweaks and shelf items are saved in `%APPDATA%\SuiNotch`, along with a small log file for troubleshooting. If you add a calendar link, the link and a copy of the calendar are kept there too. Nothing in that folder is uploaded anywhere. Uninstalling SuiNotch and deleting that folder removes all of it.

Clipboard history is saved in `%APPDATA%\SuiNotch` (`clips.json`, with copied images in the `clips` folder) so it survives a quick restart such as an update. If SuiNotch was off for longer than the time set in Settings > Clipboard (5 minutes by default), it is deleted at the next start; set that to Never to keep the history in memory only. Copies that password managers and other apps mark as private are never kept. You can turn clipboard history off in Settings > Clipboard, which deletes it.

Song details and playback state come from Windows' own media controls and are only shown in the notch.

## What leaves your PC

SuiNotch makes only these network requests:

| Service | When | What is sent |
|---|---|---|
| [LRCLIB](https://lrclib.net) | Once each time a new song starts playing, and when you press Search again | The song's title, artist, album and length, to look up synced lyrics |
| [GitHub](https://github.com/giran03/SuiNotch/releases) | Checking for updates | A request for the latest release; an installer is downloaded only when a newer version exists |
| Your calendar provider | Only if you add a calendar link, every 15 minutes | A request for that calendar, to the address you pasted |

Like any web request, these services can see your IP address, and their own privacy policies apply to it. No other information about you or your PC is sent.
