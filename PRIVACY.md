# Privacy

SuiNotch has no accounts, no analytics and no telemetry. Everything it keeps about you stays on your PC.

## What stays on your PC

Your settings, to-dos, water log, stats, lyrics timing tweaks and shelf items are saved in `%APPDATA%\SuiNotch`, along with a small log file for troubleshooting. Looked-up lyrics are kept in `lyrics_cache` (songs with none are remembered for three days), lyrics you marked wrong in `lyrics.json`, and, unless you turn off Settings > Music and lyrics > Learn what plays next, which songs tend to follow which in `transitions.json`, so the next song's lyrics can be fetched ahead. Clear recently played deletes that history too. Lyric files you put in the `lyrics` folder are read there and never uploaded. If you add a calendar link, the link and a copy of the calendar are kept there too. Nothing in that folder is uploaded anywhere. Uninstalling SuiNotch and deleting that folder removes all of it.

Clipboard history is saved in `%APPDATA%\SuiNotch` (`clips.json`, with copied images in the `clips` folder) so it survives a quick restart such as an update. If SuiNotch was off for longer than the time set in Settings > Clipboard (5 minutes by default), it is deleted at the next start; set that to Never to keep the history in memory only. Copies that password managers and other apps mark as private are never kept. You can turn clipboard history off in Settings > Clipboard, which deletes it.

Song details and playback state come from Windows' own media controls and are only shown in the notch.

Imported reminder sounds are copied into the `sounds` folder in `%APPDATA%\SuiNotch`. Sound names, clip boundaries, volume and reminder assignments are saved with your settings. Imports and audio processing stay on your PC. Delete an imported sound in Settings > Sounds to remove its local copy.

## What leaves your PC

SuiNotch makes only these network requests:

| Service | When | What is sent |
|---|---|---|
| [LRCLIB](https://lrclib.net) | After likely song playback remains eligible for two seconds, or when you request a manual search; respects per-player preferences and Lyrics Off. With Fetch the next song's lyrics ahead (on by default), also for the song likely to play next, at most 30 times an hour and never on a metered connection | The song's title, artist, album and length, to look up synced lyrics |
| Backup lyrics services (two music services' lyrics search) | Only with Fall back to other services (on by default): when LRCLIB has no synced lyrics, or for romaji, translations and word timing you chose to show | The song's title, artist and length |
| [Apple's music store search](https://itunes.apple.com) | Only with Fall back to other services: to find a song's native-script name, to check that a track from a player not made for music is a song before its lyrics are looked up, and to find the next track of an album for fetching ahead | The song's title, artist and album |
| [GitHub](https://github.com/giran03/SuiNotch/releases) | Checking for updates | A request for the latest release; an installer is downloaded only when a newer version exists |
| Your calendar provider | Only if you add a calendar link, every 15 minutes | A request for that calendar, to the address you pasted |

Like any web request, these services can see your IP address, and their own privacy policies apply to it. No other information about you or your PC is sent.
