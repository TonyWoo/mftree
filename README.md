# mftree

WizTree-style disk space analyzer written in Rust.

On Windows it reads the NTFS **Master File Table ($MFT) directly** instead of
walking the directory tree, so a full-disk scan finishes in seconds. The UI
shows the largest files/folders and a treemap, just like WizTree.

On other platforms it falls back to a plain recursive directory walk (useful
for development).

## Build

```sh
cargo build --release
```

## Run

Windows: **run as Administrator** (raw disk access requires it).

```sh
./target/release/mftree
```

Pick a drive, hit **Scan**. Click a treemap block to highlight the file in the
list; hover for the full path.

## Layout

- `src/mft.rs` — $MFT parser (boot sector → fixup → run list → FILE records →
  `$FILE_NAME` / `$DATA`), plus path rebuilding and directory aggregation.
- `src/treemap.rs` — squarified treemap layout.
- `src/main.rs` — egui/eframe UI.

## Status

Early prototype. Tested parsing logic on synthetic MFT records; real-volume
testing on Windows pending.
