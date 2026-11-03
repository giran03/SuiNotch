# Updates tab

The notch adds an **Updates** tab when a newer release is available. The tab is
temporary and does not appear in the permanent page toggles. Finding an update
keeps your current page selected; an accent ring marks the new tab in the page bar.

- **Available:** Download update saves and verifies the installer without immediately
  restarting SuiNotch.
- **Downloading:** An activity indicator shows that the update is downloading and
  being checked. No percentage is shown unless one is actually known.
- **Ready:** Install and restart checks the saved installer again, then starts the
  existing silent update process.
- **Installing:** The tab remains visible while the installer is running. SuiNotch
  normally closes and the updated copy reopens with the tab hidden.
- **Failed:** The error stays visible with Retry download. Release notes remain
  accessible throughout the flow.

When installation finishes, or a check finds there is no update to install, the tab
hides. If it was selected, SuiNotch returns to the previous page, or an enabled page
if that page has since been hidden. A background tab disappearing never changes
the page you are using. Routine update checks preserve a download that is already
ready and do not briefly remove the tab.

The existing **Settings > About > Automatic updates** preference still applies:
Off disables background checks, Notify me leaves installation to you, and Automatic
can install after the PC has been idle for ten minutes. Manual Check now remains
available in About even when background checks are off. Changing away from Automatic
cancels its pending wait to install. About also shows the same download/install states.

Downloads use the release's installer checksum and are verified again before launch.
Other update operations stay locked out while an installer is running. If the
installer reports failure, the tab offers a retry rather than claiming success.

Native tab and rendering tests never download or run an installer. To run the
isolated test in a separate PowerShell session:

```powershell
$env:APPDATA = Join-Path (Get-Location) 'target/update-tab-test'
cargo test --locked update_tab_appears -- --ignored --test-threads=1
```

Snapshots of available, downloading, ready, installing and failed states at 100%,
150% and 200% scaling are saved under that directory. Standard updater tests cover
state transitions, busy locking, installer filenames and checksum revalidation.
