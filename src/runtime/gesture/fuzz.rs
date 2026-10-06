//! Randomized invariant testing: thousands of chaotic-but-realistic
//! multi-finger sequences thrown at the state machine, with a shadow
//! model of the synthetic clone tracking every event the machine emits.
//! After every single step we assert the invariants that, if ever
//! violated, produce exactly the class of bug this project has been
//! bitten by before: a clone left holding phantom touches or stuck tool
//! bits, a virtual button that never releases, or a panic.
//!
//! The PRNG is a fixed-seed LCG, so every failure is perfectly
//! reproducible: re-run the test, get the same sequence.

use super::*;
use std::time::Duration;

/// Deterministic PRNG (Knuth MMIX LCG).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 16
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const BTN_TOUCH: u16 = 0x14a;
const BTN_TOOL_FINGER: u16 = 0x145;
const BTN_TOOL_DOUBLETAP: u16 = 0x14d;
const BTN_TOOL_TRIPLETAP: u16 = 0x14e;
const BTN_TOOL_QUADTAP: u16 = 0x14f;
const BTN_TOOL_QUINTTAP: u16 = 0x148;

const TOOL_BITS: [u16; 5] = [
    BTN_TOOL_FINGER,
    BTN_TOOL_DOUBLETAP,
    BTN_TOOL_TRIPLETAP,
    BTN_TOOL_QUADTAP,
    BTN_TOOL_QUINTTAP,
];

/// Shadow model of the synthetic clone: replays everything the machine
/// emits, exactly as the kernel/libinput would accumulate it.
struct ShadowClone {
    active: [bool; MAX_SLOTS],
    tracking_id: [i32; MAX_SLOTS],
    x: [i32; MAX_SLOTS],
    y: [i32; MAX_SLOTS],
    keys: Vec<(u16, i32)>,
    current_slot: usize,
}

impl Default for ShadowClone {
    fn default() -> Self {
        ShadowClone {
            active: [false; MAX_SLOTS],
            tracking_id: [-1; MAX_SLOTS],
            x: [0; MAX_SLOTS],
            y: [0; MAX_SLOTS],
            keys: Vec::new(),
            current_slot: 0,
        }
    }
}

impl ShadowClone {
    fn feed(&mut self, evs: &[Ev]) {
        for ev in evs {
            match (ev.type_, ev.code) {
                (EV_ABS, ABS_MT_SLOT) => {
                    self.current_slot = (ev.value.max(0) as usize).min(MAX_SLOTS - 1);
                }
                (EV_ABS, ABS_MT_TRACKING_ID) => {
                    self.active[self.current_slot] = ev.value >= 0;
                    self.tracking_id[self.current_slot] = ev.value;
                    // A tracking ID identifies one physical contact. Check
                    // immediately after each ID update so a transient
                    // duplicate cannot be hidden by a later event in the
                    // same frame.
                    self.assert_unique_tracking_ids();
                }
                (EV_ABS, ABS_MT_POSITION_X) => {
                    self.x[self.current_slot] = ev.value;
                }
                (EV_ABS, ABS_MT_POSITION_Y) => {
                    self.y[self.current_slot] = ev.value;
                }
                (EV_KEY, code) => {
                    if let Some(e) = self.keys.iter_mut().find(|e| e.0 == code) {
                        e.1 = ev.value;
                    } else {
                        self.keys.push((code, ev.value));
                    }
                }
                _ => {}
            }
        }
    }
    fn active_count(&self) -> usize {
        self.active.iter().filter(|a| **a).count()
    }

    fn assert_unique_tracking_ids(&self) {
        for left in 0..MAX_SLOTS {
            if !self.active[left] || self.tracking_id[left] < 0 {
                continue;
            }
            for right in (left + 1)..MAX_SLOTS {
                if self.active[right] && self.tracking_id[right] >= 0 {
                    assert_ne!(
                        self.tracking_id[left], self.tracking_id[right],
                        "clone duplicates tracking ID {} in slots {left} and {right}",
                        self.tracking_id[left]
                    );
                }
            }
        }
    }

    fn key_value(&self, code: u16) -> i32 {
        self.keys
            .iter()
            .find(|(key, _)| *key == code)
            .map_or(0, |(_, value)| *value)
    }

    fn pressed_keys(&self) -> Vec<u16> {
        self.keys.iter().filter(|e| e.1 != 0).map(|e| e.0).collect()
    }
}

