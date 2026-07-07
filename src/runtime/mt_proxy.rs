//! The I/O shell around the gesture state machine.
//!
//! Owns the real touchpad (exclusively grabbed for the program's whole
//! life) and a synthetic uinput clone of it. Raw events are read here,
//! split into frames, and fed to [`GestureMachine`]; the machine's
//! [`Output`]s are applied back to the clone and the virtual mouse.
//! All *decisions* live in `gesture.rs` -- this file only moves bytes.
//!
//! Why a lifetime-long grab + clone (vs. grabbing mid-gesture, this
//! project's first approach): a mid-gesture EVIOCGRAB leaves the
//! compositor's touch tracking permanently corrupted -- it never sees
//! the closing lift-off frames for fingers grabbed away mid-flight, so
//! it's stuck believing they're still down. By owning the real device
//! outright and re-emitting a clean copy, we control every frame the
//! compositor ever sees: either an accurate mirror of the real pad, or
//! an explicit "nothing is touching" state -- never a silent gap.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::time::Instant;

use libc::O_NONBLOCK;
use tracing::{debug, info, warn};

use input_linux::{sys, AbsoluteAxis, AbsoluteInfoSetup, EvdevHandle, EventKind, UInputHandle};

use super::gesture::{Ev, GestureMachine, Output, EV_SYN, MAX_SLOTS, SYN_DROPPED, SYN_REPORT};
use super::virtual_trackpad::VirtualTrackpad;

const READ_BATCH: usize = 64;

/// The `phys` marker stamped on our synthetic clone so device discovery
/// can never mistake our own clone for a real touchpad (it impersonates
/// the real device's name, vendor and capabilities *exactly*, so this
/// marker is the only reliable way to tell them apart -- which matters
/// when re-discovering after a hotplug).
pub const CLONE_PHYS_MARKER: &str = "linux-3-finger-drag/proxy";

fn zero_event() -> sys::input_event {
    unsafe { std::mem::zeroed() }
}

