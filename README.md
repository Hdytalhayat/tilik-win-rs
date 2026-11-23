# Tilik

Fast, local desktop search for Windows 11 built with Rust. Indexes your configured folders and lets you find files by name or content instantly via a global hotkey or the system tray.

## Key features

- **Full-text & filename search** across configured folders (`.txt`, `.md`, `.json`, `.csv`, source code, `.pdf`, `.docx`, `.xlsx`).
- **Incremental indexing** backed by a real-time file watcher so only new or modified files are re-indexed.
- **Instant keyboard workflow** with live results, document snippet preview, and full keyboard navigation (`Up`/`Down`, `Enter` to open, `Ctrl+Enter` to reveal in Explorer, `Esc` to hide).
- **Global hotkey** to toggle the search window from anywhere.
- **System tray integration** with quick actions: *Open Search*, *Re-index Now*, *Start with Windows*, and *Quit*.
- **Background-first lifecycle** where closing the window (`X`) hides it to the tray without stopping the background indexer, protected by a Windows single-instance mutex.

## Default hotkey

- `Ctrl+Alt+F` (configurable in `%APPDATA%\localsearch-win\config.toml`)

`Alt+Space` is also supported, though it may conflict with Windows system menus or launchers like PowerToys Run, so it is not enabled by default.

## Configuration and index locations

- **Config:** `%APPDATA%\localsearch-win\config.toml`
- **Index:** `%APPDATA%\localsearch-win\index\`
- **Manifest:** `%APPDATA%\localsearch-win\index\manifest.toml`

## Download & Releases

Prebuilt Windows binaries are published on **GitHub Releases**:

- **Portable archive:** `tilik-win-rs-portable.zip` (extract and run `tilik-ui.exe` directly)

Download the latest release from the repository's **Releases** page on GitHub.

## Build from source

**Prerequisites:** Rust toolchain (`stable`, `x86_64-pc-windows-msvc` target).

```powershell
cargo build --release
.\target\release\tilik-ui.exe

```

To build the release package locally before uploading to GitHub Releases:

```powershell
.\scripts\build-release.ps1

```

## AI-Assisted Development (`OpenCode` & `AGENTS.md`)

This project was developed with the **OpenCode** terminal coding agent governed by strict repository rules defined in [`AGENTS.md`](https://www.google.com/search?q=./AGENTS.md). Every AI-assisted change follows a deterministic execution protocol:

1. **Explore & Plan:** Inspect relevant modules and context first, outlining a concise step-by-step plan before modifying any code.
2. **Surgical Execution:** Apply minimal, targeted edits without altering unrelated logic or deleting/weakening tests to bypass failures.
3. **Non-Interactive Commands:** Execute all CLI/Cargo commands with non-interactive flags so automated runs never hang.
4. **Verify with Evidence:** Run `cargo check`, `cargo build`, or `cargo test` after changes and validate raw terminal output before claiming completion.
5. **Retry Cap:** Stop after 3 failed attempts on the same approach, document the root cause, and escalate for review.
6. **Final FMEA Check:** Audit top edge cases and potential regressions (such as file locks, encoding errors, or hotkey conflicts) and document remaining caveats before closing a task.

## License

This project is licensed under the **MIT License**. See the [`LICENSE`](https://www.google.com/search?q=./LICENSE) file for details.
