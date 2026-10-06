//! Software-in-the-loop integration test.
//!
//! Creates a *fake* multitouch touchpad via uinput, points the real
//! compiled binary at it (`--device`), injects scripted multi-finger
//! sequences, and asserts on what actually comes out of the proxy's two
//! output devices (the synthetic touchpad clone and the virtual mouse).
//! End-to-end coverage of the evdev plumbing with zero involvement of
//! the machine's real touchpad.
//!
//! Requirements & caveats:
//!  * needs write access to /dev/uinput (input group + uaccess rule) --
//!    the same permissions the program itself needs;
//!  * the created devices are REAL input devices. The test grabs both
//!    proxy outputs before injecting anything, isolating the compositor
//!    while the assertions inspect their evdev streams.
//!
//! Because it creates live input devices the test is #[ignore]d by default:
//!
//!     cargo test --test integration -- --ignored --test-threads=1

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use input_linux::{
    sys, AbsoluteAxis, AbsoluteInfo, AbsoluteInfoSetup, EvdevHandle, EventKind, InputId,
    InputProperty, Key, UInputHandle,
};

const O_NONBLOCK: i32 = libc::O_NONBLOCK;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;

const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const BTN_LEFT: u16 = 0x110;
const BTN_TOUCH: u16 = 0x14a;
const BTN_TOOL_FINGER: u16 = 0x145;
const BTN_TOOL_TRIPLETAP: u16 = 0x14e;
const BTN_TOOL_QUADTAP: u16 = 0x14f;

fn raw(type_: u16, code: u16, value: i32) -> sys::input_event {
    let mut ev: sys::input_event = unsafe { std::mem::zeroed() };
    ev.type_ = type_;
    ev.code = code;
    ev.value = value;
    ev
}

/// A fake bcm5974-ish touchpad we fully control.
struct FakePad {
    handle: UInputHandle<File>,
    path: PathBuf,
}

impl FakePad {
    fn create() -> std::io::Result<Self> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open("/dev/uinput")?;
        let h = UInputHandle::new(f);

        h.set_evbit(EventKind::Key)?;
        for key in [
            Key::ButtonTouch,
            Key::ButtonToolFinger,
            Key::ButtonToolDoubleTap,
            Key::ButtonToolTripleTap,
            Key::ButtonToolQuadtap,
            Key::ButtonLeft,
        ] {
            h.set_keybit(key)?;
        }

        h.set_evbit(EventKind::Absolute)?;
        let abs = |axis, min, max, res| AbsoluteInfoSetup {
            axis,
            info: AbsoluteInfo {
                value: 0,
                minimum: min,
                maximum: max,
                fuzz: 0,
                flat: 0,
                resolution: res,
            },
        };
        let setups = [
            abs(AbsoluteAxis::X, 0, 2000, 10),
            abs(AbsoluteAxis::Y, 0, 1400, 10),
            abs(AbsoluteAxis::MultitouchSlot, 0, 15, 0),
            abs(AbsoluteAxis::MultitouchTrackingId, 0, 65535, 0),
            abs(AbsoluteAxis::MultitouchPositionX, 0, 2000, 10),
            abs(AbsoluteAxis::MultitouchPositionY, 0, 1400, 10),
        ];
        for s in &setups {
            h.set_absbit(s.axis)?;
        }

        h.set_propbit(InputProperty::Pointer)?;
        h.set_propbit(InputProperty::ButtonPad)?;

        let id = InputId {
            bustype: sys::BUS_USB,
            vendor: 0x3f3f,
            product: 0x0001,
            version: 1,
        };
        h.create(&id, b"3fd-integration-fake-touchpad", 0, &setups)?;
        let path = h.evdev_path()?;
        // let udev settle before anything opens it
        std::thread::sleep(Duration::from_millis(300));
        Ok(FakePad { handle: h, path })
    }

    /// Injects one frame (SYN_REPORT appended).
    fn frame(&self, events: &[(u16, u16, i32)]) {
        let mut evs: Vec<sys::input_event> = events.iter().map(|&(t, c, v)| raw(t, c, v)).collect();
        evs.push(raw(EV_SYN, SYN_REPORT, 0));
        self.handle.write(&evs).expect("inject frame");
    }
}

