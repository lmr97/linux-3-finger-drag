//! The gesture state machine: the pure, I/O-free heart of the proxy.
//!
//! Everything time- and state-sensitive about classifying touches lives
//! here, decoupled from the devices themselves: the machine consumes raw
//! evdev frames (as plain `Ev` triples) plus an explicit `Instant`, and
//! emits explicit [`Output`] actions for the I/O shell (`mt_proxy`) to
//! carry out. No file descriptors, no wall clock, no side effects --
//! which means every historical regression this project has ever hit
//! (phantom right-clicks from staggered liftoffs, 4-finger swipes
//! misread as drags, stuck virtual buttons) is reproducible in a plain
//! `cargo test`, no fingers required. See the `tests` module at the
//! bottom: that suite is the project's institutional memory.
//!
//! Overview of the classification model (unchanged from the proven
//! design, now with the state isolated):
//!
//! * A "touch" spans from the first finger down to the last finger up,
//!   tracked as a whole -- never judged frame-by-frame, because real
//!   fingers land and lift asynchronously.
//! * A fresh touch is *buffered* (withheld from the compositor) until it
//!   is classified: a lone finger settles after a short `probe_delay`
//!   (so ordinary pointer motion never feels delayed), an ambiguous 2-3
//!   finger touch waits out `entry_debounce`, and reaching 4+ fingers
//!   settles it instantly (nothing with 4 fingers is ours).
//! * A touch that holds at exactly 3 fingers through the debounce window
//!   becomes a drag: buffered frames are discarded, the compositor never
//!   learns those fingers existed, and finger motion drives the virtual
//!   mouse instead.
//! * Once a drag starts it only ends when *every* finger lifts
//!   (hysteresis: staggered liftoff must not leak trailing 1-2 finger
//!   touches, which libinput would read as a right-click tap).
//! * A settled non-drag touch is relayed live, frame by frame, verbatim.

use std::time::{Duration, Instant};

use tracing::{debug, warn};

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_ABS: u16 = 0x03;
pub const SYN_REPORT: u16 = 0x00;
pub const SYN_DROPPED: u16 = 0x03;
pub const ABS_MT_SLOT: u16 = 0x2f;
pub const ABS_MT_TRACKING_ID: u16 = 0x39;
pub const ABS_MT_POSITION_X: u16 = 0x35;
pub const ABS_MT_POSITION_Y: u16 = 0x36;

/// Hard upper bound on tracked slots; the effective count comes from the
/// device's ABS_MT_SLOT range at construction.
pub const MAX_SLOTS: usize = 16;

/// px-per-mm scale for turning the real finger delta into cursor
/// movement; combines with the `acceleration` config knob on top.
/// 12.0 (4.0 x the initial guess) is the value confirmed to feel right
/// live on the MacBookPro11,3 pad.
pub const PX_PER_MM: f64 = 12.0;

/// A raw evdev event stripped to the fields that matter. Mirrors
/// `input_event` minus the timestamp (the kernel re-stamps everything
/// written to uinput anyway).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ev {
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}

impl Ev {
    pub const fn new(type_: u16, code: u16, value: i32) -> Self {
        Ev { type_, code, value }
    }
    pub const fn abs(code: u16, value: i32) -> Self {
        Ev::new(EV_ABS, code, value)
    }
    pub const fn syn() -> Self {
        Ev::new(EV_SYN, SYN_REPORT, 0)
    }
}

/// An effect the I/O shell must apply, in order.
#[derive(Clone, PartialEq, Debug)]
pub enum Output {
    /// Write these events to the synthetic touchpad clone.
    EmitSynth(Vec<Ev>),
    /// Press the virtual mouse's left button.
    MouseDown,
    /// Release the virtual mouse's left button.
    MouseUp,
    /// Move the cursor by whole pixels (fractional remainders are
    /// carried inside the machine so slow drags don't lose motion).
    MouseMove { dx: i32, dy: i32 },
}

