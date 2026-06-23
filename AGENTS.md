# rustReader Agent Instructions

## Project Overview

`rustReader` is a desktop comic/manga reader built with Rust, `eframe`, `egui`, and `wgpu`.
It supports ZIP/CBZ, RAR/CBR, PDF, and folders of images.

## Repository Layout

- `rust-reader-core/` — shared models, reading-state machine, and layout math.
- `rust-reader-parser/` — archive/folder/PDF parsers and comic ID generation.
- `rust-reader-storage/` — JSON persistence for settings, library, history, and bookmarks.
- `rust-reader-app/` — egui application, cache, loader, and UI views.
- `docs/` — audit reports, bug notes, and implementation plans.

## Build & Test

Run the full verification pipeline before committing:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Coding Conventions

- Use `cargo fmt` for formatting.
- Keep `clippy` warnings at zero (`-D warnings`).
- Prefer minimal, focused changes; avoid unrelated refactoring.
- Update relevant tests when changing public interfaces.
- Keep UI text in Chinese unless it is a proper noun or technical identifier.

## Key Architectural Notes

- **Comic IDs** are generated deterministically from the file/folder path via
  `rust_reader_parser::stable_comic_id`. Never use the filename alone.
- **PageLoader** runs IO and decode workers in background threads; results are
  sent back to the UI thread via channels.
- **PageCache** stores GPU textures and keeps `size_bytes` as an estimate of
  either CPU image memory or equivalent GPU memory.
- **Settings** are validated on load/save; invalid values are clamped and the
  user is informed through `error_message`.
- **History entries** store both `comic_id` and `path` for robust matching.

## Commits

- Commit after each completed task or logical change.
- Push to `main` when verification passes.
- Summarize the change and affected crates in the commit message.
