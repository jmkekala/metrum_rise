// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic weather state: authored scenarios, smooth transitions and cloud advection.
//!
//! Weather advances on the same speed-scaled seconds as the day clock, never on wall time, so a
//! paused game holds the sky still and the same save, inputs and frame sequence reproduce the
//! same clouds. Advancing is O(1) and allocation-free. Motion is integrated as an accumulated
//! offset rather than derived from speed × time, so a wind change never makes clouds jump.
//!
//! Left automatic, the weather follows a seeded schedule: each change picks the next scenario by
//! weight from the current one, blends to it over tens of in-game minutes and holds it for a few
//! hours. Each change also turns the wind a little, pulled back towards the prevailing westerly,
//! retunes its speed and draws how much high cloud (cirrus) lies above the main layer; the high
//! layer drifts on its own, faster and veered wind. Every draw is a hash of the seed and the event's index, and a long step is split
//! exactly at each change, so 32x speed lays the same weather as 1x: O(1 + changes) per step.

use crate::simulation::vegetation::hash;

/// Wrap length of every accumulated offset, in metres. Every cloud-field period must divide it,
/// so the wrap is invisible.
pub const WEATHER_PERIOD_M: f64 = 65_536.0;

/// Wrap length of the wind clock, in seconds. The shaders round every wind
/// oscillator to a whole number of cycles in it, so the wrap is invisible; four and a half hours
/// is short enough that the clock keeps millisecond precision as an `f32`.
pub const WIND_PERIOD_S: f64 = 16_384.0;

// Wind speed the tree sway and water ripples are authored at: fair cumulus's, in m/s.
const AUTHORED_WIND_MS: f64 = 4.272_001;

/// Authored weather scenarios. Rain follows the deck (`WeatherSample::rain`); gameplay effects
/// are not modelled yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherScenario {
    /// Sparse clouds and light wind.
    Clear,
    /// Broken fair-weather cumulus; the default.
    FairCumulus,
    /// A nearly continuous grey cloud deck.
    Overcast,
    /// A dark, heavy deck, strong gusty wind and a downpour, without lightning yet.
    Storm,
    /// A closed, dark rain deck and moderate wind: steady rain, lighter than a storm's. Last, so
    /// the older scenarios keep their saved indices.
    Rain,
}

impl WeatherScenario {
    /// Every scenario, in the order their stable `index` values count.
    pub const ALL: [Self; 5] = [
        Self::Clear,
        Self::FairCumulus,
        Self::Overcast,
        Self::Storm,
        Self::Rain,
    ];

    /// Stable index used by saves and the Godot bridge.
    pub fn index(self) -> u8 {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as u8
    }

    /// Inverse of [`Self::index`]; `None` for an unknown index.
    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(index as usize).copied()
    }
}

// Wind X/Z (m per speed-scaled second), cloud coverage [0, 1], vertical cloud optical depth,
// high-cloud cover [0, 1]. Visual tuning values, not a meteorological model.
type Profile = [f64; 5];

// The authored profile. Its wind blows from the west-north-west, the prevailing direction; a
// scheduled change turns and rescales it (`scheduled_profile`).
fn profile(scenario: WeatherScenario) -> Profile {
    match scenario {
        WeatherScenario::Clear => [2.0, 0.5, 0.08, 0.4, 0.3],
        WeatherScenario::FairCumulus => [4.0, 1.5, 0.45, 1.2, 0.2],
        WeatherScenario::Overcast => [6.0, 2.0, 1.0, 2.2, 0.6],
        WeatherScenario::Storm => [14.0, 5.0, 1.0, 4.0, 0.0],
        // Deep enough to rain at about 0.6 of a storm's intensity (RAIN_DEPTH).
        WeatherScenario::Rain => [7.0, 2.5, 1.0, 3.4, 0.0],
    }
}

// A scheduled change turns the wind by up to this much either way, then pulls the result this
// share of the way back to the prevailing direction, so the wind wanders but mostly comes from
// the west.
const WIND_TURN_MAX_RAD: f64 = 70.0 * std::f64::consts::PI / 180.0;
const PREVAILING_PULL: f64 = 0.3;
// Least and most of the authored wind speed a scheduled change draws.
const WIND_SPEED_RANGE: [f64; 2] = [0.75, 1.3];
// High-cloud cover a scheduled change draws, least and most: a clear day may be cloudless or
// veiled in cirrus, and a storm's deck hides whatever lies above it.
const HIGH_COVER_RANGE: [[f64; 2]; 5] =
    [[0.0, 0.8], [0.0, 0.6], [0.3, 1.0], [0.0, 0.3], [0.0, 0.2]];