/// The timing/scaling knobs the machine needs; derived from the user
/// config (and re-derivable on hot reload via [`GestureMachine::set_timing`]).
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub probe_delay: Duration,
    pub entry_debounce: Duration,
    /// 0 disables drag-lock entirely (the default). When > 0, lifting
    /// all fingers keeps the virtual button held for this long; a new
    /// touch that settles as a 3-finger drag resumes the same drag,
    /// while a touch that settles as anything else releases the button
    /// *before* its buffered events are relayed -- so ordinary pointer
    /// motion after a drag can never smear the held button around
    /// (the exact regression the first drag-lock attempt shipped).
    pub drag_end_delay: Duration,
    /// How long after a drag commits the button press is deferred when
    /// the fingers haven't moved yet. The press fires at the first
    /// actual drag motion or when this grace expires -- whichever comes
    /// first -- so a 4th finger landing *after* the entry window (a
    /// fast/sloppy 4-finger swipe, whose fingers stagger more the
    /// faster the hand comes down) can abort the misclassified drag
    /// without a phantom click ever having been sent.
    pub press_grace: Duration,
    /// Combined px-per-mm * user acceleration factor.
    pub px_per_mm: f64,
}

#[derive(Clone, Copy)]
struct Slot {
    tracking_id: i32,
    x: i32,
    y: i32,
}

impl Default for Slot {
    fn default() -> Self {
        Slot {
            tracking_id: -1,
            x: 0,
            y: 0,
        }
    }
}

/// Authoritative per-slot state, as re-read from the kernel after a
/// SYN_DROPPED (EVIOCGMTSLOTS). `(tracking_id, x, y)`.
pub type SlotSnapshot = [(i32, i32, i32)];

pub struct GestureMachine {
    timing: Timing,
    x_res: f64,
    y_res: f64,
    slot_count: usize,

    slots: [Slot; MAX_SLOTS],
    current_slot: usize,
    /// Which slots the *synthetic clone* currently believes are active.
    relayed_active: [bool; MAX_SLOTS],

    /// True while a 3-finger drag owns the touch: nothing is relayed.
    suppressing: bool,
    drag_ref_slot: Option<usize>,
    drag_last_pos: Option<(i32, i32)>,
    /// Sub-pixel motion carried between frames.
    carry: (f64, f64),
    /// While a committed drag hasn't moved yet: when to press the button
    /// anyway (see [`Timing::press_grace`]). Cleared once pressed.
    press_deadline: Option<Instant>,

    /// Last EV_KEY values seen from the REAL device (BTN_TOUCH,
    /// BTN_TOOL_*...). The truth about tool state on the pad.
    real_keys: Vec<(u16, i32)>,
    /// Last EV_KEY values the CLONE has been told. Needed to emit
    /// consistent closing state (BTN_TOUCH=0 etc.) when suppression
    /// yanks a partially-relayed touch away -- leaving these stuck at 1
    /// desyncs libinput's finger accounting (and with tap-to-click on,
    /// a slot release without tool release reads as a 2-finger tap:
    /// phantom right-click).
    clone_keys: Vec<(u16, i32)>,

    /// Frames withheld from the compositor while a fresh touch is still
    /// being classified.
    pending: Vec<Ev>,
    touch_start: Option<Instant>,
    touch_max: usize,
    settled: bool,

    /// Virtual left button state (survives across touches for drag-lock).
    held: bool,
    /// When set, the button stays held until this instant unless a new
    /// touch resolves the lock first (see [`Timing::drag_end_delay`]).
    lock_deadline: Option<Instant>,
}

