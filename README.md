# sizetree

A WizTree-style disk usage analyzer written in Rust. See what's eating your disk at a glance — as a sortable tree and a treemap.

On Windows it reads the NTFS `$MFT` directly (no slow directory walk — even a full drive scans in seconds). On Linux and macOS it falls back to a recursive directory scan.

![license](https://img.shields.io/badge/license-MIT-blue)

## Features

- **Fast scans** — direct `$MFT` parsing on Windows; parallel-friendly walker elsewhere
- **Treemap view** — squarified treemap, click a block to jump to that folder in the list
- **Sortable file/folder tables** — by name, size, or % of total; virtualized for large drives
- **Folder drill-down** — double-click to descend, breadcrumb bar to climb back up
- **Disk space bar** — total / free per drive, with a low-space warning under 10%
- **CSV export** — dump the current view
- **Right-click actions** — reveal in Explorer/Finder, move to trash, permanent delete (with confirmation)
- **Bilingual UI** — 中文 / English toggle
- **Cancellable scans** — stop a long scan any time

## Download

Grab the latest release for Windows or Linux from the
[Releases page](https://github.com/TonyWoo/sizetree/releases).
No installer, no dependencies — just unzip and run.

> On Windows, run as Administrator so sizetree can open the raw volume for `$MFT` access.
> Without elevation it falls back to a regular directory scan.

## Build from source

```sh
cargo build --release
./target/release/sizetree
```

Windows GUI notes:

```sh
# cross-check the Windows-only MFT code compiles (needs the target installed):
rustup target add x86_64-pc-windows-gnu
cargo check --target x86_64-pc-windows-gnu
```

## Usage

1. Pick a drive (or let it default to the system drive).
2. Hit **Scan** — watch the treemap fill in live.
3. Click treemap blocks or double-click folders to drill down; use the breadcrumb bar to go back.
4. Right-click any row to reveal it, trash it, or export the current view to CSV.

## How it works

- **Windows**: opens `\\.\C:` and parses the NTFS boot sector → `$MFT` data runs →
  file records → `$FILE_NAME` attributes, applying the update-sequence fixup and
  preferring Win32 long names over DOS 8.3 short names.
- **Other platforms**: recursive directory walk with symlink-loop protection.

The treemap uses the squarified layout algorithm (Bruls et al.); colors distinguish
sibling folders rather than encoding size (area already does that).

## Development

```sh
cargo fmt --all                                        # format
cargo clippy --all-targets --locked -- -D warnings     # lint (strict)
cargo test --locked                                    # unit tests
```

## License

MIT — see [LICENSE](LICENSE).