// The high layer's wind: faster than the low clouds' and veered (turned clockwise seen from
// above, as the wind turns with height in the northern hemisphere). At about 8 km it still
// crosses the sky more slowly than the low clouds do.
const HIGH_WIND_MULTIPLIER: f64 = 4.0;
const HIGH_WIND_VEER_RAD: f64 = 35.0 * std::f64::consts::PI / 180.0;

// Angle of a profile's wind against its scenario's authored wind, in (-pi, pi].
fn wind_turn(wind: [f64; 2], scenario: WeatherScenario) -> f64 {
    let authored = profile(scenario);
    wrap_angle(wind[1].atan2(wind[0]) - authored[1].atan2(authored[0]))
}

fn wrap_angle(angle: f64) -> f64 {
    let wrapped = (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU);
    wrapped - std::f64::consts::PI
}

// A profile's wind turned by `angle` and scaled by `speed`.
fn turned(mut to: Profile, angle: f64, speed: f64) -> Profile {
    let (sin, cos) = angle.sin_cos();
    let [x, z] = [to[0] * speed, to[1] * speed];
    to[0] = x * cos - z * sin;
    to[1] = x * sin + z * cos;
    to
}

// Whether a blend of reachable profiles can hold `value`: authored coverage and depth across the
// scenarios, any high cover, and wind no faster than the fastest drawn.
fn reachable(value: Profile) -> bool {
    let speed = value[0].hypot(value[1]);
    // A blend between two turned winds passes closer to calm than either, so only the fastest
    // bounds the speed.
    let fastest = WeatherScenario::ALL
        .iter()
        .map(|s| profile(*s)[0].hypot(profile(*s)[1]))
        .fold(f64::MIN, f64::max)
        * WIND_SPEED_RANGE[1];
    let within = |i: usize| {
        let (low, high) = WeatherScenario::ALL
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), s| {
                let v = profile(*s)[i];
                (lo.min(v), hi.max(v))
            });
        (low..=high).contains(&value[i])
    };
    value.iter().all(|v| v.is_finite())
        && speed <= fastest
        && within(2)
        && within(3)
        && (0.0..=1.0).contains(&value[4])
}

// Chance of each next scenario, in WeatherScenario::ALL order, from the current one. Weather
// moves to a neighbour: clear skies cloud over, a storm builds out of fair or overcast weather
// and breaks into overcast, so no change leaps from blue sky to a gale. Rain sets in from fair or
// overcast weather or follows a storm, and clears through overcast or fair weather.
const NEXT_WEIGHTS: [[f64; 5]; 5] = [
    [0.0, 0.75, 0.25, 0.0, 0.0],
    [0.35, 0.0, 0.45, 0.1, 0.1],
    [0.15, 0.45, 0.0, 0.2, 0.2],
    [0.05, 0.3, 0.45, 0.0, 0.2],
    [0.0, 0.3, 0.5, 0.2, 0.0],
];
// In-game hours a scenario holds once reached, least and most.
const DWELL_HOURS: [[f64; 2]; 5] = [[4.0, 10.0], [3.0, 8.0], [3.0, 9.0], [1.0, 3.0], [2.0, 6.0]];
// In-game minutes a change takes, least and most; a storm comes on faster.
const CHANGE_MINUTES: [[f64; 2]; 5] = [
    [45.0, 90.0],
    [40.0, 80.0],
    [40.0, 90.0],
    [20.0, 45.0],
    [40.0, 80.0],
];
// Hours the default fair weather holds before the schedule's first change.
const FIRST_DWELL_HOURS: f64 = 4.0;

// A deterministic draw in [0, 1) for one event and purpose.
fn draw(seed: u32, event: u32, purpose: i32) -> f64 {
    f64::from(hash(event as i32, purpose, seed ^ 0x77ea_7e2d) >> 8) / f64::from(1u32 << 24)
}

// Clouds drift faster than the surface wind they are tuned against.
const CLOUD_WIND_MULTIPLIER: f64 = 2.0;

// Rain falls from a closed deck thicker than an overcast one: it begins at an optical depth
// above Overcast's 2.2, so an overcast day stays dry, and is at full strength at Storm's 4.0.
// It therefore arrives and leaves with the deck that carries it, over the same transition.
const RAIN_DEPTH: [f64; 2] = [2.6, 4.0];
// The deck must be closed, or nearly, for rain: broken cloud at any depth stays dry.
const RAIN_COVERAGE: [f64; 2] = [0.9, 1.0];

fn smooth(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}

// Integral of `smooth` from zero to x, so advection stays continuous through a wind change.
fn smooth_integral(x: f64) -> f64 {
    x * x * x * (1.0 - 0.5 * x)
}

/// Authoritative weather state, saved with the game.
#[derive(Clone, Debug, PartialEq)]
pub struct WeatherState {
    /// Selects the cloud field's origin; the field itself is fixed.
    pub seed: u32,
    target: WeatherScenario,
    from: Profile,
    to: Profile,
    transition_elapsed_s: f64,
    transition_duration_s: f64,
    cloud_offset_m: [f64; 2],
    high_offset_m: [f64; 2],
    wind_clock_s: f64,
    automatic: bool,
    event_index: u32,
    until_event_h: f64,
}

