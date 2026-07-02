// Proxies the real touchpad's raw multitouch stream to a synthetic clone
// device, 1:1, EXCEPT while exactly 3 fingers are touching: that window is
// withheld from the clone entirely (never begun, never left half-open) and
// instead drives the existing 3-finger-drag emulation on the virtual mouse.
//
// This exists because a naive mid-gesture EVIOCGRAB on the real device (this
// project's first approach) leaves the compositor's own touch-state tracking
// permanently corrupted: the compositor never gets to see the closing
// lift-off frames for the fingers grabbed away mid-gesture, so it's stuck
// believing they're still down. By instead owning the real device for the
// program's entire lifetime and re-emitting a clean copy of it, we control
// every frame the compositor ever sees: either an accurate mirror of the
// real pad, or an explicit "nothing is touching" state we emit ourselves --
// never a silent gap.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;

use nix::libc::O_NONBLOCK;
use tracing::{debug, info, trace, warn};

use input_linux::{sys, AbsoluteAxis, AbsoluteInfoSetup, EvdevHandle, EventKind, InputId, UInputHandle};

use super::event_handler::{GestureTranslator, GtError};

const EV_SYN: u16 = 0x00;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
const SYN_DROPPED: u16 = 0x03;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;

const MAX_SLOTS: usize = 16;
const READ_BATCH: usize = 64;

// Empirically-reasonable px-per-mm scale for turning the real finger delta
// into cursor movement; this combines with the existing `acceleration`
// config knob, so it doesn't need to be exact -- just a sane starting point.
const PX_PER_MM: f64 = 4.0;

#[derive(Clone, Copy)]
struct Slot {
    tracking_id: i32,
    x: i32,
    y: i32,
}

impl Default for Slot {
    fn default() -> Self {
        Slot { tracking_id: -1, x: 0, y: 0 }
    }
}

fn zero_event() -> sys::input_event {
    unsafe { std::mem::zeroed() }
}

fn abs_event(code: u16, value: i32) -> sys::input_event {
    let mut ev = zero_event();
    ev.type_ = EV_ABS;
    ev.code = code;
    ev.value = value;
    ev
}

fn syn_report() -> sys::input_event {
    let mut ev = zero_event();
    ev.type_ = EV_SYN;
    ev.code = SYN_REPORT;
    ev.value = 0;
    ev
}

pub struct MtProxy {
    real: EvdevHandle<File>,
    synth: UInputHandle<File>,
    x_res: f64,
    y_res: f64,
    slots: [Slot; MAX_SLOTS],
    current_slot: usize,
    relayed_active: [bool; MAX_SLOTS],
    suppressing: bool,
    drag_ref_slot: Option<usize>,
    drag_last_pos: Option<(i32, i32)>,
    frame: Vec<sys::input_event>,
    read_buf: [sys::input_event; READ_BATCH],
}

impl MtProxy {
    /// Opens the real touchpad at `path`, grabs it exclusively for the rest
    /// of the program's life, and creates a synthetic clone with the same
    /// multitouch capabilities for the compositor to read instead.
    pub fn new(path: &str) -> io::Result<Self> {
        let real_file = OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(path)?;
        let real = EvdevHandle::new(real_file);

        // held for the entire program lifetime -- see module doc for why
        // this must never be released mid-gesture.
        real.grab(true)?;
        info!("Exclusively grabbed the real trackpad at {}.", path);

        let synth = Self::clone_device(&real)?;

        let x_res = real.absolute_info(AbsoluteAxis::MultitouchPositionX)
            .map(|i| i.resolution.max(1) as f64)
            .unwrap_or(1.0);
        let y_res = real.absolute_info(AbsoluteAxis::MultitouchPositionY)
            .map(|i| i.resolution.max(1) as f64)
            .unwrap_or(1.0);

        Ok(MtProxy {
            real,
            synth,
            x_res,
            y_res,
            slots: [Slot::default(); MAX_SLOTS],
            current_slot: 0,
            relayed_active: [false; MAX_SLOTS],
            suppressing: false,
            drag_ref_slot: None,
            drag_last_pos: None,
            frame: Vec::with_capacity(READ_BATCH),
            read_buf: [zero_event(); READ_BATCH],
        })
    }