impl GestureMachine {
    pub fn new(timing: Timing, x_res: f64, y_res: f64, slot_count: usize) -> Self {
        GestureMachine {
            timing,
            x_res: x_res.max(1.0),
            y_res: y_res.max(1.0),
            slot_count: slot_count.clamp(1, MAX_SLOTS),
            slots: [Slot::default(); MAX_SLOTS],
            current_slot: 0,
            relayed_active: [false; MAX_SLOTS],
            suppressing: false,
            drag_ref_slot: None,
            drag_last_pos: None,
            carry: (0.0, 0.0),
            press_deadline: None,
            real_keys: Vec::new(),
            clone_keys: Vec::new(),
            pending: Vec::new(),
            touch_start: None,
            touch_max: 0,
            settled: false,
            held: false,
            lock_deadline: None,
        }
    }

    /// Hot-reload hook: swap in new timing/scaling without disturbing
    /// any in-flight touch state.
    pub fn set_timing(&mut self, timing: Timing) {
        self.timing = timing;
    }

    /// Whether the virtual button is currently held (used by the shell
    /// for a defensive release on shutdown).
    pub fn button_held(&self) -> bool {
        self.held
    }

    /// The next instant at which [`on_tick`](Self::on_tick) has work to
    /// do, if any. The I/O loop sleeps exactly until this, so decisions
    /// land on time instead of on the next poll interval.
    pub fn next_deadline(&self) -> Option<Instant> {
        if self.suppressing {
            // a committed drag that hasn't moved yet still owes a
            // deferred button press
            return if self.held { None } else { self.press_deadline };
        }
        if let Some(start) = self.touch_start {
            if !self.settled {
                let window = if self.touch_max <= 1 {
                    self.timing.probe_delay
                } else {
                    self.timing.entry_debounce
                };
                return Some(start + window);
            }
            return None; // settled live touch: nothing timed left to decide
        }
        self.lock_deadline
    }

    /// Wall-clock-only work: classification windows closing on a touch
    /// that's holding perfectly still, the deferred drag press, and the
    /// drag-lock timeout.
    pub fn on_tick(&mut self, now: Instant) -> Vec<Output> {
        let mut out = Vec::new();
        if self.suppressing {
            // Stationary drag: no motion has pressed the button yet, and
            // no 4th finger has shown up to abort -- commit the press.
            if !self.held {
                if let Some(deadline) = self.press_deadline {
                    if now >= deadline {
                        self.press_button(&mut out);
                    }
                }
            }
            return out;
        }

        if self.touch_start.is_none() {
            if let Some(deadline) = self.lock_deadline {
                if now >= deadline {
                    self.lock_deadline = None;
                    self.release_button(&mut out);
                }
            }
            return out;
        }

        if self.settled {
            return out;
        }

        let count = self.active_count();
        let start = self.touch_start.expect("guarded by is_none() above");

        if count == 1 && self.touch_max == 1 && now >= start + self.timing.probe_delay {
            self.settled = true;
            self.flush_pending(&mut out);
            return out;
        }
        if now >= start + self.timing.entry_debounce {
            self.resolve_touch_decision(count, now, &mut out);
        }
        out
    }

    /// Feed one complete frame (everything up to and including its
    /// SYN_REPORT). Partial frames interrupted by SYN_DROPPED must not
    /// be fed; discard them and call [`on_resync`](Self::on_resync)
    /// with a fresh kernel snapshot instead.
    pub fn on_frame(&mut self, frame: &[Ev], now: Instant) -> Vec<Output> {
        // 1. fold the frame's slot and key-state updates into our model
        for ev in frame {
            if ev.type_ == EV_KEY {
                Self::note_key(&mut self.real_keys, ev.code, ev.value);
                continue;
            }
            if ev.type_ != EV_ABS {
                continue;
            }
            match ev.code {
                ABS_MT_SLOT => {
                    self.current_slot = (ev.value.max(0) as usize).min(self.slot_count - 1);
                }
                ABS_MT_TRACKING_ID => self.slots[self.current_slot].tracking_id = ev.value,
                ABS_MT_POSITION_X => self.slots[self.current_slot].x = ev.value,
                ABS_MT_POSITION_Y => self.slots[self.current_slot].y = ev.value,
                _ => {}
            }
        }
        // 2. decide what to do about it
        let mut out = Vec::new();
        self.decide(frame, now, &mut out);
        out
    }

