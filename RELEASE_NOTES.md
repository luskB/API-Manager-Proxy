## APIManagerProxy v1.7.1

This release packages the latest stable state of the desktop proxy app and focuses on reliability around CLI sync, local proxy behavior, and day-to-day account operations.

### Highlights

- Stabilized CLI model sync behavior for local agent tools.
  Model selections now remain easier to reason about when syncing to tools like OpenCode and Claude Code, while keeping the proxy-facing model routing behavior consistent.

- Improved proxy workflow diagnostics and recoverability.
  The current build keeps the existing proxy flow intact while preserving recent fixes around status refreshes, port-conflict handling, and source-site visibility in model pickers.

- Refined UI behavior across monitoring, Hub management, and token statistics.
  Bulk actions, time-window views, and operational panels are carried forward in a cleaner release baseline.

### Included fixes

- Preserved current CLI sync compatibility without bundling local machine configuration into the repository.
- Kept proxy page model-source tooltips and routing-aware model selection behavior.
- Carried forward the recent dashboard, monitoring, Hub, and token-statistics improvements into a tagged release.

### Privacy and packaging

- This release is built from repository source only.
- Local runtime configuration, browser profiles, logs, caches, and build outputs remain excluded from source control.
- GitHub Release artifacts are produced by Actions from the sanitized repository state.

### Downloads

| Platform | Architecture | File |
| --- | --- | --- |
| Windows | x64 | `APIManagerProxy_1.7.1_x64-setup.exe` |
| Windows | x64 | `APIManagerProxy_1.7.1_x64_en-US.msi` |
| macOS | Apple Silicon / Intel | Built by GitHub Actions |
| Linux | x64 | Built by GitHub Actions |