    /// Builds a synthetic uinput device with the same EV_KEY/EV_ABS/
    /// INPUT_PROP capabilities as the real device, so the compositor's own
    /// libinput sees something functionally identical to the real hardware.
    fn clone_device(real: &EvdevHandle<File>) -> io::Result<UInputHandle<File>> {
        let uinput_file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open("/dev/uinput")?;
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

        let input_id = InputId {
            bustype: input_linux::sys::BUS_USB,
            vendor: 0x1234,
            product: 0x5679, // distinct from the drag-emulation mouse's 0x5678
            version: 0,
        };
        let device_name = b"Virtual touchpad (proxied by linux-3-finger-drag)";
        synth.create(&input_id, device_name, 0, &abs_setups)?;
        debug!("Synthetic touchpad clone created.");

        std::thread::sleep(std::time::Duration::from_millis(500));

        Ok(synth)
    }

    /// One non-blocking pass: drains whatever raw events are currently
    /// available, processing them frame-by-frame (a frame being everything
    /// since the last SYN_REPORT). Safe to call frequently in a poll loop.
    pub async fn poll(&mut self, translator: &mut GestureTranslator) -> Result<(), GtError> {
        loop {
            let n = match self.real.read(&mut self.read_buf) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(GtError::from(e)),
            };

            if n == 0 {
                return Ok(());
            }

            for i in 0..n {
                let ev = self.read_buf[i];

                if ev.type_ == EV_SYN && ev.code == SYN_DROPPED {
                    warn!("Kernel reported dropped events; resyncing slot state.");
                    self.frame.clear();
                    self.resync()?;
                    continue;
                }

                if ev.type_ == EV_SYN && ev.code == SYN_REPORT {
                    self.frame.push(ev);
                    self.handle_frame(translator).await?;
                    self.frame.clear();
                    continue;
                }

                if ev.type_ == EV_ABS && ev.code == ABS_MT_SLOT {
                    self.current_slot = (ev.value as usize).min(MAX_SLOTS - 1);
                } else if ev.type_ == EV_ABS && ev.code == ABS_MT_TRACKING_ID {
                    self.slots[self.current_slot].tracking_id = ev.value;
                } else if ev.type_ == EV_ABS && ev.code == ABS_MT_POSITION_X {
                    self.slots[self.current_slot].x = ev.value;
                } else if ev.type_ == EV_ABS && ev.code == ABS_MT_POSITION_Y {
                    self.slots[self.current_slot].y = ev.value;
                }

                self.frame.push(ev);
            }
        }
    }

    /// Re-derives slot state directly from the kernel (EVIOCGMTSLOTS)
    /// instead of trusting the incremental event history, used after a
    /// SYN_DROPPED. Forces a full resync dump to the synthetic device
    /// afterward, same as any other suppress/passthrough transition.
    fn resync(&mut self) -> Result<(), GtError> {
        let mut ids = vec![0i32; MAX_SLOTS];
        self.real.multi_touch_slots(AbsoluteAxis::MultitouchTrackingId, &mut ids)?;

        let mut xs = vec![0i32; MAX_SLOTS];
        self.real.multi_touch_slots(AbsoluteAxis::MultitouchPositionX, &mut xs)?;

        let mut ys = vec![0i32; MAX_SLOTS];
        self.real.multi_touch_slots(AbsoluteAxis::MultitouchPositionY, &mut ys)?;

        for slot in 0..MAX_SLOTS {
            self.slots[slot] = Slot { tracking_id: ids[slot], x: xs[slot], y: ys[slot] };
        }

        // don't decide suppress/passthrough here; the next real frame will
        // trigger handle_frame() and pick correctly based on active_count
        Ok(())
    }

    fn active_slots(&self) -> Vec<usize> {
        (0..MAX_SLOTS).filter(|&s| self.slots[s].tracking_id >= 0).collect()
    }

    async fn handle_frame(&mut self, translator: &mut GestureTranslator) -> Result<(), GtError> {
        let active = self.active_slots();

        if active.len() == 3 {
            if !self.suppressing {
                self.enter_suppress()?;
                translator.mouse_down().await?;
            }
            self.drive_drag(&active, translator).await?;
            // frame intentionally not relayed
            return Ok(());
        }

        if self.suppressing {
            self.suppressing = false;
            self.drag_ref_slot = None;
            self.drag_last_pos = None;
            translator.handle_mouse_up().await?;
            self.resync_synth_to_real()?;
            // the resync dump above already reflects this frame's true
            // state, so the raw frame itself doesn't also need relaying
            return Ok(());
        }

        self.relay_frame()
    }

    fn enter_suppress(&mut self) -> Result<(), GtError> {
        self.suppressing = true;

        let mut release_frame = Vec::new();
        for slot in 0..MAX_SLOTS {
            if self.relayed_active[slot] {
                release_frame.push(abs_event(ABS_MT_SLOT, slot as i32));
                release_frame.push(abs_event(ABS_MT_TRACKING_ID, -1));
                self.relayed_active[slot] = false;
            }
        }
        if !release_frame.is_empty() {
            release_frame.push(syn_report());
            self.synth.write(&release_frame)?;
            trace!("Released all relayed slots to the synthetic device before suppressing.");
        }
        Ok(())
    }

    async fn drive_drag(&mut self, active: &[usize], translator: &mut GestureTranslator) -> Result<(), GtError> {
        let reference = match self.drag_ref_slot {
            Some(s) if active.contains(&s) => s,
            _ => {
                // first frame of the gesture, or our previous reference
                // finger lifted and a different one took its place:
                // re-baseline without applying a delta this frame.
                let s = active[0];
                self.drag_ref_slot = Some(s);
                self.drag_last_pos = Some((self.slots[s].x, self.slots[s].y));
                return Ok(());
            }
        };

        let (x, y) = (self.slots[reference].x, self.slots[reference].y);
        if let Some((lx, ly)) = self.drag_last_pos {
            let dx_mm = (x - lx) as f64 / self.x_res;
            let dy_mm = (y - ly) as f64 / self.y_res;
            translator.update_cursor_position(dx_mm * PX_PER_MM, dy_mm * PX_PER_MM).await?;
        }
        self.drag_last_pos = Some((x, y));
        Ok(())
    }

    /// Dumps the current real slot state to the synthetic device as a
    /// coherent, self-contained frame -- used when leaving suppression, so
    /// the compositor gets an accurate picture regardless of which fields
    /// happened to change in the triggering frame.
    fn resync_synth_to_real(&mut self) -> Result<(), GtError> {
        let mut dump = Vec::new();
        for slot in 0..MAX_SLOTS {
            let s = self.slots[slot];
            let active = s.tracking_id >= 0;
            if active || self.relayed_active[slot] {
                dump.push(abs_event(ABS_MT_SLOT, slot as i32));
                dump.push(abs_event(ABS_MT_TRACKING_ID, s.tracking_id));
                if active {
                    dump.push(abs_event(ABS_MT_POSITION_X, s.x));
                    dump.push(abs_event(ABS_MT_POSITION_Y, s.y));
                }
                self.relayed_active[slot] = active;
            }
        }
        if !dump.is_empty() {
            dump.push(syn_report());
            self.synth.write(&dump)?;
            trace!("Resynced synthetic device to current real slot state.");
        }
        Ok(())
    }

    fn relay_frame(&mut self) -> Result<(), GtError> {
        if self.frame.is_empty() {
            return Ok(());
        }
        for slot in self.active_slots() {
            self.relayed_active[slot] = true;
        }
        self.synth.write(&self.frame)?;
        Ok(())
    }
}
