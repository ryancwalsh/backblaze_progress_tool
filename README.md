# Backblaze Progress

A small desktop app that shows **every file Backblaze Personal Backup still has queued**, with its size and the most likely reason it hasn't uploaded yet, in a sortable, filterable table. It also has panels for Backblaze's own health signals, a skipped-files ("issues") report, exclusions, recent backup passes and a history of the "remaining" figure.

**It is read-only and offline.** It reads Backblaze's local files and never changes them. It has no network code, and its content security policy blocks any outside connection. The only things it writes are its own history file and the CSV files you export.

## One-time setup (Windows)

1. **Rust:** install from <https://rustup.rs> (accept the defaults). If you already have Rust, make sure the MSVC toolchain is the default: `rustup default stable-msvc`.
2. **Microsoft C++ Build Tools:** <https://visualstudio.microsoft.com/visual-cpp-build-tools/>. In the installer, tick **"Desktop development with C++"**.
3. **WebView2 Runtime:** usually already on Windows 10/11. If the app opens to a blank window or complains, install the "Evergreen Bootstrapper" from <https://developer.microsoft.com/microsoft-edge/webview2/>.

## Build

Double-click `build.cmd`, or run it from a normal (non-admin) terminal:

```
cd C:\code\backblaze_progress_tool
build.cmd
```

The first run installs the Tauri CLI (`cargo install tauri-cli`) and downloads libraries, so it takes a while. Later builds are quick. The result is `Backblaze Progress.exe` in this folder, a single file you can move anywhere.

## Run

Double-click `Backblaze Progress.exe` and approve the Administrator prompt. Backblaze's data folder (`C:\ProgramData\Backblaze\bzdata`) can only be read by administrators, so the app asks for elevation every time it starts.

If Backblaze's data is somewhere else, set the `BZDATA` environment variable to that `bzdata` folder.

## What you see

* **Cards:**
  * Backblaze's own "remaining" figure, and what this app counts as not yet uploaded
  * The to-do list's size and when it was written
  * The file uploading now, with large-file chunk progress
  * Speed and key settings (threads, throttle)
  * When the last backup pass finished, and what's remaining per drive
* **Table:** one row per file in the newest to-do list. Click a header to sort, and use the search box and drop-downs to filter. Click a row for details; double-click opens the file's folder in Explorer.
* **Health:**
  * Problems found in Backblaze's logs and reports: unreadable upload-history lines, repeating self-healing requests, folders it couldn't scan, files it can't read
  * Whether uploads use a single thread
* **Groups:** totals by status, folder, extension or drive for the current filter. Click a group to filter to it.
* **History:** Backblaze's "remaining" figure each time the app refreshed. It's stored in `%APPDATA%\local.bzprogress.viewer\remaining_history.csv`.
* **Skipped report:** Backblaze's `bzlist_skipped_files.txt`.
* **Exclusions:** excluded file types and folders, and the advanced rules.
* **Backup passes:** Backblaze's event log (start, end, and how many files each pass backed up).

### Statuses

| Status | Meaning |
|---|---|
| Uploading | The file Backblaze is sending right now. |
| Partly uploaded | Some 10 MB chunks of a large file have been sent. |
| Uploaded | Recorded as uploaded after the to-do list was written. It disappears at Backblaze's next rescan. Hidden by default. |
| Gone | The file no longer exists, or the transmit log says it was skipped because it no longer exists. |
| In use | Skipped-files report: `TEMPORARY_FILE_BUSY`. Retried automatically. |
| No permission | Skipped-files report: `PERMANENT_BAD_PERMISSIONS`, or access denied when opening. Not retried. |
| Locked | This app couldn't open the file because another program has it locked. |
| Cloud-only | A OneDrive (or similar) placeholder that isn't downloaded. |
| Path too long | Path is 260+ characters, or invalid. |
| Changed | Modified again after Backblaze queued it. Backblaze will read the newer version. |
| Waiting | No problem found: it's in the queue. If the transmit log mentions the file, its last message is shown. |

"Check files on disk" makes the app look at each pending file: whether it exists, whether it's locked, whether it changed, and whether it's a cloud-only placeholder. Turn it off for a faster refresh.

## How the Backblaze files are interpreted

These formats are undocumented. They were worked out from client 9.2.2 on Windows, so treat the reasons as well-informed guesses.

* `bzbackup\bzdatacenter\bz_todo_*.dat` (newest file): the queue. Tab-separated fields: `1`, `+`, file id (hex), file modified time (hex ms), **size in bytes**, **path**.
* `bzbackup\bzdatacenter\bz_done_*.dat`: upload records. Field 4 is the GMT time (`yyyymmddhhmmss`), field 8 is the chunk marker (`k0_` = whole file, otherwise a chunk of a large file), field 13 is the size and field 14 the path. A `!` in field 2 is treated as "large file finished".
* `bzreports\bzlist_skipped_files.txt` (and `.old`): `time <TAB> CODE <TAB> ... <TAB> path`.
* `bzreports\bzstat_remainingbackup.xml`, `bzstat_totalbackup.xml`, `bzvolumes.xml`: totals per drive.
* `bzbackup\bzdatacenter\bzcurrentlargefile\*.xml` and the `<bztransmit cur_state=...>` XML in `bzdata`: what's uploading now.
* `bzlogs\bztransmit\*.log`, `bzlogs\bzfilelist\*.log`: per-file warnings and health counters.
* `bzinfo.xml`, `bzexcluderules_editable.xml`: settings and exclusions.

`probe.ps1` and `probe2.ps1` are the redacted structure probes used to work these out. Re-run them if a Backblaze update changes the formats.

## Mac

The same code should build on a Mac (`cargo tauri build` inside `src-tauri`), but it is **untested**. It reads `/Library/Backblaze.bzpkg/bzdata` by default; set `BZDATA` if yours is elsewhere. That folder is root-only, so give the app Full Disk Access or start it with `sudo`. Linux isn't supported because Backblaze Personal Backup has no Linux client.

## Development

`cargo tauri dev` (inside `src-tauri`) must be run from an **Administrator** terminal, because the app's manifest requires elevation. You can also open `ui/index.html` through any local web server with a `sample-snapshot.json` next to it to work on the UI without Backblaze.
