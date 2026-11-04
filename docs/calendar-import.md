# Calendar links

In Settings > Calendar, paste an iCal (.ics) address from Google, Outlook or another
calendar provider. A single-calendar public Google embed link also works:
SuiNotch converts its `src` calendar ID to the public iCal feed automatically.
Previously saved embed links are converted when syncing, so they can be refreshed
without pasting again. The calendar must be public to use its public feed.

For a private Google calendar, use its Secret address in iCal format from
Google Calendar > Settings > your calendar > Integrate calendar. SuiNotch does not
sign into Google or change sharing settings. Combined embed links with several
`src` calendars need to be replaced by a link to one calendar.

Events refresh every 15 minutes, or immediately when you choose Refresh. SuiNotch
loads occurrences from 35 days ago through 70 days ahead; dates outside that range
are not imported. Calendar times come from the iCal data, not an embed page's
display-timezone parameter. The last valid feed stays available offline.

HTML calendar pages, sign-in responses and invalid calendar data show an error
instead of being recorded as a successful sync with zero events. URLs and private
calendar tokens are not included in refresh error logs.