/// One simulated "hand": up to 5 fingers occupying random slots, with
/// realistic tool-bit reporting (BTN_TOUCH + BTN_TOOL_<count>).
struct Hand {
    fingers: [Option<(i32, i32, i32)>; MAX_SLOTS], // (id, x, y) per slot
    next_id: i32,
    current_slot: usize,
    last_tool: Option<u16>,
    touching: bool,
}

impl Hand {
    fn new() -> Self {
        Hand {
            fingers: [None; MAX_SLOTS],
            next_id: 1,
            current_slot: 0,
            last_tool: None,
            touching: false,
        }
    }

    fn count(&self) -> usize {
        self.fingers.iter().filter(|f| f.is_some()).count()
    }

    /// Complete key state used by EVIOCGKEY after SYN_DROPPED. Unlike
    /// `tool_events`, this includes both pressed and released tool bits.
    fn key_state(&self) -> Vec<(u16, i32)> {
        let count = self.count();
        let active_tool = if count > 0 {
            Some((count - 1).min(TOOL_BITS.len() - 1))
        } else {
            None
        };
        let mut keys = Vec::with_capacity(TOOL_BITS.len() + 1);
        keys.push((BTN_TOUCH, i32::from(count > 0)));
        keys.extend(
            TOOL_BITS
                .iter()
                .enumerate()
                .map(|(index, code)| (*code, i32::from(active_tool == Some(index)))),
        );
        keys
    }

    /// Select a slot only when it differs from the kernel's persistent
    /// ABS_MT_SLOT state. Real evdev frames omit unchanged slot selections,
    /// including across SYN_REPORT boundaries and idle frames.
    fn select_slot(&mut self, slot: usize) -> Vec<Ev> {
        if self.current_slot == slot {
            Vec::new()
        } else {
            self.current_slot = slot;
            vec![Ev::abs(ABS_MT_SLOT, slot as i32)]
        }
    }

    /// Events updating BTN_TOUCH / BTN_TOOL_* to match the new count,
    /// the way real touchpad firmware reports it.
    fn tool_events(&mut self) -> Vec<Ev> {
        let mut evs = Vec::new();
        let count = self.count();
        let tool = if count == 0 {
            None
        } else {
            Some(TOOL_BITS[(count - 1).min(4)])
        };
        if tool != self.last_tool {
            if let Some(old) = self.last_tool {
                evs.push(Ev::new(EV_KEY, old, 0));
            }
            if let Some(new) = tool {
                evs.push(Ev::new(EV_KEY, new, 1));
            }
            self.last_tool = tool;
        }
        let touching = count > 0;
        if touching != self.touching {
            evs.push(Ev::new(EV_KEY, BTN_TOUCH, i32::from(touching)));
            self.touching = touching;
        }
        evs
    }

    fn land(&mut self, rng: &mut Lcg) -> Vec<Ev> {
        let free: Vec<usize> = (0..MAX_SLOTS)
            .filter(|&s| self.fingers[s].is_none())
            .collect();
        if free.is_empty() || self.count() >= 5 {
            return Vec::new();
        }
        let slot = free[rng.below(free.len() as u64) as usize];
        let id = self.next_id;
        self.next_id += 1;
        let x = rng.below(2000) as i32;
        let y = rng.below(1400) as i32;
        self.fingers[slot] = Some((id, x, y));
        let mut evs = self.select_slot(slot);
        evs.extend([
            Ev::abs(ABS_MT_TRACKING_ID, id),
            Ev::abs(ABS_MT_POSITION_X, x),
            Ev::abs(ABS_MT_POSITION_Y, y),
        ]);
        evs.extend(self.tool_events());
        evs
    }

    fn lift(&mut self, rng: &mut Lcg) -> Vec<Ev> {
        let used: Vec<usize> = (0..MAX_SLOTS)
            .filter(|&s| self.fingers[s].is_some())
            .collect();
        if used.is_empty() {
            return Vec::new();
        }
        let slot = used[rng.below(used.len() as u64) as usize];
        self.fingers[slot] = None;
        let mut evs = self.select_slot(slot);
        evs.push(Ev::abs(ABS_MT_TRACKING_ID, -1));
        evs.extend(self.tool_events());
        evs
    }

