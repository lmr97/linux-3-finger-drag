# Changelog

## 1.1.0 - 2025-07-01

### Fixed

- Fix the cursor drift while holding three fingers at rest
- Fix upward drift of the the relative starting point 

### Added

- Add an option in `3fd-config.json`, called `min_motion`, which determines the minumum value of the relative motion needed to actually move the cursor during a drag gesture (the default is 0.2)
- Add camelCase compatibility for `3fd-config.json`; it will parse appropriately whether the keys are `camelCase` or `snake_case`
- Add this changelog

## 1.0.0 - 2025-06-30

## Changed

- **Central design**: instead of using regex to parse a text stream from std-out after a command-line call to `libinput debug-events`, the program now uses proper `libinput` bindings for Rust to parse events ([`input.rs`](https://crates.io/crates/input)). This reduces memory usage significantly.
- File structure, readability

## Added

- Automatically detect trackpad, listen only to events from that device


## 0.2.1 - 2025-04-22

## Fixed

- Fix drag persisting after pinch gestures


## 0.2.0 - 2025-02-26

## Added

- Add support for `XDG_HOME` (contributed by @Diegovsky)