    /// Reconcile with an authoritative kernel snapshot after the kernel
    /// reported dropped events (SYN_DROPPED). Whatever the dropped
    /// events said is gone; `snapshot` is the truth now.
    pub fn on_resync(&mut self, snapshot: &SlotSnapshot, now: Instant) -> Vec<Output> {
        for slot in 0..MAX_SLOTS {
            self.slots[slot] = match snapshot.get(slot) {
                Some(&(id, x, y)) => Slot {
                    tracking_id: id,
                    x,
                    y,
                },
                None => Slot::default(),
            };
        }

        let mut out = Vec::new();

        if self.suppressing {
            // drive_drag reads self.slots directly next frame; the drop
            // may have moved the reference finger arbitrarily far, so
            // re-baseline rather than applying the gap as a cursor jump.
            self.drag_last_pos = None;
        } else if self.touch_start.is_some() && !self.settled && self.active_count() == 0 {
            // Every finger lifted *inside* the dropped window while the
            // touch was still buffered. The buffer holds touchdowns whose
            // matching releases were dropped -- flushing it would leave
            // the synthetic device holding a touch forever. Swallow the
            // (rare) truncated tap instead; consistency wins.
            self.pending.clear();
            self.touch_start = None;
            self.touch_max = 0;
        } else {
            // The synthetic device (or the buffer of a still-undecided
            // touch) may hold stale state. Correct it explicitly: release
            // any slot the clone believes is active but no longer is,
            // then assert the authoritative position of every live slot.
            let mut correction = Vec::new();
            for slot in 0..self.slot_count {
                if self.relayed_active[slot] && self.slots[slot].tracking_id < 0 {
                    correction.push(Ev::abs(ABS_MT_SLOT, slot as i32));
                    correction.push(Ev::abs(ABS_MT_TRACKING_ID, -1));
                }
            }
            correction.extend(self.active_slot_dump());
            if !correction.is_empty() {
                if self.touch_start.is_some() && !self.settled {
                    self.pending.extend(correction);
                    self.pending.push(Ev::syn());
                } else {
                    let mut frame = correction;
                    frame.push(Ev::syn());
                    self.mark_relayed();
                    out.push(Output::EmitSynth(frame));
                }
            }
        }

        // The dropped events may have included liftoffs (even the whole
        // touch ending). Run the normal decision logic against the new
        // state with an empty frame so we can't be left suppressing (or
        // buffering) a touch that no longer exists.
        self.decide(&[], now, &mut out);
        out
    }

    // ---- internals ----------------------------------------------------

    fn active_slots(&self) -> Vec<usize> {
        (0..self.slot_count)
            .filter(|&s| self.slots[s].tracking_id >= 0)
            .collect()
    }

    fn active_count(&self) -> usize {
        (0..self.slot_count)
            .filter(|&s| self.slots[s].tracking_id >= 0)
            .count()
    }