    fn wiggle(&mut self, rng: &mut Lcg) -> Vec<Ev> {
        let mut evs = Vec::new();
        for slot in 0..MAX_SLOTS {
            let moved = if let Some((_, x, y)) = self.fingers[slot].as_mut() {
                if rng.below(2) == 0 {
                    *x = (*x + rng.below(41) as i32 - 20).clamp(0, 2000);
                    *y = (*y + rng.below(41) as i32 - 20).clamp(0, 1400);
                    Some((*x, *y))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((x, y)) = moved {
                evs.extend(self.select_slot(slot));
                evs.push(Ev::abs(ABS_MT_POSITION_X, x));
                evs.push(Ev::abs(ABS_MT_POSITION_Y, y));
            }
        }
        evs
    }
}

fn run_scenario(seed: u64, drag_end_delay_ms: u64) {
    let mut rng = Lcg(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut m = GestureMachine::new(
        Timing {
            probe_delay: Duration::from_millis(15),
            entry_debounce: Duration::from_millis(50),
            drag_end_delay: Duration::from_millis(drag_end_delay_ms),
            press_grace: Duration::from_millis(75),
            px_per_mm: PX_PER_MM,
        },
        10.0,
        10.0,
        16,
    );
    let mut shadow = ShadowClone::default();
    let mut hand = Hand::new();
    let mut now = Instant::now();
    let mut expect_held = false;

    let apply = |outs: &[Output], shadow: &mut ShadowClone, expect_held: &mut bool, ctx: &str| {
        for o in outs {
            match o {
                Output::EmitSynth(evs) => shadow.feed(evs),
                Output::MouseDown => {
                    assert!(!*expect_held, "double MouseDown ({ctx}, seed {seed})");
                    *expect_held = true;
                }
                Output::MouseUp => {
                    assert!(*expect_held, "MouseUp while not held ({ctx}, seed {seed})");
                    *expect_held = false;
                }
                Output::MouseMove { .. } => {}
            }
        }
    };

    let assert_keys_match = |shadow: &ShadowClone, hand: &Hand, ctx: &str| {
        for (code, expected) in hand.key_state() {
            assert_eq!(
                shadow.key_value(code),
                expected,
                "clone key {code:#x} has {}, real pad has {expected} ({ctx}, seed {seed})",
                shadow.key_value(code)
            );
        }
    };

    for step in 0..2500u32 {
        // random action: land / lift / wiggle / idle
        let evs = match rng.below(10) {
            0..=2 => hand.land(&mut rng),
            3..=5 => hand.lift(&mut rng),
            6..=8 => hand.wiggle(&mut rng),
            _ => Vec::new(),
        };

        // advance time by a random small amount, delivering ticks the
        // way the event loop would (deadline-driven)
        let dt = 1 + rng.below(60);
        for _ in 0..2 {
            // fire any deadline that falls inside this window, like the
            // real select! loop does
            if let Some(deadline) = m.next_deadline() {
                if deadline <= now + Duration::from_millis(dt) {
                    let outs = m.on_tick(deadline);
                    apply(&outs, &mut shadow, &mut expect_held, "tick");
                }
            }
        }
        now += Duration::from_millis(dt);
        let outs = m.on_tick(now);
        apply(&outs, &mut shadow, &mut expect_held, "tick2");

        if !evs.is_empty() || rng.below(4) == 0 {
            let mut frame = evs;
            frame.push(Ev::syn());
            let outs = m.on_frame(&frame, now);
            apply(&outs, &mut shadow, &mut expect_held, "frame");
        }

        // occasionally simulate a SYN_DROPPED resync with the hand's
        // true state (what EVIOCGMTSLOTS would report)
        if rng.below(97) == 0 {
            let snapshot: Vec<(i32, i32, i32)> = (0..16)
                .map(|s| match hand.fingers[s] {
                    Some((id, x, y)) => (id, x, y),
                    None => (-1, 0, 0),
                })
                .collect();
            let keys = hand.key_state();
            let outs = m.on_resync(&snapshot, hand.current_slot, &keys, now);
            apply(&outs, &mut shadow, &mut expect_held, "resync");
            if (m.settled && !m.suppressing) || hand.count() == 0 {
                assert_keys_match(&shadow, &hand, "resync");
            }
            // a resync may swallow tool state legitimately; re-assert
            // the hand's tool bits on the next frame like hardware would
            let mut frame = hand.wiggle(&mut rng);
            frame.push(Ev::syn());
            let outs = m.on_frame(&frame, now);
            apply(&outs, &mut shadow, &mut expect_held, "post-resync");
            if (m.settled && !m.suppressing) || hand.count() == 0 {
                assert_keys_match(&shadow, &hand, "post-resync");
            }
        }

        // ---- INVARIANTS, every step --------------------------------
        // 1. the machine's belief about the clone matches the clone
        for slot in 0..MAX_SLOTS {
            assert_eq!(
                m.relayed_active[slot], shadow.active[slot],
                "clone slot-state desync at slot {slot} (seed {seed}, step {step})"
            );
        }
        // 2. machine's button belief matches emitted Down/Up pairing
        assert_eq!(
            m.held, expect_held,
            "button-state desync (seed {seed}, step {step})"
        );
        // 3. the clone never has MORE active slots than the real pad
        //    (it may briefly have fewer: buffered/suppressed touches)
        assert!(
            shadow.active_count() <= hand.count(),
            "clone shows {} touches, real pad has {} (seed {seed}, step {step})",
            shadow.active_count(),
            hand.count()
        );
        // 4. Once classification has settled on an ordinary live touch,
        //    every active slot the clone sees must carry the real finger's
        //    tracking ID and coordinates, not merely an active bit.
        if m.settled && !m.suppressing {
            for slot in 0..MAX_SLOTS {
                if !shadow.active[slot] {
                    continue;
                }
                let (id, x, y) = hand.fingers[slot].unwrap_or_else(|| {
                    panic!(
                        "clone slot {slot} is active but real pad is not (seed {seed}, step {step})"
                    )
                });
                assert_eq!(
                    shadow.tracking_id[slot], id,
                    "clone slot {slot} has tracking ID {}, real pad has {id} (seed {seed}, step {step})",
                    shadow.tracking_id[slot]
                );
                assert_eq!(
                    shadow.x[slot], x,
                    "clone slot {slot} has x={}, real pad has {x} (seed {seed}, step {step})",
                    shadow.x[slot]
                );
                assert_eq!(
                    shadow.y[slot], y,
                    "clone slot {slot} has y={}, real pad has {y} (seed {seed}, step {step})",
                    shadow.y[slot]
                );
            }
        }
        // 5. A valid tracking ID identifies one contact and must never be
        //    active in two clone slots at once. This is also checked after
        //    each TRACKING_ID event in ShadowClone::feed, before the frame
        //    can hide a transient duplicate.
        shadow.assert_unique_tracking_ids();
        if (m.settled && !m.suppressing) || hand.count() == 0 {
            assert_keys_match(&shadow, &hand, "step");
        }
    }

    // ---- quiescence: lift everything, let every deadline expire ----
    loop {
        let evs = hand.lift(&mut rng);
        if evs.is_empty() {
            break;
        }
        let mut frame = evs;
        frame.push(Ev::syn());
        now += Duration::from_millis(9);
        let outs = m.on_frame(&frame, now);
        apply(&outs, &mut shadow, &mut expect_held, "drain-lift");
    }
    // empty frame so a resync-less machine sees the final state
    let outs = m.on_frame(&[Ev::syn()], now);
    apply(&outs, &mut shadow, &mut expect_held, "drain-frame");
    for _ in 0..8 {
        now += Duration::from_millis(500);
        let outs = m.on_tick(now);
        apply(&outs, &mut shadow, &mut expect_held, "drain-tick");
    }

    assert!(
        !m.held,
        "virtual button left stuck down at quiescence (seed {seed})"
    );
    assert_eq!(
        shadow.active_count(),
        0,
        "clone left holding phantom touches at quiescence (seed {seed})"
    );
    assert!(
        shadow.pressed_keys().is_empty(),
        "clone left with stuck tool bits {:?} at quiescence (seed {seed})",
        shadow.pressed_keys()
    );
    assert_eq!(
        m.next_deadline(),
        None,
        "machine still scheduling wakeups at quiescence (seed {seed})"
    );
}

/// 96 scenarios x 2500 steps, half with drag-lock enabled. Fully
/// deterministic; a failing seed reproduces exactly.
#[test]
fn randomized_stress_invariants() {
    for seed in 0..48u64 {
        run_scenario(seed, 0);
        run_scenario(seed, 300);
    }
}
