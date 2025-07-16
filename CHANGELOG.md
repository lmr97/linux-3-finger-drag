# Changelog


## 1.1.3 - 2025-07-15

### Changed

- Change the error-catching logic and messages to be more detailed

### Added

- **Update script**: Add the command to actually build the new version once pulled
- **Update script**: Add a check of the working directory to ensure the script is running from the repo directory

## 1.1.2 - 2025-07-15

### Fixed

- Fix ambiguity of error cause when program cannot find trackpad, throwing separate error for permission issue vs `libinput` naming issue

### Changed

- Change error/warning styles, so they have colors!

## 1.1.1 - 2025-07-15

### Fixed

- Fix bug where devices named "trackpad" (instead of "touchpad") are not found (issue [#8](https://github.com/lmr97/linux-3-finger-drag/issues/8#issuecomment-3073401437))

### Changed

- **Install script**: installation does not give the executable root ownership (since the necessary privileges are set via `uinput` rules)

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