/// The kernel's legacy `UI_SET_PHYS` is `_IOW('U', 108, char*)` -- ioctl
/// size = sizeof(char*), argument = pointer to a NUL-terminated string.
/// (input-linux 0.7's binding encodes size 1, which the kernel rejects
/// with EINVAL, so we issue the ioctl ourselves.)
fn ui_set_phys(fd: RawFd, phys: &std::ffi::CStr) -> io::Result<()> {
    const IOC_WRITE: libc::c_ulong = 1;
    let cmd: libc::c_ulong = (IOC_WRITE << 30)
        | ((std::mem::size_of::<*const libc::c_char>() as libc::c_ulong) << 16)
        | ((b'U' as libc::c_ulong) << 8)
        | 108;
    let rc = unsafe { libc::ioctl(fd, cmd as _, phys.as_ptr()) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn to_raw(ev: &Ev) -> sys::input_event {
    let mut raw = zero_event();
    raw.type_ = ev.type_;
    raw.code = ev.code;
    raw.value = ev.value;
    raw
}

pub struct MtProxy {
    real: EvdevHandle<File>,
    synth: UInputHandle<File>,
    raw_fd: RawFd,
    x_res: f64,
    y_res: f64,
    slot_count: usize,
    frame: Vec<Ev>,
    /// True between a SYN_DROPPED and the SYN_REPORT that closes it:
    /// per the evdev protocol, everything in that window is garbage and
    /// must be discarded, with state re-read from the kernel afterward.
    dropping: bool,
    read_buf: [sys::input_event; READ_BATCH],
}

impl AsRawFd for MtProxy {
    fn as_raw_fd(&self) -> RawFd {
        self.raw_fd
    }
}

impl MtProxy {
    /// Opens the real touchpad at `path`, grabs it exclusively for the
    /// rest of the program's life, and creates a synthetic clone with
    /// identical capabilities for the compositor to read instead.
    pub fn new(path: &str) -> io::Result<Self> {
        let real_file = OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(path)?;
        let raw_fd = real_file.as_raw_fd();
        let real = EvdevHandle::new(real_file);

        // held for the entire program lifetime -- see module doc for why
        // this must never be released mid-gesture
        real.grab(true)?;
        info!("Exclusively grabbed the real trackpad at {}.", path);

        let synth = Self::clone_device(&real)?;

        // Units-per-mm, from the device's reported resolution. Some
        // touchpads (various Synaptics/Elan units) report resolution 0;
        // treating that as 1 unit/mm would make drags 10-40x too fast,
        // so fall back to estimating from the axis range against a
        // typical pad size (~100mm x 70mm). Imperfect, but lands within
        // a factor of ~2 -- the `acceleration` knob covers the rest.
        let axis_res = |axis: AbsoluteAxis, assumed_mm: f64| -> f64 {
            match real.absolute_info(axis) {
                Ok(info) if info.resolution > 0 => info.resolution as f64,
                Ok(info) if info.maximum > info.minimum => {
                    let est = (info.maximum - info.minimum) as f64 / assumed_mm;
                    warn!(
                        "Touchpad reports no resolution for {:?}; estimating {:.1} units/mm \
                        from its axis range (tune drag speed with `acceleration` if needed).",
                        axis, est
                    );
                    est
                }
                _ => 1.0,
            }
        };
        let x_res = axis_res(AbsoluteAxis::MultitouchPositionX, 100.0);
        let y_res = axis_res(AbsoluteAxis::MultitouchPositionY, 70.0);
        // The device's real slot range: snapshot ioctls sized past it
        // return zeroed entries whose tracking_id 0 reads as "finger
        // down" -- the phantom-touch bug. Ask the device, don't assume.
        let slot_count = real
            .absolute_info(AbsoluteAxis::MultitouchSlot)
            .map(|i| (i.maximum as usize + 1).clamp(1, MAX_SLOTS))
            .unwrap_or(MAX_SLOTS);

        Ok(MtProxy {
            real,
            synth,
            raw_fd,
            x_res,
            y_res,
            slot_count,
            frame: Vec::with_capacity(READ_BATCH),
            dropping: false,
            read_buf: [zero_event(); READ_BATCH],
        })
    }

    pub fn x_res(&self) -> f64 {
        self.x_res
    }
    pub fn y_res(&self) -> f64 {
        self.y_res
    }
    pub fn slot_count(&self) -> usize {
        self.slot_count
    }

    /// Builds a synthetic uinput device with the same EV_KEY/EV_ABS/
    /// INPUT_PROP capabilities as the real device, so the compositor's
    /// libinput sees something functionally identical to the hardware.
    fn clone_device(real: &EvdevHandle<File>) -> io::Result<UInputHandle<File>> {
        let uinput_file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open("/dev/uinput")?;
        let uinput_fd = uinput_file.as_raw_fd();
        let synth = UInputHandle::new(uinput_file);

        synth.set_evbit(EventKind::Key)?;
        synth.set_evbit(EventKind::Absolute)?;

        for key in real.key_bits()?.iter() {
            synth.set_keybit(key)?;
        }

        let mut abs_setups = Vec::new();
        for axis in real.absolute_bits()?.iter() {
            synth.set_absbit(axis)?;
            let info = real.absolute_info(axis)?;
            abs_setups.push(AbsoluteInfoSetup { axis, info });
        }

        for prop in real.device_properties()?.iter() {
            synth.set_propbit(prop)?;
        }

        // Impersonate the real device's identity (vendor/product/name),
        // not just its capabilities. KDE keys its per-device libinput
        // settings (natural scroll, accel profile, tap-to-click...) in
        // kcminputrc by exactly this triple -- a clone with a made-up
        // identity is "new" to KDE and silently falls back to defaults,
        // which is what caused scrolling to come back reversed when this
        // proxy first replaced the real device as KWin's input source.
        // Matching identity means the user's saved preferences apply
        // automatically, with nothing to keep in sync.
        let real_id = real.device_id()?;
        let mut real_name = real.device_name()?;
        while real_name.last() == Some(&0) {
            real_name.pop();
        }
        // ...but stamp our marker into `phys` (which KDE ignores) so
        // device discovery can always tell the clone from the original.
        let phys = std::ffi::CString::new(CLONE_PHYS_MARKER).expect("no NUL in marker");
        ui_set_phys(uinput_fd, &phys)
            .map_err(|e| io::Error::new(e.kind(), format!("set_phys: {e}")))?;
        synth
            .create(&real_id, &real_name, 0, &abs_setups)
            .map_err(|e| io::Error::new(e.kind(), format!("uinput create: {e}")))?;
        debug!(
            "Synthetic touchpad clone created, impersonating \"{}\".",
            String::from_utf8_lossy(&real_name)
        );

        // give udev/the compositor a beat to pick the new device up
        // before events start flowing through it
        std::thread::sleep(std::time::Duration::from_millis(500));

        Ok(synth)
    }

    /// Drains every event currently readable, feeding complete frames to
    /// the machine and applying its outputs. Returns when the fd would
    /// block. An `ENODEV` error means the device was unplugged /
    /// re-enumerated; the caller handles re-discovery.
    pub fn drain(
        &mut self,
        machine: &mut GestureMachine,
        vtp: &mut VirtualTrackpad,
    ) -> io::Result<()> {
        loop {
            let n = match self.real.read(&mut self.read_buf) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            };
            if n == 0 {
                return Ok(());
            }

            for i in 0..n {
                let raw = self.read_buf[i];
                if self.dropping {
                    // evdev protocol: after SYN_DROPPED, everything up to
                    // and including the next SYN_REPORT is unreliable --
                    // discard it, then re-read authoritative state.
                    if raw.type_ == EV_SYN && raw.code == SYN_REPORT {
                        self.dropping = false;
                        let snapshot = self.slot_snapshot()?;
                        let outs = machine.on_resync(&snapshot, Instant::now());
                        self.apply(&outs, vtp)?;
                    }
                    continue;
                }

                if raw.type_ == EV_SYN && raw.code == SYN_DROPPED {
                    warn!("Kernel reported dropped events; resyncing slot state.");
                    self.frame.clear();
                    self.dropping = true;
                    continue;
                }

                self.frame.push(Ev::new(raw.type_, raw.code, raw.value));

                if raw.type_ == EV_SYN && raw.code == SYN_REPORT {
                    let outs = machine.on_frame(&self.frame, Instant::now());
                    self.frame.clear();
                    self.apply(&outs, vtp)?;
                }
            }
        }
    }

    /// Applies the machine's outputs to the actual devices, in order.
    pub fn apply(&mut self, outputs: &[Output], vtp: &mut VirtualTrackpad) -> io::Result<()> {
        for output in outputs {
            match output {
                Output::EmitSynth(evs) => {
                    let raw: Vec<sys::input_event> = evs.iter().map(to_raw).collect();
                    self.synth.write(&raw)?;
                }
                Output::MouseDown => vtp.mouse_down()?,
                Output::MouseUp => vtp.mouse_up()?,
                Output::MouseMove { dx, dy } => vtp.mouse_move_relative(*dx, *dy)?,
            }
        }
        Ok(())
    }

    /// Authoritative per-slot state straight from the kernel
    /// (EVIOCGMTSLOTS), sized to the device's true slot range.
    fn slot_snapshot(&self) -> io::Result<Vec<(i32, i32, i32)>> {
        let mut ids = vec![0i32; self.slot_count];
        self.real
            .multi_touch_slots(AbsoluteAxis::MultitouchTrackingId, &mut ids)?;
        let mut xs = vec![0i32; self.slot_count];
        self.real
            .multi_touch_slots(AbsoluteAxis::MultitouchPositionX, &mut xs)?;
        let mut ys = vec![0i32; self.slot_count];
        self.real
            .multi_touch_slots(AbsoluteAxis::MultitouchPositionY, &mut ys)?;

        Ok((0..self.slot_count)
            .map(|s| (ids[s], xs[s], ys[s]))
            .collect())
    }

    /// Destroys the synthetic clone (the grab on the real device is
    /// released automatically when the fd closes on drop).
    pub fn destruct(self) -> io::Result<()> {
        self.synth.dev_destroy()
    }
}
