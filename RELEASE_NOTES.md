## APIManagerProxy v1.6.0

This release focuses on stability and day-to-day usability for local proxy workflows and CLI sync.

### Highlights

- Improved proxy status detection on the Proxy page.
  The app now corrects stale "stopped" states after startup and handles local port conflicts more gracefully when the proxy is already active.

- Better model diagnostics in the Proxy page.
  When configuring per-key model access, hovering a model now shows its source site information, making it easier to confirm where each model comes from.

- CLI sync now keeps site-aware model selections more reliably.
  Site-prefixed model labels remain visible in the sync list, while synced CLI configs still receive bare model IDs for compatibility.

- CLI sync selection UX has been cleaned up.
  The selected model list is easier to review, bulk actions are clearer, and the sync panel is more consistent with the rest of the app theme.

### Included fixes

- Refined route-backed CLI model syncing so site-specific selections remain usable with local proxy routing.
- Improved startup-time proxy state refresh behavior to reduce false stop/start confusion.
- Polished the Proxy page model picker tooltips and preserved the current working proxy behavior.

### Downloads

| Platform | Architecture | File |
| --- | --- | --- |
| Windows | x64 | `apimanagerproxy.exe` |
| Windows | x64 | `APIManagerProxy_1.6.0_x64-setup.exe` |
| Windows | x64 | `APIManagerProxy_1.6.0_x64_en-US.msi` |
| macOS | Apple Silicon / Intel | Built by GitHub Actions |
| Linux | x64 | Built by GitHub Actions |