/// Render-facing weather values for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeatherSample {
    /// World X/Z added to a position before looking up the cloud field, in metres, wrapped at
    /// [`WEATHER_PERIOD_M`]. Combines the seeded origin with the accumulated cloud drift.
    pub cloud_origin_m: [f32; 2],
    /// Cloud fraction control in `[0, 1]`.
    pub cloud_coverage: f32,
    /// World X/Z added to a position before looking up the high cloud layer, in metres, wrapped
    /// at [`WEATHER_PERIOD_M`]: its own seeded origin minus its own drift.
    pub high_origin_m: [f32; 2],
    /// High cloud (cirrus) cover in `[0, 1]`.
    pub high_cover: f32,
    /// Unit X/Z direction the wind blows towards.
    pub wind_direction: [f32; 2],
    /// Dimensionless vertical optical depth of a fully dense cloud.
    pub cloud_optical_depth: f32,
    /// Seconds of wind, wrapped at [`WIND_PERIOD_S`]: the phase every tree sway and water ripple
    /// reads. It runs with the game, never faster than normal speed, and stops while paused.
    pub wind_clock_s: f32,
    /// Current wind speed over the speed the sway and ripples are authored at.
    pub wind_strength: f32,
    /// Distance the wind has carried the air, in metres, wrapped at half [`WEATHER_PERIOD_M`]:
    /// the gust field's offset, which moves at the wind's speed as the clouds move at twice it.
    pub wind_offset_m: [f32; 2],
    /// Whether the seeded schedule moves the weather, rather than a scenario the player chose.
    pub automatic: bool,
    /// Rain intensity in `[0, 1]`, from dry to a downpour. Derived from the deck's coverage and
    /// optical depth, so it needs no state of its own and follows every transition.
    pub rain: f32,
}

/// Every field of a [`WeatherState`], as a save stores it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WeatherRecord {
    /// Cloud-field origin seed.
    pub seed: u32,
    /// [`WeatherScenario::index`] of the scenario settled on or moved towards.
    pub target: u8,
    /// Wind X/Z, coverage, optical depth and high cover the current transition started from.
    pub from: [f64; 5],
    /// Wind X/Z and high cover the current transition moves to; its coverage and depth are the
    /// target scenario's.
    pub to_wind: [f64; 2],
    /// High cover the current transition moves to.
    pub to_high_cover: f64,
    /// Speed-scaled seconds into the current transition.
    pub transition_elapsed_s: f64,
    /// Length of the current transition; zero when settled.
    pub transition_duration_s: f64,
    /// Accumulated cloud drift, wrapped at [`WEATHER_PERIOD_M`].
    pub cloud_offset_m: [f64; 2],
    /// Accumulated high cloud drift, wrapped at [`WEATHER_PERIOD_M`].
    pub high_offset_m: [f64; 2],
    /// Wind clock, wrapped at [`WIND_PERIOD_S`].
    pub wind_clock_s: f64,
    /// Whether the seeded schedule moves the weather.
    pub automatic: bool,
    /// Index of the schedule's next change, which seeds its draws.
    pub event_index: u32,
    /// In-game hours until the schedule's next change.
    pub until_event_h: f64,
}

impl Default for WeatherState {
    fn default() -> Self {
        Self::new(0)
    }
}

