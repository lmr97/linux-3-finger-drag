# Three-Finger Drag for Wayland/KDE (and X11)

Rest three fingers on the touchpad and move them: the window / text / icon under the cursor is dragged, exactly like macOS's "three finger drag". Lift, and the drag ends.

The program is a full **evdev multitouch proxy** (replacing the earlier libinput-gesture-listener design). The proxy exists because of KWin: KDE hardcodes desktop-switching to *both* 3- and 4-finger horizontal swipes, with no setting to disable just the 3-finger binding (#18) — so a gesture listener that merely *watches* the touchpad can never stop KWin from also acting on the same three fingers. Owning the device and deciding per-frame what the compositor gets to see is the only clean fix.

## How it works

```
 real touchpad ──(exclusive grab)──> linux-3-finger-drag
                                        │        │
                                        │        ├──> synthetic touchpad clone
                                        │        │    (byte-identical mirror of
                                        │        │     everything that is NOT a
                                        │        │     3-finger drag)
                                     gesture     │
                                     machine     └──> virtual mouse
                                                      (BTN_LEFT + motion while
                                                       a 3-finger drag is live)
```

* The real touchpad is **exclusively grabbed** for the program's whole lifetime. The compositor instead reads a **synthetic clone** that impersonates the real device's identity (name/vendor/product), so saved per-device settings (natural scrolling, tap-to-click, pointer accel…) keep applying. The clone carries a `phys` marker (`linux-3-finger-drag/proxy`) so the proxy can always tell its own clone apart from real hardware.
* A fresh touch is **withheld** from the compositor until classified: a lone finger goes live after `probeDelay` (default 15 ms — ordinary pointer motion never feels delayed), an ambiguous 2-3 finger touch waits out `entryDebounce` (default 50 ms), 4+ fingers goes live instantly. Real fingers land and lift asynchronously; judging a touch frame-by-frame (the naive approach) leaks phantom taps and misreads gestures.
* A touch that holds at **exactly 3 fingers** through the debounce becomes a drag: the compositor never learns those fingers existed (KWin can't desktop-switch on what it can't see), and finger motion drives the virtual mouse with the left button held. The drag ends when the last finger lifts — staggered liftoffs can't leak trailing 1-2 finger touches (which libinput would read as a right-click tap).
* Anything else — taps (3-finger tap still middle-clicks!), scrolls, 4-finger gestures, quick flicks — is replayed to the clone verbatim.

The classification logic lives in a pure, I/O-free state machine (`src/runtime/gesture.rs`) driven by an injected clock, with a regression test suite encoding every failure mode this project has hit live (`src/runtime/gesture/tests.rs`). The event loop is fully event-driven (epoll on the device fd + exact decision deadlines): idle CPU is zero, and no polling interval sits between your fingers and a decision.

## Requirements

* Rust toolchain (build-time only — there are **no** C library dependencies; the program speaks evdev/uinput directly)
* `uinput` kernel module
* read access to `/dev/input` (user in the `input` group) and write access to `/dev/uinput` (udev rule included)
* a systemd user session for the provided unit (any init works if you start the binary yourself)

Wayland and X11 are both fine; the proxy operates below the display server. Developed and tuned on a MacBookPro11,3 (bcm5974 touchpad) running CachyOS + KDE Plasma Wayland.

## Installation

Automated (installs udev rule, adds you to `input`, builds, installs binary + config + systemd user unit):

```bash
sudo ./install.sh
```

Manual:

```bash
# 1. permissions
sudo cp 60-uinput.rules /etc/udev/rules.d/
sudo gpasswd --add $USER input
echo uinput | sudo tee /etc/modules-load.d/uinput.conf
sudo modprobe uinput
# log out & in (or reboot) so the group change applies

# 2. build & install
cargo build --release
sudo cp target/release/linux-3-finger-drag /usr/bin/

# 3. config + service
mkdir -p ~/.config/linux-3-finger-drag
cp 3fd-config.json ~/.config/linux-3-finger-drag/
mkdir -p ~/.config/systemd/user
cp three-finger-drag.service ~/.config/systemd/user/
systemctl --user enable --now three-finger-drag.service
```

Test in the foreground first if you're changing code: `./target/release/linux-3-finger-drag` (Ctrl-C to quit — the touchpad returns to normal the moment the process exits).

### CLI

```
linux-3-finger-drag [--device /dev/input/eventN]
```

`--device` skips touchpad auto-discovery and proxies the given device. Used by the integration test harness; also handy on machines with more than one touchpad (auto-discovery proxies the first one found).

## Configuration

`~/.config/linux-3-finger-drag/3fd-config.json`, hot-reloaded on change (log settings excepted — those need a restart). All fields optional:

| field | default | meaning |
|---|---|---|
| `acceleration` | `1.0` | drag speed multiplier (`> 1` faster, `< 1` slower) |
| `dragEndDelay` | `0` | drag-lock, in ms: after lifting, the button stays held this long, and a new 3-finger touch inside the window **continues the same drag**. Any other touch releases the button *before* it is relayed, so post-drag pointer motion can never smear the held button around. `0` disables. |
| `entryDebounce` | `50` | length of time (in milliseconds) an ambiguous (2-3 finger, possibly still growing) fresh touch is withheld before committing: drag, or replay to the compositor |
| `probeDelay` | `15` | Length of time (in milliseconds) a so-far-lone finger is withheld (just long enough to catch a 2nd/3rd finger landing a beat behind the 1st) |
| `pressGrace` | `75` | Length of time (in milliseconds) a committed drag defers its button press while the fingers haven't moved. Lets a 4th finger that lands *after* the entry window (fast, sloppy 4-finger swipes stagger hard) abort the misclassified drag with no phantom click — the touch is handed to the compositor mid-gesture instead |
| `logFile` | `"stdout"` | log destination (`"stdout"` or a file path). It works best with absolute paths, because `~` or other shell variables are not expanded, but relative filepaths work. Will create the file if it does not exist. |
| `logLevel` | `"info"` | One of the following values: `off` / `error` / `warn` / `info` / `debug` / `trace`. For more info on what these levels are intended to capture, see the documentation for [the `enum` to which these values correspond](https://docs.rs/log/0.4.6/log/enum.Level.html). Note that `debug` and `trace` levels generate logs extremely rapidly, which both baloons the log file size (even after short periods of use), and consumes spikes CPU usage on fast, long gestures. |


## Testing

```bash
cargo test                                    # gesture regression suite (pure, instant)
cargo test --test integration -- --ignored    # software-in-the-loop, see below
```

The integration test creates a **fake touchpad** via uinput, points the real binary at it (`--device`), injects scripted multi-finger sequences, and asserts on what actually comes out of the clone and the virtual mouse. It exercises the entire evdev plumbing without touching your real touchpad — but its output devices are real input devices, so the compositor will act on them (the test parks the cursor at the right screen edge and produces one brief left-click there). Run it from a session where that's acceptable.

## Troubleshooting

If the fixes here and in the Issues section of the repo don't address your issue, please open a new issue!

* **Touchpad dead while the program runs?** The proxy has the device grabbed but something is failing after that. Check `journalctl --user -u three-finger-drag.service -e` — and note the touchpad always returns the instant the process exits.
* **"You are not yet allowed to write to /dev/uinput"** — udev rule not applied, or you haven't logged out and back in since being added to the `input` group.
* **Drag feels too slow/fast** — tune `acceleration`; it multiplies a baseline of 12 px per mm of finger travel.
* **KDE gestures still firing on 3 fingers?** Then the compositor is reading the *real* touchpad, not the clone — the service probably isn't running.
* **Two touchpads?** Auto-discovery takes the first device discovered. Pin one explicitly with `--device`.
* **``error: linking with `cc` failed: exit status: 1`` during compilation** This error arises when some underlying system library can't be found. Cargo produces several "notes" in addition to the error message; if the final note includes some mention of `-linput`, then you need to install the development library for `libinput`, probably called `libinput-dev` or `libinput-devel` in your distro's package repo. If that doesn't work, may be missing the basic C/C++ developer tools, which are needed to build this program. These are typically bundled with your distro's "base development" or "build essentials" package.

## License

MIT (see `LICENSE`).