    fn decide(&mut self, frame: &[Ev], now: Instant, out: &mut Vec<Output>) {
        let active = self.active_slots();
        let count = active.len();

        // Once a drag has started, stay suppressed until every finger is
        // off, not just until the count first drops below 3. Fingers
        // never lift in perfect unison; without this hysteresis the
        // trailing 1-2 fingers of a liftoff would be relayed as a fresh
        // touch, which libinput reads as a 2-finger tap (right-click)
        // the moment they lift too.
        if self.suppressing {
            if count == 0 {
                self.suppressing = false;
                self.drag_ref_slot = None;
                self.drag_last_pos = None;
                self.press_deadline = None;
                // Reset touch bookkeeping: without this the next touch
                // would inherit touch_max/settled from this drag and
                // skip the debounce protection entirely.
                self.touch_start = None;
                self.touch_max = 0;
                self.settled = false;
                // A committed drag that never moved and lifted before
                // the press grace still owes its click: press now so the
                // release below (or the drag-lock) completes it.
                self.press_button(out);
                if self.timing.drag_end_delay > Duration::ZERO {
                    // Drag-lock: keep the button held; a new 3-finger
                    // touch inside the window resumes the drag, anything
                    // else releases it (see flush_pending / on_tick).
                    self.lock_deadline = Some(now + self.timing.drag_end_delay);
                } else {
                    self.release_button(out);
                }
                // The synth clone has nothing active on it (suppression
                // never relayed anything), so there's nothing to resync.
                return;
            }
            if count >= 4 {
                // A 4th finger arrived AFTER the entry window closed --
                // this was never a drag, it's a fast/sloppy 4-finger
                // gesture whose last finger staggered in late (the
                // faster the hand comes down, the bigger the stagger).
                // Abort: release the touch to the compositor mid-gesture
                // so the rest of the swipe still registers. Thanks to
                // the deferred press, in the common case no button was
                // ever pressed, so nothing to undo.
                if self.held {
                    warn!(
                        "4th finger after the drag already pressed the button; \
                        releasing (a brief phantom click was unavoidable)"
                    );
                } else {
                    debug!("late 4th finger: aborting committed drag, handing touch to compositor");
                }
                self.suppressing = false;
                self.drag_ref_slot = None;
                self.drag_last_pos = None;
                self.press_deadline = None;
                self.settled = true; // continues as an ordinary live touch
                self.release_button(out);
                // Introduce the touch to the clone as a fresh, complete,
                // consistent touchdown: all live slots plus the real
                // pad's current tool state (BTN_TOUCH/BTN_TOOL_*).
                let mut intro = self.active_slot_dump();
                for i in 0..self.real_keys.len() {
                    let (code, value) = self.real_keys[i];
                    if Self::key_value(&self.clone_keys, code) != value {
                        intro.push(Ev::new(EV_KEY, code, value));
                        Self::note_key(&mut self.clone_keys, code, value);
                    }
                }
                intro.push(Ev::syn());
                self.mark_relayed();
                out.push(Output::EmitSynth(intro));
                return;
            }
            self.drive_drag(&active, out);
            // frame intentionally not relayed
            return;
        }

        if count == 0 {
            let had_pending = self.touch_start.is_some() && !self.settled;
            self.touch_start = None;
            self.touch_max = 0;
            self.settled = false;
            if had_pending {
                // Touch ended before a decision was reached (e.g. a
                // quick tap): flush everything buffered, including this
                // release frame, so the tap isn't silently swallowed.
                self.pending.extend_from_slice(frame);
                self.flush_pending(out);
                return;
            }
            // Already-settled touch ending (most touches): this frame
            // carries the release events the compositor needs to see.
            self.relay_frame(frame, out);
            return;
        }

        if self.touch_start.is_none() {
            // the first frame of a brand new touch
            self.touch_start = Some(now);
            self.touch_max = count;
            self.settled = false;
            self.pending.clear();
        } else {
            self.touch_max = self.touch_max.max(count);
        }

        if self.settled {
            // Already decided this touch is an ordinary gesture -- relay
            // live. One exception: a touch that *grew* to exactly 3
            // without ever lifting (1->2->3 well after the window
            // closed) is a deliberate late drag and must not leak
            // through as a real 3-finger touch. The touch_max == 3
            // guard is what keeps the *other* way of hitting count == 3
            // -- a 4-finger gesture shedding a finger (4->3), which
            // happens at the tail of every 4-finger swipe because
            // fingers never lift in unison -- from being hijacked into
            // a phantom drag + click.
            if count == 3 && self.touch_max == 3 {
                self.commit_drag(&active, now, out);
                return;
            }
            self.relay_frame(frame, out);
            return;
        }

        self.pending.extend_from_slice(frame);

        if self.touch_max >= 4 {
            // Unambiguously bigger than a 3-finger drag could ever be --
            // no need to wait out the rest of the window.
            self.settled = true;
            self.flush_pending(out);
            return;
        }

        let start = self.touch_start.expect("set above when the touch began");

        if count == 1 && self.touch_max == 1 && now >= start + self.timing.probe_delay {
            // Still just one finger after a short probe: ordinary
            // pointer movement, by far the most common case. Go live now
            // rather than waiting out the full entry_debounce, or every
            // touch-lift-reposition cycle of normal cursor use would add
            // a felt hitch.
            self.settled = true;
            self.flush_pending(out);
            return;
        }

        if now >= start + self.timing.entry_debounce {
            self.resolve_touch_decision(count, now, out);
        }
    }

