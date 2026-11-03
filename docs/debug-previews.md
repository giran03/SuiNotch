# Debug previews

Settings > About > Debug contains local feature tests:

- **Preview Updates tab** opens the notch on a temporary simulated update. Next test state advances through Available, Downloading, Ready, Installing and Failed; Finish preview closes it. Close preview, switching tabs, opening Settings from the notch, Escape or collapsing the notch also removes the preview and restores normal navigation. Reopening starts fresh.
- **Test volume HUD** collapses the notch and shows a sample volume of 50%. It does not change system volume. Normal full-screen hiding and an active reminder can suppress it.
- **Test notification** shows a short message using the existing temporary notice display.

Debug state is kept only in memory. Simulated updater status is separate from real updater status and has separate UI actions. No preview action checks for updates, downloads files, launches an installer or changes preferences. If a real actionable update arrives, it replaces the preview. Closing a preview does not remove an actual update tab.

Diagnostics, already accessible from About, still provides real system details and log access.

Validation uses the native update lifecycle test with APPDATA ending in `update-tab-test`, and `about_debug_previews_render_and_dispatch` with APPDATA ending in `sound-test`. Both run with `cargo test --locked --offline <test-name> -- --ignored --test-threads=1`, render at 100/150/200%, and use disposable data with no network or real installer calls.
