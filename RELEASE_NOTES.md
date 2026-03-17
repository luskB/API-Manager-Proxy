## APIManagerProxy v1.7.2

This release publishes the current stable desktop build and rolls up the latest proxy, monitoring, Hub, token-statistics, and CLI-sync improvements into a new tagged baseline.

### Highlights

- Improved model-source clarity across the app.
  Site-aware model labels, route-aware selection behavior, and source visibility improvements are carried forward for both proxy usage and CLI sync workflows.

- Refined operational screens for daily use.
  Dashboard, Hub, monitoring, and token-statistics views include the latest interaction fixes, time-window behavior, and UI refinements from the current workspace state.

- Preserved the current local proxy workflow while keeping recent quality-of-life updates.
  The release keeps the existing desktop proxy behavior intact while packaging the latest stable changes from the local project.

### Included in this release

- Current CLI sync UI and selection management updates.
- Recent monitoring and token-statistics screen improvements.
- Ongoing proxy, Hub, and account-management refinements from the current stable codebase.

### Privacy and packaging

- This release is built from repository source only.
- Local runtime configuration, API keys, tokens, browser profiles, caches, logs, and build outputs remain excluded from source control.
- GitHub Release artifacts are produced by Actions from the sanitized repository state.

### Downloads

| Platform | Architecture | File |
| --- | --- | --- |
| Windows | x64 | `APIManagerProxy_1.7.2_x64-setup.exe` |
| Windows | x64 | `APIManagerProxy_1.7.2_x64_en-US.msi` |
| macOS | Apple Silicon / Intel | Built by GitHub Actions |
| Linux | x64 | Built by GitHub Actions |