    /// The entry_debounce window has closed: commit to a drag if the
    /// touch held stably at exactly 3 fingers the whole time, otherwise
    /// release it to the compositor as an ordinary gesture.
    fn resolve_touch_decision(&mut self, count: usize, now: Instant, out: &mut Vec<Output>) {
        if count == 3 && self.touch_max == 3 {
            self.pending.clear();
            let active = self.active_slots();
            self.commit_drag(&active, now, out);
            return;
        }
        self.settled = true;
        self.flush_pending(out);
    }

    /// Commit the current touch as a 3-finger drag. The button press is
    /// DEFERRED: it fires at the first actual drag motion, or when
    /// press_grace expires -- so a late 4th finger (fast 4-finger swipe)
    /// can still abort without a phantom click having been sent.
    fn commit_drag(&mut self, active: &[usize], now: Instant, out: &mut Vec<Output>) {
        debug!("3-finger touch committed as a drag");
        self.settled = true;
        self.enter_suppress(out);
        if !self.held {
            self.press_deadline = Some(now + self.timing.press_grace);
        }
        self.drive_drag(active, out);
    }

    /// Releases a buffered touch to the compositor: it either never
    /// became a drag, or grew past 3 into a bigger gesture that isn't
    /// ours to intercept. If a drag-lock is pending, the button is
    /// released *first*, so the flushed motion can never drag anything.
    fn flush_pending(&mut self, out: &mut Vec<Output>) {
        if self.lock_deadline.take().is_some() {
            self.release_button(out);
        }
        if !self.pending.is_empty() {
            let flushed = std::mem::take(&mut self.pending);
            self.note_clone_keys(&flushed);
            out.push(Output::EmitSynth(flushed));
        }
        self.mark_relayed();
    }

    fn relay_frame(&mut self, frame: &[Ev], out: &mut Vec<Output>) {
        self.mark_relayed();
        if !frame.is_empty() {
            self.note_clone_keys(frame);
            out.push(Output::EmitSynth(frame.to_vec()));
        }
    }

    fn mark_relayed(&mut self) {
        for slot in 0..MAX_SLOTS {
            self.relayed_active[slot] = self.slots[slot].tracking_id >= 0;
        }
    }

    /// Record what EV_KEY state a batch of events tells the clone.
    fn note_clone_keys(&mut self, events: &[Ev]) {
        for ev in events {
            if ev.type_ == EV_KEY {
                Self::note_key(&mut self.clone_keys, ev.code, ev.value);
            }
        }
    }

    fn note_key(map: &mut Vec<(u16, i32)>, code: u16, value: i32) {
        for entry in map.iter_mut() {
            if entry.0 == code {
                entry.1 = value;
                return;
            }
        }
        map.push((code, value));
    }

    fn key_value(map: &[(u16, i32)], code: u16) -> i32 {
        map.iter().find(|e| e.0 == code).map(|e| e.1).unwrap_or(0)
    }