impl WeatherState {
    /// Starts settled fair weather with the given seed.
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            target: WeatherScenario::FairCumulus,
            from: profile(WeatherScenario::FairCumulus),
            to: profile(WeatherScenario::FairCumulus),
            transition_elapsed_s: 0.0,
            transition_duration_s: 0.0,
            cloud_offset_m: [0.0; 2],
            high_offset_m: [0.0; 2],
            wind_clock_s: 0.0,
            automatic: true,
            event_index: 0,
            until_event_h: FIRST_DWELL_HOURS,
        }
    }

    /// Scenario the weather is settled on or moving towards.
    pub fn target(&self) -> WeatherScenario {
        self.target
    }

    fn current_profile(&self) -> Profile {
        let target = self.to;
        let blend = if self.transition_duration_s > 0.0 {
            smooth(self.transition_elapsed_s / self.transition_duration_s)
        } else {
            1.0
        };
        std::array::from_fn(|i| self.from[i] + (target[i] - self.from[i]) * blend)
    }

    /// Moves to `scenario` over `duration_s` speed-scaled seconds, starting from the current
    /// blend so neither the look nor the cloud drift jumps. The wind keeps the direction the
    /// weather has turned it to, at the scenario's authored speed, and the high cloud moves to
    /// the scenario's authored cover. A non-positive or non-finite duration switches at once.
    pub fn transition_to(&mut self, scenario: WeatherScenario, duration_s: f64) {
        let turn = wind_turn([self.to[0], self.to[1]], self.target);
        self.retarget(scenario, turned(profile(scenario), turn, 1.0), duration_s);
    }

    fn retarget(&mut self, scenario: WeatherScenario, to: Profile, duration_s: f64) {
        self.from = self.current_profile();
        self.target = scenario;
        self.to = to;
        self.transition_elapsed_s = 0.0;
        self.transition_duration_s = if duration_s.is_finite() {
            duration_s.max(0.0)
        } else {
            0.0
        };
        if self.transition_duration_s == 0.0 {
            self.from = to;
        }
    }

    /// Advances by `delta_s` speed-scaled seconds. Zero or less is a no-op, which is pause.
    ///
    /// The wind ramp of a transition is integrated analytically, so one long step and many short
    /// ones give the same drift up to rounding.
    pub fn advance(&mut self, delta_s: f64) {
        if delta_s.is_nan() || delta_s <= 0.0 {
            return;
        }
        let target = self.to;
        let remaining_s = (self.transition_duration_s - self.transition_elapsed_s).max(0.0);
        let ramp_s = delta_s.min(remaining_s);
        let blend_integral_s = if ramp_s > 0.0 {
            let a = self.transition_elapsed_s / self.transition_duration_s;
            let b = (self.transition_elapsed_s + ramp_s) / self.transition_duration_s;
            self.transition_duration_s * (smooth_integral(b) - smooth_integral(a))
        } else {
            0.0
        };
        let air: [f64; 2] = std::array::from_fn(|axis| {
            self.from[axis] * ramp_s
                + (target[axis] - self.from[axis]) * blend_integral_s
                + target[axis] * (delta_s - ramp_s)
        });
        // The high wind is the low one veered and scaled, both constant, so its drift is the
        // same rotation of the air's displacement.
        let (sin, cos) = HIGH_WIND_VEER_RAD.sin_cos();
        let high = [air[0] * cos - air[1] * sin, air[0] * sin + air[1] * cos];
        for axis in 0..2 {
            self.cloud_offset_m[axis] = (self.cloud_offset_m[axis]
                + air[axis] * CLOUD_WIND_MULTIPLIER)
                .rem_euclid(WEATHER_PERIOD_M);
            self.high_offset_m[axis] = (self.high_offset_m[axis]
                + high[axis] * HIGH_WIND_MULTIPLIER)
                .rem_euclid(WEATHER_PERIOD_M);
        }
        self.transition_elapsed_s += ramp_s;
        if self.transition_elapsed_s >= self.transition_duration_s {
            self.from = target;
            self.transition_elapsed_s = 0.0;
            self.transition_duration_s = 0.0;
        }
    }

    /// Whether the seeded schedule moves the weather.
    pub fn automatic(&self) -> bool {
        self.automatic
    }

    /// Hands the weather to the seeded schedule, whose next change comes after the current
    /// scenario's dwell, or takes it away so a chosen scenario holds.
    pub fn set_automatic(&mut self, automatic: bool) {
        if automatic && !self.automatic {
            self.until_event_h = self.dwell_hours(self.target);
        }
        self.automatic = automatic;
    }

    /// Moves to a scenario the player chose and holds it: the schedule stops.
    pub fn choose(&mut self, scenario: WeatherScenario, duration_s: f64) {
        self.automatic = false;
        self.transition_to(scenario, duration_s);
    }

    /// Advances by `delta_s` speed-scaled seconds, an in-game hour being `seconds_per_hour` of
    /// them, and makes every scheduled change that falls inside the step at its exact moment.
    /// Zero or less is a no-op, which is pause.
    pub fn advance_scheduled(&mut self, delta_s: f64, seconds_per_hour: f64) {
        if delta_s.is_nan() || delta_s <= 0.0 {
            return;
        }
        if !self.automatic || !seconds_per_hour.is_finite() || seconds_per_hour <= 0.0 {
            self.advance(delta_s);
            return;
        }
        let mut left_s = delta_s;
        loop {
            let until_s = self.until_event_h * seconds_per_hour;
            if until_s > left_s {
                self.advance(left_s);
                self.until_event_h -= left_s / seconds_per_hour;
                return;
            }
            self.advance(until_s);
            left_s -= until_s;
            self.change(seconds_per_hour);
        }
    }

    // One scheduled change: the next scenario by weight from the current one, the blend and the
    // dwell after it, each from its own draw.
    fn change(&mut self, seconds_per_hour: f64) {
        let event = self.event_index;
        let from = self.target.index() as usize;
        let mut pick = draw(self.seed, event, 0);
        let mut next = self.target;
        for (scenario, weight) in WeatherScenario::ALL.iter().zip(NEXT_WEIGHTS[from]) {
            if pick < weight {
                next = *scenario;
                break;
            }
            pick -= weight;
        }
        let [low, high] = CHANGE_MINUTES[next.index() as usize];
        let minutes = low + (high - low) * draw(self.seed, event, 1);
        let to = self.scheduled_profile(next, event);
        self.retarget(next, to, minutes / 60.0 * seconds_per_hour);
        let [low, high] = DWELL_HOURS[next.index() as usize];
        self.until_event_h = low + (high - low) * draw(self.seed, event, 2);
        self.event_index = event.wrapping_add(1);
    }

    // The profile a scheduled change moves to: the scenario's own, with the wind turned from
    // where it blows now and pulled back towards the prevailing direction, its speed rescaled,
    // and the high cover drawn. Draws 3 to 5 of the event; 0 to 2 pick the scenario and timing.
    fn scheduled_profile(&self, next: WeatherScenario, event: u32) -> Profile {
        let turn = wind_turn([self.to[0], self.to[1]], self.target)
            + (draw(self.seed, event, 3) * 2.0 - 1.0) * WIND_TURN_MAX_RAD;
        let turn = wrap_angle(turn) * (1.0 - PREVAILING_PULL);
        let [slow, fast] = WIND_SPEED_RANGE;
        let speed = slow + (fast - slow) * draw(self.seed, event, 4);
        let mut to = turned(profile(next), turn, speed);
        let [low, high] = HIGH_COVER_RANGE[next.index() as usize];
        to[4] = low + (high - low) * draw(self.seed, event, 5);
        to
    }

    // The middle of a scenario's dwell range, for a schedule restarted by the player.
    fn dwell_hours(&self, scenario: WeatherScenario) -> f64 {
        let [low, high] = DWELL_HOURS[scenario.index() as usize];
        0.5 * (low + high)
    }

    /// Advances the wind clock by `delta_s` seconds. The caller passes frame time capped at
    /// normal speed: sway and ripples stop with a paused game, but a time-lapse would only shake
    /// every tree, so above normal speed they keep their normal pace. Zero or less is a no-op.
    pub fn advance_wind(&mut self, delta_s: f64) {
        if delta_s > 0.0 {
            self.wind_clock_s = (self.wind_clock_s + delta_s).rem_euclid(WIND_PERIOD_S);
        }
    }

    /// The state as a save stores it.
    pub(crate) fn record(&self) -> WeatherRecord {
        WeatherRecord {
            seed: self.seed,
            target: self.target.index(),
            from: self.from,
            to_wind: [self.to[0], self.to[1]],
            to_high_cover: self.to[4],
            transition_elapsed_s: self.transition_elapsed_s,
            transition_duration_s: self.transition_duration_s,
            cloud_offset_m: self.cloud_offset_m,
            high_offset_m: self.high_offset_m,
            wind_clock_s: self.wind_clock_s,
            automatic: self.automatic,
            event_index: self.event_index,
            until_event_h: self.until_event_h,
        }
    }

    /// Restores a saved state; `None` when the record could not have been written by
    /// [`Self::record`]: an unknown scenario, a non-finite value, a negative or overrun
    /// transition, a drift or wind clock outside its wrap period, or a starting profile no blend
    /// of the authored scenarios can reach.
    pub(crate) fn from_record(record: WeatherRecord) -> Option<Self> {
        let target = WeatherScenario::from_index(record.target)?;
        let authored = profile(target);
        let to = [
            record.to_wind[0],
            record.to_wind[1],
            authored[2],
            authored[3],
            record.to_high_cover,
        ];
        let finite =
            record.transition_elapsed_s.is_finite() && record.transition_duration_s.is_finite();
        let transition = record.transition_duration_s >= 0.0
            && (0.0..=record.transition_duration_s).contains(&record.transition_elapsed_s);
        let wrapped = record
            .cloud_offset_m
            .iter()
            .chain(&record.high_offset_m)
            .all(|v| (0.0..WEATHER_PERIOD_M).contains(v))
            && (0.0..WIND_PERIOD_S).contains(&record.wind_clock_s);
        let schedule = record.until_event_h.is_finite() && record.until_event_h >= 0.0;
        // Every reachable profile is a blend of two reachable ones, so it lies within their range.
        let profiles = reachable(record.from) && reachable(to);
        (finite && transition && wrapped && schedule && profiles).then_some(Self {
            seed: record.seed,
            target,
            from: record.from,
            to,
            transition_elapsed_s: record.transition_elapsed_s,
            transition_duration_s: record.transition_duration_s,
            cloud_offset_m: record.cloud_offset_m,
            high_offset_m: record.high_offset_m,
            wind_clock_s: record.wind_clock_s,
            automatic: record.automatic,
            event_index: record.event_index,
            until_event_h: record.until_event_h,
        })
    }

    /// Render values for the current state. Does not advance anything.
    pub fn sample(&self) -> WeatherSample {
        let current = self.current_profile();
        let cloud_origin_m = std::array::from_fn(|axis| {
            let seeded = f64::from(hash(axis as i32, 0, self.seed) & 0xffff);
            (seeded - self.cloud_offset_m[axis]).rem_euclid(WEATHER_PERIOD_M) as f32
        });
        let high_origin_m = std::array::from_fn(|axis| {
            let seeded = f64::from(hash(axis as i32, 1, self.seed) & 0xffff);
            (seeded - self.high_offset_m[axis]).rem_euclid(WEATHER_PERIOD_M) as f32
        });
        // A blend between two turned winds can pass through calm; the direction is then the one
        // the wind is turning to.
        let mut wind = [current[0], current[1]];
        if wind[0].hypot(wind[1]) < 1.0e-3 {
            wind = [self.to[0], self.to[1]];
        }
        let speed = wind[0].hypot(wind[1]).max(1.0e-9);
        WeatherSample {
            cloud_origin_m,
            cloud_coverage: current[2] as f32,
            cloud_optical_depth: current[3] as f32,
            wind_clock_s: self.wind_clock_s as f32,
            wind_strength: (current[0].hypot(current[1]) / AUTHORED_WIND_MS) as f32,
            // The clouds drift at exactly twice the wind, so halving their wrapped drift gives
            // the wind's, wrapped at half the period.
            wind_offset_m: std::array::from_fn(|axis| {
                (self.cloud_offset_m[axis] / CLOUD_WIND_MULTIPLIER) as f32
            }),
            high_origin_m,
            high_cover: current[4] as f32,
            wind_direction: [(wind[0] / speed) as f32, (wind[1] / speed) as f32],
            automatic: self.automatic,
            rain: rain(current[2], current[3]) as f32,
        }
    }
}

