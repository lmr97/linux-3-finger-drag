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
//!  * the created devices are REAL input devices: the compositor will
//!    act on them. The test parks the cursor against the right screen
//!    edge first, and the drag scenario presses the left button for
//!    ~100ms there. Run it from a session where a stray click at the
//!    right edge of the screen is acceptable.
//!
//! Because of the second point the test is #[ignore]d by default:
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
        Ok(Reader {
            dev: EvdevHandle::new(f),
        })
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
    std::fs::write(
        cfg_dir.join("linux-3-finger-drag/3fd-config.json"),
        r#"{ "acceleration": 1.0, "dragEndDelay": 0, "logLevel": "debug" }"#,
    )
    .unwrap();

    let before = list_event_nodes();

    let child = Command::new(env!("CARGO_BIN_EXE_linux-3-finger-drag"))
        .arg("--device")
        .arg(&pad.path)
        .env("XDG_CONFIG_HOME", &cfg_dir)
        .stdout(std::process::Stdio::null())
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

    let f = |slot: i32, id: i32, x: i32, y: i32| -> Vec<(u16, u16, i32)> {
        vec![
            (EV_ABS, ABS_MT_SLOT, slot),
            (EV_ABS, ABS_MT_TRACKING_ID, id),
            (EV_ABS, ABS_MT_POSITION_X, x),
            (EV_ABS, ABS_MT_POSITION_Y, y),
        ]
    };

    // -- scenario 1: single-finger motion passes through the clone --------
    // (doubles as cursor parking: sweep hard right repeatedly so the
    // pointer ends pinned at the right screen edge before scenario 2's
    // button press)
    for id in 100..112 {
        let mut evs = f(0, id, 100, 700);
        evs.push((EV_KEY, BTN_TOUCH, 1));
        evs.push((EV_KEY, BTN_TOOL_FINGER, 1));
        pad.frame(&evs);
        for step in 1..=6 {
            std::thread::sleep(Duration::from_millis(10));
            pad.frame(&[
                (EV_ABS, ABS_MT_SLOT, 0),
                (EV_ABS, ABS_MT_POSITION_X, 100 + step * 300),
            ]);
        }
        pad.frame(&[
            (EV_ABS, ABS_MT_SLOT, 0),
            (EV_ABS, ABS_MT_TRACKING_ID, -1),
            (EV_KEY, BTN_TOUCH, 0),
            (EV_KEY, BTN_TOOL_FINGER, 0),
        ]);
        std::thread::sleep(Duration::from_millis(30));
    }
    let (seen, ok) = clone.wait_for(Duration::from_secs(2), |&(t, c, _)| {
        t == EV_ABS && c == ABS_MT_POSITION_X
    });
    assert!(ok, "single-finger motion never reached the clone: {seen:?}");
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

    // -- scenario 3: 4-finger touch passes through untouched --------------
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

    drop(pad);
    let _ = std::fs::remove_dir_all(&cfg_dir);
}