    /// Begin withholding everything from the compositor. Any slots the
    /// synthetic clone still believes are active are explicitly released
    /// first -- and any tool state (BTN_TOUCH, BTN_TOOL_*) it was told is
    /// pressed is explicitly released too, so it is never left holding a
    /// half-open touch or stuck finger-count bits (which desync
    /// libinput's tap/gesture accounting).
    fn enter_suppress(&mut self, out: &mut Vec<Output>) {
        self.suppressing = true;
        self.lock_deadline = None; // a live drag owns the button now
        self.carry = (0.0, 0.0);

        let mut release = Vec::new();
        for slot in 0..MAX_SLOTS {
            if self.relayed_active[slot] {
                release.push(Ev::abs(ABS_MT_SLOT, slot as i32));
                release.push(Ev::abs(ABS_MT_TRACKING_ID, -1));
                self.relayed_active[slot] = false;
            }
        }
        for i in 0..self.clone_keys.len() {
            let (code, value) = self.clone_keys[i];
            if value != 0 {
                release.push(Ev::new(EV_KEY, code, 0));
                self.clone_keys[i].1 = 0;
            }
        }
        if !release.is_empty() {
            release.push(Ev::syn());
            out.push(Output::EmitSynth(release));
        }
    }

    fn press_button(&mut self, out: &mut Vec<Output>) {
        if !self.held {
            self.held = true;
            self.press_deadline = None;
            out.push(Output::MouseDown);
        }
        // if still held from a drag-lock, the drag just resumes --
        // no re-press, no glitch
    }

    fn release_button(&mut self, out: &mut Vec<Output>) {
        if self.held {
            self.held = false;
            out.push(Output::MouseUp);
        }
    }

    fn drive_drag(&mut self, active: &[usize], out: &mut Vec<Output>) {
        let reference = match self.drag_ref_slot {
            Some(s) if active.contains(&s) => s,
            _ => {
                // first frame of the gesture, or the previous reference
                // finger lifted and another took its place: re-baseline
                // without applying a delta this frame.
                let s = active[0];
                self.drag_ref_slot = Some(s);
                self.drag_last_pos = Some((self.slots[s].x, self.slots[s].y));
                return;
            }
        };

        let (x, y) = (self.slots[reference].x, self.slots[reference].y);
        if let Some((lx, ly)) = self.drag_last_pos {
            let px = (x - lx) as f64 / self.x_res * self.timing.px_per_mm + self.carry.0;
            let py = (y - ly) as f64 / self.y_res * self.timing.px_per_mm + self.carry.1;
            let dx = px.trunc() as i32;
            let dy = py.trunc() as i32;
            // carry the sub-pixel remainder instead of discarding it, so
            // slow, precise drags don't systematically lose motion
            self.carry = (px - dx as f64, py - dy as f64);
            if dx != 0 || dy != 0 {
                // real drag motion: the deferred press (if still pending)
                // must land before the movement it accompanies
                self.press_button(out);
                out.push(Output::MouseMove { dx, dy });
            }
        } else {
            self.drag_last_pos = Some((x, y));
            return;
        }
        self.drag_last_pos = Some((x, y));
    }

    /// SLOT/TRACKING_ID/X/Y events asserting the current state of every
    /// active slot (used to correct downstream state after a resync).
    fn active_slot_dump(&self) -> Vec<Ev> {
        let mut dump = Vec::new();
        for slot in 0..self.slot_count {
            let s = self.slots[slot];
            if s.tracking_id >= 0 {
                dump.push(Ev::abs(ABS_MT_SLOT, slot as i32));
                dump.push(Ev::abs(ABS_MT_TRACKING_ID, s.tracking_id));
                dump.push(Ev::abs(ABS_MT_POSITION_X, s.x));
                dump.push(Ev::abs(ABS_MT_POSITION_Y, s.y));
            }
        }
        dump
    }
}

#[cfg(test)]
mod tests;