/// Non-blocking reader over an output device of the proxy.
struct Reader {
    dev: EvdevHandle<File>,
}

impl Reader {
    fn open(path: &PathBuf) -> std::io::Result<Self> {
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(path)?;
        let dev = EvdevHandle::new(f);
        // Grab the synthetic outputs before any test input is injected. This
        // keeps a desktop compositor from consuming the events we assert on.
        dev.grab(true)?;
        Ok(Reader { dev })
    }

    /// Reads the kernel's authoritative multitouch state (EVIOCGMTSLOTS).
    fn mt_slots(&self, slot_count: usize) -> std::io::Result<Vec<(i32, i32, i32)>> {
        let mut ids = vec![0; slot_count];
        self.dev
            .multi_touch_slots(AbsoluteAxis::MultitouchTrackingId, &mut ids)?;
        let mut xs = vec![0; slot_count];
        self.dev
            .multi_touch_slots(AbsoluteAxis::MultitouchPositionX, &mut xs)?;
        let mut ys = vec![0; slot_count];
        self.dev
            .multi_touch_slots(AbsoluteAxis::MultitouchPositionY, &mut ys)?;
        Ok((0..slot_count)
            .map(|slot| (ids[slot], xs[slot], ys[slot]))
            .collect())
    }

    fn key_pressed(&self, code: u16) -> std::io::Result<bool> {
        let mut state = vec![0u8; (sys::KEY_MAX as usize + 8) / 8];
        self.dev.key_state(&mut state)?;
        let code = code as usize;
        Ok(state[code / 8] & (1 << (code % 8)) != 0)
    }

    fn wait_for_keys_clear(&self, timeout: Duration, codes: &[u16]) {
        let deadline = Instant::now() + timeout;
        loop {
            let clear = codes
                .iter()
                .all(|&code| !self.key_pressed(code).expect("read output key state"));
            if clear {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "output key state remained pressed: {codes:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Polls EVIOCGMTSLOTS until the output device reaches the expected state.
    fn wait_for_mt_slots(
        &self,
        timeout: Duration,
        pred: impl Fn(&[(i32, i32, i32)]) -> bool,
    ) -> Vec<(i32, i32, i32)> {
        let deadline = Instant::now() + timeout;
        loop {
            let slots = self.mt_slots(16).expect("read output multitouch slots");
            if pred(&slots) {
                return slots;
            }
            assert!(
                Instant::now() < deadline,
                "output multitouch state did not reach the expected value: {slots:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn drain(&self) -> Vec<(u16, u16, i32)> {
        let mut buf = [raw(0, 0, 0); 64];
        let mut all = Vec::new();
        loop {
            match self.dev.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => all.extend(buf[..n].iter().map(|e| (e.type_, e.code, e.value))),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => panic!("read output device: {e}"),
            }
        }
        all
    }

    /// Drains until `pred` matches an event or the timeout expires.
    /// Returns everything drained.
    fn wait_for(
        &self,
        timeout: Duration,
        pred: impl Fn(&(u16, u16, i32)) -> bool,
    ) -> (Vec<(u16, u16, i32)>, bool) {
        let deadline = Instant::now() + timeout;
        let mut seen = Vec::new();
        loop {
            seen.extend(self.drain());
            if seen.iter().any(&pred) {
                return (seen, true);
            }
            if Instant::now() >= deadline {
                return (seen, false);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn list_event_nodes() -> HashSet<PathBuf> {
    std::fs::read_dir("/dev/input")
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with("event"))
                .unwrap_or(false)
        })
        .collect()
}

fn device_name(path: &PathBuf) -> Option<String> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(O_NONBLOCK)
        .open(path)
        .ok()?;
    let h = EvdevHandle::new(f);
    let name = h.device_name().ok()?;
    Some(
        String::from_utf8_lossy(&name)
            .trim_end_matches('\0')
            .to_string(),
    )
}

/// Kills the child on drop so a panicking assertion can't leak a
/// process that holds the fake pad grabbed.
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "creates live input devices; run manually: cargo test --test integration -- --ignored"]
fn end_to_end() {
    // -- setup ------------------------------------------------------------
    let pad = FakePad::create().expect(
        "cannot create uinput device -- are you in the 'input' group with the udev rule applied?",
    );

    let cfg_dir = std::env::temp_dir().join(format!("3fd-itest-{}", std::process::id()));
    std::fs::create_dir_all(cfg_dir.join("linux-3-finger-drag")).unwrap();
    let log_path = cfg_dir.join("proxy.log");
    std::fs::write(
        cfg_dir.join("linux-3-finger-drag/3fd-config.json"),
        r#"{ "acceleration": 1.0, "dragEndDelay": 0, "pressGrace": 1000, "logLevel": "debug" }"#,
    )
    .unwrap();

    let before = list_event_nodes();

    let child_bin = std::env::var_os("THREE_FINGER_DRAG_TEST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_linux-3-finger-drag")));
    let child = Command::new(child_bin)
        .arg("--device")
        .arg(&pad.path)
        .env("XDG_CONFIG_HOME", &cfg_dir)
        .stdout(std::process::Stdio::from(
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&log_path)
                .expect("open proxy log"),
        ))
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn proxy binary");
    let mut child = ChildGuard(child);

