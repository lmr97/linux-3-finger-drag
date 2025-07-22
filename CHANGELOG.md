# Changelog

## 1.3.0 - 2025-07-21

### Changed

- Change all configuration options be optional
- Slim down manual install steps
- Change install script to match more minimal install steps
- Change error handling in config file parsing

### Added

- Troubleshooting section to README
- Table of contents to README

## 1.2.0 - 2025-07-19

### Added

- Add optional config field `logFile` to send logs to an external file; defaults to standard output if the field is not set (or file path is invalid)
- Add optional config field `logLevel` to set the logging verbosity level. This can be one of the following values (from least to most verbose):
    1. `off`
    2. `error`
    3. `warn`
    4. `info`
    5. `debug`
    6. `trace`
    For more info on these levels, see the documentation for [the `enum`](https://docs.rs/log/0.4.6/log/enum.Level.html) to which these values correspond.
- Add optional config field `failFast` which will tell the program to exit on the first runtime error (after setup; errors occurring during setup will always cause the program to exit) 
- Add more detailed startup logging
- Add Contents section to the README for easier navigation

### Changed

- Cause program to exit with non-zero status on fatal errors during startup
- Install script will now only soft-reboot (reboot user-space only) instead of fully reboot
- Prevent program from crashing during runtime in any control path (unless configured to, see the Added section for this version), but will log errors to the console
- Change logging mechanism to `simplelog` crate


## 1.1.4 - 2025-07-16

### Fix

- Fix method of locating user trackpad, base it on device capabilities, not name


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

