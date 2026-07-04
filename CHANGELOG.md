# Changelog

## 2.2.0 - 2026-07-04

Production-hardening release.

### Added

- **Randomized invariant fuzzer**: 96 deterministic scenarios x 2500
  steps of chaotic multi-finger input (with realistic tool-bit
  reporting and random SYN_DROPPED resyncs), with a shadow model of the
  synthetic clone verifying after every step that the clone's slot
  state matches the machine's belief, button presses/releases pair
  correctly, the clone never shows more fingers than the real pad, and
  quiescence leaves no phantom touches, stuck tool bits, held button,
  or scheduled wakeups. Mutation-tested (a deliberately broken machine
  fails it immediately).
- Config sanitization: out-of-range values (negative/zero/huge
  acceleration, probeDelay > entryDebounce, multi-second delays) are
  clamped with a logged warning instead of being applied blindly --
  garbage in the config file can no longer produce a broken touchpad.
- `--version` flag.
- GitHub Actions CI: fmt, clippy -D warnings, tests (unit + fuzzer),
  release build.

### Changed

- install.sh is shellcheck-clean; remaining unwraps in the state
  machine converted to documented expects.

## 2.1.0 - 2026-07-04

Fixes flaky 4-finger vertical swipes (overview/grid gestures sometimes
not registering, especially on fast swipes).

### Fixed

- **Late-4th-finger bailout**: on a fast/sloppy 4-finger swipe the 4th
  finger often lands *after* the 50ms entry window, so the machine saw a
  stable 3-finger touch and committed a drag -- eating the swipe. A 4th
  finger arriving during a committed drag now aborts it and hands the
  touch to the compositor mid-gesture (full slot + tool-state
  introduction), so the remaining swipe motion still registers.
- **Deferred button press** (`pressGrace`, default 75ms): the drag's
  button press now fires at the first real drag motion or when the grace
  expires, whichever is first -- so the bailout above normally happens
  before any press, and no phantom click is ever sent. Stationary-hold
  presses and the "3-finger hold = click" behavior are preserved (a
  liftoff inside the grace presses+releases at liftoff).
- **Tool-state consistency on suppress**: entering suppression from a
  partially-relayed touch (e.g. 2 fingers settled, 3rd added to start a
  drag) released the clone's MT slots but left BTN_TOUCH/BTN_TOOL_*
  stuck pressed -- desyncing libinput's finger accounting, and (with
  tap-to-click on) making the yanked touch read as a 2-finger tap =
  phantom right-click. The clone's relayed key state is now tracked and
  explicitly released alongside the slots.

## 2.0.0 - 2026-07-03

Major rework of this fork: the gesture logic is now a pure, fully
unit-tested state machine, and the runtime is event-driven.

### Fixed

- **4-finger liftoff hijack**: the staggered liftoff at the tail of every
  4-finger gesture passes through exactly 3 active fingers; that moment
  was classified as a drag, producing a phantom left-click at the end of
  4-finger swipes. Touches that ever exceeded 3 fingers can no longer
  become drags.
- **Phantom slots after SYN_DROPPED**: the kernel resync read MAX_SLOTS
  entries regardless of the device's real slot range; zeroed tail entries
  (tracking_id 0) counted as active touches. Snapshots are now sized to
  the device's actual ABS_MT_SLOT range.
- Resyncs now also *release* clone slots whose fingers lifted during the
  dropped window (previously the clone could hold a phantom touch
  forever), re-baseline an in-flight drag instead of applying the gap as
  a cursor jump, and end a drag/buffered touch that fully lifted inside
  the drop.
- SYN_DROPPED handling now follows the evdev protocol (discard everything
  up to and including the next SYN_REPORT before resyncing).
- Log files are created if missing (logFile previously required the file
  to already exist).

### Added

- **Pure gesture state machine** (`src/runtime/gesture.rs`) with an
  injected clock, plus a regression test suite encoding every failure
  mode this project has hit live (staggered touchdowns/liftoffs, phantom
  taps, 4-finger transients, drag-lock misbehavior, resync edge cases).