    // wait for the proxy's two output devices to appear
    let (clone_path, mouse_path) = {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let new: Vec<PathBuf> = list_event_nodes().difference(&before).cloned().collect();
            let clone = new
                .iter()
                .find(|p| device_name(p).as_deref() == Some("3fd-integration-fake-touchpad"))
                .cloned();
            let mouse = new
                .iter()
                .find(|p| {
                    device_name(p).as_deref()
                        == Some("Virtual trackpad (created by linux-3-finger-drag)")
                })
                .cloned();
            if let (Some(c), Some(m)) = (clone, mouse) {
                break (c, m);
            }
            assert!(
                Instant::now() < deadline,
                "proxy did not create its output devices within 10s"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let clone = Reader::open(&clone_path).expect("open clone");
    let mouse = Reader::open(&mouse_path).expect("open mouse");
    // MtProxy waits for udev during clone creation and snapshots the input
    // state at startup. Let that initialization finish before the first
    // frame so the initial snapshot cannot consume test input.
    std::thread::sleep(Duration::from_secs(1));

    let f = |slot: i32, id: i32, x: i32, y: i32| -> Vec<(u16, u16, i32)> {
        vec![
            (EV_ABS, ABS_MT_SLOT, slot),
            (EV_ABS, ABS_MT_TRACKING_ID, id),
            (EV_ABS, ABS_MT_POSITION_X, x),
            (EV_ABS, ABS_MT_POSITION_Y, y),
        ]
    };

    // -- scenario 1: single-finger motion passes through the clone --------
    let mut evs = f(0, 100, 100, 700);
    evs.push((EV_KEY, BTN_TOUCH, 1));
    evs.push((EV_KEY, BTN_TOOL_FINGER, 1));
    pad.frame(&evs);
    std::thread::sleep(Duration::from_millis(20));
    pad.frame(&[(EV_ABS, ABS_MT_SLOT, 0), (EV_ABS, ABS_MT_POSITION_X, 400)]);
    pad.frame(&[
        (EV_ABS, ABS_MT_SLOT, 0),
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
        (EV_KEY, BTN_TOUCH, 0),
        (EV_KEY, BTN_TOOL_FINGER, 0),
    ]);
    let (seen, ok) = clone.wait_for(Duration::from_secs(2), |&(t, c, _)| {
        t == EV_ABS && c == ABS_MT_POSITION_X
    });
    assert!(ok, "single-finger motion never reached the clone: {seen:?}");
    clone.wait_for_mt_slots(Duration::from_secs(2), |slots| {
        slots.iter().all(|&(id, _, _)| id < 0)
    });
    clone.wait_for_keys_clear(Duration::from_secs(2), &[BTN_TOUCH, BTN_TOOL_FINGER]);
    // wait_for above may return on the motion before the release frame is
    // consumed; drain only after the kernel state confirms the touch ended.
    clone.drain();
    let mouse_noise = mouse.drain();
    assert!(
        !mouse_noise.iter().any(|&(t, _, _)| t == EV_KEY),
        "single-finger motion must not touch the mouse button: {mouse_noise:?}"
    );

    // -- scenario 2: sustained 3-finger touch becomes a drag --------------
    let mut evs = Vec::new();
    evs.extend(f(0, 200, 800, 700));
    evs.extend(f(1, 201, 900, 700));
    evs.extend(f(2, 202, 1000, 700));
    evs.push((EV_KEY, BTN_TOUCH, 1));
    evs.push((EV_KEY, BTN_TOOL_TRIPLETAP, 1));
    pad.frame(&evs);

    // hold past the entry debounce (50ms default)
    std::thread::sleep(Duration::from_millis(90));
    // wiggle 2 units -- barely over one pixel of cursor motion
    pad.frame(&[(EV_ABS, ABS_MT_SLOT, 0), (EV_ABS, ABS_MT_POSITION_X, 802)]);

    let (_, got_down) = mouse.wait_for(Duration::from_secs(2), |&(t, c, v)| {
        t == EV_KEY && c == BTN_LEFT && v == 1
    });
    assert!(got_down, "3-finger hold never pressed the virtual button");

    let leaked = clone.drain();
    assert!(
        !leaked
            .iter()
            .any(|&(t, c, _)| t == EV_ABS && c == ABS_MT_TRACKING_ID),
        "the 3-finger touch leaked to the compositor: {leaked:?}"
    );

    // lift all three (staggered, like real fingers)
    pad.frame(&[(EV_ABS, ABS_MT_SLOT, 0), (EV_ABS, ABS_MT_TRACKING_ID, -1)]);
    std::thread::sleep(Duration::from_millis(8));
    pad.frame(&[(EV_ABS, ABS_MT_SLOT, 1), (EV_ABS, ABS_MT_TRACKING_ID, -1)]);
    std::thread::sleep(Duration::from_millis(8));
    pad.frame(&[
        (EV_ABS, ABS_MT_SLOT, 2),
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
        (EV_KEY, BTN_TOUCH, 0),
        (EV_KEY, BTN_TOOL_TRIPLETAP, 0),
    ]);

    let (_, got_up) = mouse.wait_for(Duration::from_secs(2), |&(t, c, v)| {
        t == EV_KEY && c == BTN_LEFT && v == 0
    });
    assert!(got_up, "drag never released the virtual button");
    let leaked = clone.drain();
    assert!(
        !leaked
            .iter()
            .any(|&(t, c, _)| t == EV_ABS && c == ABS_MT_TRACKING_ID),
        "the staggered 3-finger liftoff leaked to the compositor: {leaked:?}"
    );

    // -- scenario 3: an implicit slot after a suppressed drag ------------
    // The real pad's selected slot persists across SYN_REPORT. The drag
    // above ended by releasing slot 2, so this new touch deliberately omits
    // ABS_MT_SLOT and must still land in slot 2 on the clone. This exercises
    // the real kernel slot state, rather than only inspecting emitted bytes.
    clone.drain();
    mouse.drain();
    let mut evs = f(2, 900, 800, 500);
    evs.remove(0); // hardware keeps slot 2 selected
    evs.push((EV_KEY, BTN_TOUCH, 1));
    evs.push((EV_KEY, BTN_TOOL_FINGER, 1));
    pad.frame(&evs);
    let (_, got_touch) = clone.wait_for(Duration::from_secs(2), |&(t, c, v)| {
        t == EV_ABS && c == ABS_MT_TRACKING_ID && v == 900
    });
    assert!(
        got_touch,
        "implicit slot-2 touchdown never reached the clone"
    );
    let slots = clone.wait_for_mt_slots(Duration::from_secs(2), |slots| {
        slots[2] == (900, 800, 500) && slots[0].0 < 0
    });
    assert_eq!(slots[2], (900, 800, 500));
    assert!(
        slots[0].0 < 0,
        "implicit slot selection must not create a phantom slot-0 touch: {slots:?}"
    );

    // Release the same implicitly selected slot, then verify the clone has
    // no active slots left at all.
    pad.frame(&[
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
        (EV_KEY, BTN_TOUCH, 0),
        (EV_KEY, BTN_TOOL_FINGER, 0),
    ]);
    let slots = clone.wait_for_mt_slots(Duration::from_secs(2), |slots| {
        slots.iter().all(|&(id, _, _)| id < 0)
    });
    assert!(
        slots.iter().all(|&(id, _, _)| id < 0),
        "implicit slot-2 liftoff left a phantom touch: {slots:?}"
    );

    // -- scenario 4: late-fourth handoff preserves the hardware slot ------
    // A committed drag is handed back when slot 1 arrives as the fourth
    // finger. The handoff dump ends at the maximum active slot (3), while
    // the hardware remains selected on slot 1. Subsequent implicit position
    // updates must therefore continue updating slot 1, not slot 3.
    clone.drain();
    mouse.drain();
    let mut evs = Vec::new();
    evs.extend(f(0, 200, 800, 700));
    evs.extend(f(1, 201, 900, 700));
    evs.extend(f(2, 202, 1000, 700));
    evs.push((EV_KEY, BTN_TOUCH, 1));
    evs.push((EV_KEY, BTN_TOOL_TRIPLETAP, 1));
    pad.frame(&evs);
    std::thread::sleep(Duration::from_millis(90));
    // A slot-only frame makes the entry decision without moving a finger.
    pad.frame(&[(EV_ABS, ABS_MT_SLOT, 2)]);
    std::thread::sleep(Duration::from_millis(8));
    pad.frame(&[(EV_ABS, ABS_MT_SLOT, 1), (EV_ABS, ABS_MT_TRACKING_ID, -1)]);
    std::thread::sleep(Duration::from_millis(8));
    pad.frame(&[
        (EV_ABS, ABS_MT_SLOT, 3),
        (EV_ABS, ABS_MT_TRACKING_ID, 203),
        (EV_ABS, ABS_MT_POSITION_X, 1100),
        (EV_ABS, ABS_MT_POSITION_Y, 700),
    ]);
    std::thread::sleep(Duration::from_millis(8));
    // Current slot is now 1, but slot 3 is the highest active slot.
    pad.frame(&[
        (EV_ABS, ABS_MT_SLOT, 1),
        (EV_ABS, ABS_MT_TRACKING_ID, 204),
        (EV_ABS, ABS_MT_POSITION_X, 600),
        (EV_ABS, ABS_MT_POSITION_Y, 700),
    ]);
    let (_, got_handoff) = clone.wait_for(Duration::from_secs(2), |&(t, c, v)| {
        t == EV_ABS && c == ABS_MT_TRACKING_ID && v == 204
    });
    assert!(got_handoff, "late-fourth handoff never reached the clone");
    let slots = clone.wait_for_mt_slots(Duration::from_secs(2), |slots| {
        slots[1] == (204, 600, 700) && slots[3] == (203, 1100, 700)
    });
    assert_eq!(slots[1], (204, 600, 700));
    assert_eq!(slots[3], (203, 1100, 700));

    // Omit ABS_MT_SLOT on the next frame. It is still slot 1 on the input
    // device, despite the clone's handoff dump having ended on slot 3.
    pad.frame(&[(EV_ABS, ABS_MT_POSITION_X, 620)]);
    let slots = clone.wait_for_mt_slots(Duration::from_secs(2), |slots| {
        slots[1].1 == 620 && slots[3].1 == 1100
    });
    assert_eq!(slots[1].1, 620, "implicit motion must stay on slot 1");
    assert_eq!(slots[3].1, 1100, "slot 3 must not receive slot-1 motion");

    // Close the handed-off touch and verify there are no residual slots or
    // virtual-button events before the ordinary 4-finger scenario starts.
    let mut evs = Vec::new();
    for slot in 0..=3 {
        evs.push((EV_ABS, ABS_MT_SLOT, slot));
        evs.push((EV_ABS, ABS_MT_TRACKING_ID, -1));
    }
    evs.push((EV_KEY, BTN_TOUCH, 0));
    evs.push((EV_KEY, BTN_TOOL_TRIPLETAP, 0));
    pad.frame(&evs);
    let slots = clone.wait_for_mt_slots(Duration::from_secs(2), |slots| {
        slots.iter().all(|&(id, _, _)| id < 0)
    });
    assert!(
        slots.iter().all(|&(id, _, _)| id < 0),
        "late-fourth handoff left phantom touches: {slots:?}"
    );
    let mouse_noise = mouse.drain();
    assert!(
        !mouse_noise.iter().any(|&(t, _, _)| t == EV_KEY),
        "late-fourth handoff must not leave virtual-button events: {mouse_noise:?}"
    );

    // -- scenario 5: 4-finger touch passes through untouched --------------
    // (kept slow and nearly static so the compositor sees no swipe and
    // no tap; we only assert the plumbing)
    let mut evs = Vec::new();
    evs.extend(f(0, 300, 500, 600));
    evs.extend(f(1, 301, 700, 600));
    evs.extend(f(2, 302, 900, 600));
    evs.extend(f(3, 303, 1100, 600));
    evs.push((EV_KEY, BTN_TOUCH, 1));
    evs.push((EV_KEY, BTN_TOOL_QUADTAP, 1));
    pad.frame(&evs);
    std::thread::sleep(Duration::from_millis(250)); // past any tap window
    let mut evs = vec![
        (EV_ABS, ABS_MT_SLOT, 0),
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
        (EV_ABS, ABS_MT_SLOT, 1),
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
        (EV_ABS, ABS_MT_SLOT, 2),
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
        (EV_ABS, ABS_MT_SLOT, 3),
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
    ];
    evs.push((EV_KEY, BTN_TOUCH, 0));
    evs.push((EV_KEY, BTN_TOOL_QUADTAP, 0));
    pad.frame(&evs);

    let (seen, ok) = clone.wait_for(Duration::from_secs(2), |&(t, c, v)| {
        t == EV_ABS && c == ABS_MT_TRACKING_ID && v == 303
    });
    assert!(
        ok,
        "4-finger touch did not pass through to the clone: {seen:?}"
    );
    let mouse_noise = mouse.drain();
    assert!(
        !mouse_noise.iter().any(|&(t, _, _)| t == EV_KEY),
        "4-finger touch must not press the mouse button: {mouse_noise:?}"
    );

    // -- scenario 6: a real SYN_DROPPED queue overflow --------------------
    // First leave a settled touch relayed to the clone. Then stop the child
    // so its real-device queue cannot be drained, change the selected slot
    // and key state while injecting enough frames to overflow that queue,
    // and finish with every finger/key released. After SIGCONT the proxy
    // must snapshot authoritative slots and keys and explicitly clean up
    // the clone's old touch state.
    clone.drain();
    mouse.drain();
    let mut evs = f(4, 700, 700, 500);
    evs.push((EV_KEY, BTN_TOUCH, 1));
    evs.push((EV_KEY, BTN_TOOL_FINGER, 1));
    pad.frame(&evs);
    let (_, got_touch) = clone.wait_for(Duration::from_secs(2), |&(t, c, v)| {
        t == EV_ABS && c == ABS_MT_TRACKING_ID && v == 700
    });
    assert!(got_touch, "pre-overflow touch never reached the clone");
    clone.wait_for_mt_slots(Duration::from_secs(2), |slots| slots[4] == (700, 700, 500));
    clone.drain();

    unsafe {
        assert_eq!(libc::kill(child.0.id() as i32, libc::SIGSTOP), 0);
    }
    // Give SIGSTOP time to take effect before filling the real evdev queue.
    std::thread::sleep(Duration::from_millis(100));

    // Change both the selected slot and the active key state while the
    // proxy is stopped. Release the relayed slot/key state near the start,
    // then flood the queue after it so the release is in the discarded
    // window rather than being replayed as an ordinary frame. The final
    // state leaves slot 7 selected and every slot/key released.
    pad.frame(&[
        (EV_ABS, ABS_MT_SLOT, 4),
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
        (EV_ABS, ABS_MT_SLOT, 7),
        (EV_KEY, BTN_TOUCH, 0),
        (EV_KEY, BTN_TOOL_FINGER, 0),
    ]);
    for i in 0..8_192 {
        pad.frame(&[
            (EV_ABS, ABS_MT_SLOT, 7),
            (EV_ABS, ABS_MT_POSITION_X, 1200 + (i % 500)),
            (EV_ABS, ABS_MT_POSITION_Y, 800),
        ]);
    }
    pad.frame(&[(EV_ABS, ABS_MT_SLOT, 7)]);

    unsafe {
        assert_eq!(libc::kill(child.0.id() as i32, libc::SIGCONT), 0);
    }
    let slots = clone.wait_for_mt_slots(Duration::from_secs(5), |slots| {
        slots.iter().all(|&(id, _, _)| id < 0)
    });
    assert!(
        slots.iter().all(|&(id, _, _)| id < 0),
        "SYN_DROPPED recovery left phantom touches: {slots:?}"
    );
    clone.wait_for_keys_clear(
        Duration::from_secs(5),
        &[
            BTN_TOUCH,
            BTN_TOOL_FINGER,
            BTN_TOOL_TRIPLETAP,
            BTN_TOOL_QUADTAP,
        ],
    );
    mouse.wait_for_keys_clear(Duration::from_secs(5), &[BTN_LEFT]);

    // The authoritative snapshot also restores the input device's selected
    // slot. Verify that a post-recovery frame omitting ABS_MT_SLOT follows
    // slot 7, rather than the machine's pre-overflow cursor.
    let mut evs = f(7, 800, 1500, 900);
    evs.remove(0);
    evs.push((EV_KEY, BTN_TOUCH, 1));
    evs.push((EV_KEY, BTN_TOOL_FINGER, 1));
    pad.frame(&evs);
    let (_, got_touch) = clone.wait_for(Duration::from_secs(5), |&(t, c, v)| {
        t == EV_ABS && c == ABS_MT_TRACKING_ID && v == 800
    });
    assert!(
        got_touch,
        "post-resync implicit touchdown never reached clone"
    );
    let slots = clone.wait_for_mt_slots(Duration::from_secs(5), |slots| {
        slots[7] == (800, 1500, 900) && slots[4].0 < 0
    });
    assert_eq!(slots[7], (800, 1500, 900));
    assert!(
        slots[4].0 < 0,
        "post-resync touch used stale slot 4: {slots:?}"
    );
    pad.frame(&[
        (EV_ABS, ABS_MT_TRACKING_ID, -1),
        (EV_KEY, BTN_TOUCH, 0),
        (EV_KEY, BTN_TOOL_FINGER, 0),
    ]);
    clone.wait_for_mt_slots(Duration::from_secs(5), |slots| {
        slots.iter().all(|&(id, _, _)| id < 0)
    });
    clone.wait_for_keys_clear(Duration::from_secs(5), &[BTN_TOUCH, BTN_TOOL_FINGER]);

    // -- shutdown ----------------------------------------------------------
    // SIGTERM and expect a clean exit (virtual devices destroyed)
    unsafe {
        libc::kill(child.0.id() as i32, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(st) = child.0.try_wait().expect("wait child") {
            break st;
        }
        assert!(Instant::now() < deadline, "proxy did not exit on SIGTERM");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "proxy exited unclean: {status:?}");

    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log.contains("Kernel reported dropped events"),
        "overflow scenario did not observe a real SYN_DROPPED in proxy output: {log}"
    );

    drop(pad);
    let _ = std::fs::remove_dir_all(&cfg_dir);
}