fn rain(coverage: f64, optical_depth: f64) -> f64 {
    smoothstep(RAIN_COVERAGE, coverage) * smoothstep(RAIN_DEPTH, optical_depth)
}

fn smoothstep(edges: [f64; 2], value: f64) -> f64 {
    let t = ((value - edges[0]) / (edges[1] - edges[0])).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_falls_from_storms_only_and_ramps_with_the_transition() {
        let settled = |scenario| {
            let mut state = WeatherState::new(7);
            state.transition_to(scenario, 0.0);
            state.sample().rain
        };
        assert_eq!(settled(WeatherScenario::Clear), 0.0);
        assert_eq!(settled(WeatherScenario::FairCumulus), 0.0);
        assert_eq!(settled(WeatherScenario::Overcast), 0.0);
        assert_eq!(settled(WeatherScenario::Storm), 1.0);
        let steady = settled(WeatherScenario::Rain);
        assert!(steady > 0.4 && steady < 0.8, "steady rain was {steady}");
        let mut state = WeatherState::new(7);
        state.transition_to(WeatherScenario::Overcast, 0.0);
        state.transition_to(WeatherScenario::Storm, 600.0);
        let mut previous = state.sample().rain;
        assert_eq!(previous, 0.0);
        for _ in 0..60 {
            state.advance(10.0);
            let rain = state.sample().rain;
            assert!(
                rain >= previous,
                "rain fell from {previous} to {rain} on the way to a storm"
            );
            previous = rain;
        }
        assert_eq!(previous, 1.0);
    }

    fn drift(state: &WeatherState) -> [f64; 2] {
        state.cloud_offset_m
    }

    #[test]
    fn pause_is_a_no_op_and_replay_is_exact() {
        let mut first = WeatherState::new(42);
        let mut second = WeatherState::new(42);
        for state in [&mut first, &mut second] {
            state.transition_to(WeatherScenario::Storm, 120.0);
            for delta in [0.5, 0.0, -1.0, 32.0 / 60.0, 125.0] {
                state.advance(delta);
            }
        }
        assert_eq!(first, second);
        let before = first.clone();
        first.advance(0.0);
        first.advance(f64::NAN);
        assert_eq!(first, before);
    }

    #[test]
    fn seed_moves_the_cloud_origin() {
        assert_ne!(
            WeatherState::new(42).sample().cloud_origin_m,
            WeatherState::new(43).sample().cloud_origin_m
        );
    }

    #[test]
    fn settled_wind_drifts_clouds_at_twice_its_speed() {
        let mut state = WeatherState::new(0);
        state.advance(10.0);
        assert_eq!(drift(&state), [80.0, 30.0]);
    }

    #[test]
    fn transition_drift_is_the_integral_of_the_ramp_however_it_is_stepped() {
        let mut whole = WeatherState::default();
        whole.transition_to(WeatherScenario::Storm, 10.0);
        let mut split = whole.clone();
        whole.advance(15.0);
        for _ in 0..30 {
            split.advance(0.5);
        }
        // 4 -> 14 m/s over 10 s averages 9 m/s, then 5 s at 14 m/s: 160 m of wind, 320 m of cloud.
        assert!((drift(&whole)[0] - 320.0).abs() < 1e-9);
        for axis in 0..2 {
            assert!((drift(&whole)[axis] - drift(&split)[axis]).abs() < 1e-9);
        }
        assert_eq!(whole.sample().cloud_coverage, 1.0);
        assert_eq!(whole.target(), WeatherScenario::Storm);
    }

    #[test]
    fn retargeting_mid_transition_does_not_jump() {
        let mut state = WeatherState::default();
        state.transition_to(WeatherScenario::Clear, 20.0);
        state.advance(4.0);
        let before = state.sample();
        state.transition_to(WeatherScenario::Overcast, 20.0);
        assert_eq!(before, state.sample());
    }

    #[test]
    fn instant_transition_settles_on_the_target() {
        let mut state = WeatherState::default();
        state.transition_to(WeatherScenario::Overcast, 0.0);
        assert_eq!(state.sample().cloud_coverage, 1.0);
        state.advance(1.0);
        assert_eq!(drift(&state), [12.0, 4.0]);
    }

    #[test]
    fn offsets_wrap_inside_the_period() {
        let mut state = WeatherState::default();
        state.advance(WEATHER_PERIOD_M);
        for axis in 0..2 {
            assert!((0.0..WEATHER_PERIOD_M).contains(&drift(&state)[axis]));
            let origin = f64::from(state.sample().cloud_origin_m[axis]);
            assert!((0.0..=WEATHER_PERIOD_M).contains(&origin));
        }
    }

    #[test]
    fn record_round_trips_mid_transition_and_rejects_impossible_states() {
        let mut state = WeatherState::new(7);
        state.transition_to(WeatherScenario::Storm, 90.0);
        state.advance(31.5);
        let record = state.record();
        assert_eq!(WeatherState::from_record(record), Some(state));
        let invalid = [
            WeatherRecord {
                target: 9,
                ..record
            },
            WeatherRecord {
                from: [f64::NAN, 0.0, 0.0, 0.0, 0.0],
                ..record
            },
            WeatherRecord {
                transition_elapsed_s: 91.0,
                ..record
            },
            WeatherRecord {
                transition_duration_s: -1.0,
                transition_elapsed_s: 0.0,
                ..record
            },
            WeatherRecord {
                cloud_offset_m: [WEATHER_PERIOD_M, 0.0],
                ..record
            },
            WeatherRecord {
                cloud_offset_m: [-1.0, 0.0],
                ..record
            },
            WeatherRecord {
                wind_clock_s: WIND_PERIOD_S,
                ..record
            },
            WeatherRecord {
                from: [4.0, 1.5, 1.2, 1.2, 0.2],
                ..record
            },
            WeatherRecord {
                from: [4.0, 1.5, 0.45, -0.5, 0.2],
                ..record
            },
            WeatherRecord {
                from: [1.0e30, 1.5, 0.45, 1.2, 0.2],
                ..record
            },
            WeatherRecord {
                from: [4.0, 1.5, 0.45, 1.2, -0.1],
                ..record
            },
            WeatherRecord {
                to_high_cover: 1.5,
                ..record
            },
            WeatherRecord {
                to_wind: [f64::INFINITY, 0.0],
                ..record
            },
            WeatherRecord {
                high_offset_m: [0.0, WEATHER_PERIOD_M],
                ..record
            },
        ];
        for record in invalid {
            assert_eq!(WeatherState::from_record(record), None, "{record:?}");
        }
    }

    #[test]
    fn wind_clock_wraps_and_the_air_moves_at_half_the_cloud_drift() {
        let mut state = WeatherState::default();
        state.advance(10.0);
        state.advance_wind(2.5);
        let sample = state.sample();
        assert_eq!(sample.wind_clock_s, 2.5);
        assert_eq!(sample.wind_offset_m, [40.0, 15.0]);
        assert!((sample.wind_strength - 1.0).abs() < 1e-6);
        state.advance_wind(WIND_PERIOD_S);
        assert!((f64::from(state.sample().wind_clock_s) - 2.5).abs() < 1e-3);
        let before = state.clone();
        state.advance_wind(0.0);
        state.advance_wind(f64::NAN);
        assert_eq!(state, before);
        state.transition_to(WeatherScenario::Storm, 0.0);
        assert!(state.sample().wind_strength > 3.0);
    }

    // The scenario sequence and the moment of each change, sampled once per in-game hour.
    fn schedule_trace(steps_per_hour: usize, hours: usize) -> Vec<(u8, u32)> {
        let mut state = WeatherState::new(5);
        let step = 60.0 / steps_per_hour as f64;
        (0..hours)
            .map(|_| {
                for _ in 0..steps_per_hour {
                    state.advance_scheduled(step, 60.0);
                }
                (state.target().index(), state.event_index)
            })
            .collect()
    }

    #[test]
    fn schedule_is_the_same_however_the_time_is_stepped() {
        let fine = schedule_trace(240, 96);
        assert_eq!(fine, schedule_trace(1, 96));
        // Four days hold several changes, and every scenario a change can reach is reached.
        assert!(fine.last().unwrap().1 >= 8);
        let mut seen = [false; WeatherScenario::ALL.len()];
        for (scenario, _) in &fine {
            seen[*scenario as usize] = true;
        }
        assert!(seen.iter().filter(|s| **s).count() >= 3);
    }

    #[test]
    fn schedule_moves_to_neighbours_and_a_choice_stops_it() {
        let mut state = WeatherState::new(9);
        let mut previous = state.target();
        // Half-hour steps, shorter than the shortest dwell, so each step holds at most one change.
        for _ in 0..4000 {
            state.advance_scheduled(30.0, 60.0);
            let target = state.target();
            if target != previous {
                assert!(NEXT_WEIGHTS[previous.index() as usize][target.index() as usize] > 0.0);
                previous = target;
            }
        }
        state.choose(WeatherScenario::Storm, 0.0);
        let events = state.event_index;
        state.advance_scheduled(1.0e6, 60.0);
        assert_eq!(
            (state.target(), state.event_index),
            (WeatherScenario::Storm, events)
        );
        state.set_automatic(true);
        state.advance_scheduled(1.0e6, 60.0);
        assert!(state.event_index > events);
    }

    #[test]
    fn high_clouds_drift_on_a_faster_veered_wind() {
        let mut state = WeatherState::new(0);
        state.advance(10.0);
        // 40 m by 15 m of air, veered 35 degrees clockwise seen from above and four times as far.
        let (sin, cos) = HIGH_WIND_VEER_RAD.sin_cos();
        let expected = [
            (40.0 * cos - 15.0 * sin) * 4.0,
            (40.0 * sin + 15.0 * cos) * 4.0,
        ];
        for (axis, expected) in expected.iter().enumerate() {
            let drift = state.high_offset_m[axis];
            let wrapped = expected.rem_euclid(WEATHER_PERIOD_M);
            assert!((drift - wrapped).abs() < 1e-9, "{drift} {wrapped}");
        }
        assert_ne!(state.sample().high_origin_m, state.sample().cloud_origin_m);
        assert_eq!(state.sample().high_cover, 0.2);
    }

    #[test]
    fn schedule_turns_and_retunes_the_wind_around_the_prevailing_direction() {
        let mut state = WeatherState::new(3);
        let mut turns = Vec::new();
        let mut speeds = Vec::new();
        let mut covers = Vec::new();
        for _ in 0..400 {
            let events = state.event_index;
            state.advance_scheduled(30.0, 60.0);
            if state.event_index != events {
                turns.push(wind_turn([state.to[0], state.to[1]], state.target));
                let authored = profile(state.target);
                speeds.push(state.to[0].hypot(state.to[1]) / authored[0].hypot(authored[1]));
                covers.push(state.to[4]);
            }
        }
        assert!(turns.len() >= 20);
        // The walk wanders both ways but the pull keeps it within the turn it can reach.
        let bound = WIND_TURN_MAX_RAD * (1.0 - PREVAILING_PULL) / PREVAILING_PULL;
        assert!(turns.iter().all(|t| t.abs() <= bound + 1e-9));
        assert!(turns.iter().any(|t| *t > 0.3) && turns.iter().any(|t| *t < -0.3));
        assert!(
            speeds
                .iter()
                .all(|s| (0.75 - 1e-9..=1.3 + 1e-9).contains(s))
        );
        assert!(speeds.iter().any(|s| *s < 0.9) && speeds.iter().any(|s| *s > 1.15));
        assert!(covers.iter().any(|c| *c < 0.1) && covers.iter().any(|c| *c > 0.4));
        // A choice keeps the direction the weather turned the wind to.
        let turn = wind_turn([state.to[0], state.to[1]], state.target);
        state.choose(WeatherScenario::Storm, 0.0);
        let after = wind_turn([state.to[0], state.to[1]], WeatherScenario::Storm);
        assert!((after - turn).abs() < 1e-9);
        let direction = state.sample().wind_direction;
        assert!((f32::hypot(direction[0], direction[1]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn scenario_index_round_trips() {
        for scenario in WeatherScenario::ALL {
            assert_eq!(
                WeatherScenario::from_index(scenario.index()),
                Some(scenario)
            );
        }
        assert_eq!(WeatherScenario::from_index(9), None);
    }
}