- **Software-in-the-loop integration test**: creates a fake touchpad via
  uinput, drives the real binary against it with `--device`, and asserts
  on the actual clone/virtual-mouse output streams.
- `--device /dev/input/eventN` CLI flag (skips auto-discovery).
- Correct-by-construction **drag-lock** (`dragEndDelay` > 0): a new
  3-finger touch inside the window resumes the same held drag; any other
  touch releases the button *before* its events are relayed — the
  regression that shipped with the first drag-lock attempt (1-finger
  motion dragging things after a drag) is now structurally impossible,
  and tested.
- Sub-pixel motion carry: slow drags no longer lose the fractional
  remainder of each frame's motion to integer truncation.
- In-process touchpad re-discovery on ENODEV (device re-enumeration no
  longer kills the service), with `Restart=on-failure` in the unit as
  backstop.
- A `phys` marker (`linux-3-finger-drag/proxy`) on the synthetic clone so
  discovery can never grab our own clone (includes a manual UI_SET_PHYS
  ioctl: input-linux 0.7's binding mis-encodes the ioctl size).

### Changed

- **Event-driven runtime**: the old 5 ms busy-poll loop (plus its config
  mtime check 200x/s) is replaced by a single-threaded tokio loop that
  sleeps on the device fd and on exact gesture-decision deadlines. Idle
  CPU is zero; decisions land on time instead of on the next poll tick.
- Touchpad discovery inspects evdev capabilities directly; the `input`
  (libinput FFI) and unmaintained `users` crates are gone, as are
  `signal-hook`, `futures-lite`, `async-io`, and the `criterion` dev-dep.
  No C library dependencies remain.
- The drag-end timer thread, control-signal channel, and
  `VirtualTrackpad::clone` machinery are gone; the delay lives in the
  state machine.
- Release profile builds with LTO.

### Removed

- `responseTime` config knob (no poll interval exists anymore; leftover
  entries in existing config files are ignored).
- `response-map.md` (described the pre-proxy libinput design).

## 1.7.0 - 2026-07-03

### Fixed

- Fix subpixel motion, for smoother drags (thanks to [R3-da](https://github.com/R3-da), with [PR #21](https://github.com/lmr97/linux-3-finger-drag/pull/21))

- Fix outdated depependencies


## 1.6.0 - 2025-11-24

### Fixed

- Fix lack of granularity in movement (thanks to [R3-da](https://github.com/R3-da), with [PR #21](https://github.com/lmr97/linux-3-finger-drag/pull/21))

### Added

- Add hot reloading for config file, except for logging options (open to PRs for this)

### Removed

- Remove `minMotion` option, as it is useless


## 1.5.0 - 2025-08-23

### Added

- Add auto cancelation of drag end delay upon non-three-fingered gesture (issue #12)
- Add `udev` rule for restarting systemd service when new trackpad is connected to the laptop (issue #13)

### Removed

- Remove option `failFast`; operation will continue if initialization is successful, only logging errors, and not crashing

### Fixed

- Fix pointer not moving on Fedora machines running KDE (issue #9)

## 1.4.1 - 2025-07-27

### Fixed

- Fix issue (#11) where `dragEndDelay` config values were ignored

### Changed

- Change order of config options in `3fd-config.json` to alphabetical

### Removed

- Remove update script (didn't work, redundant)

## 1.4.0 - 2025-07-26

### Fixed

- Fix issue (#10) where the main loop would consume all of a CPU core while running

### Added

- Add configurable response time to config file, `responseTime`, to limit the amount of times the main loop runs per second. Default to 5ms.

### Changed

- Change in-source type for `dragEndDelay` from a `u64` to a `std::time::Duration`, deserialized into the type once. This is not to change the user interface, e.g. time-related fields in the config file are still written as simple integers.


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

