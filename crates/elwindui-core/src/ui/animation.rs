//! Backend-neutral animation primitives and the synchronous transaction scope.
//!
//! Animation state is deliberately independent from any frame source. Hosts may use these
//! primitives from a per-tree runtime, while tests can sample them with explicit monotonic times.

use crate::base::{AffineTransform, Size, Vector};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Animation {
    Linear {
        duration: Duration,
    },
    EaseIn {
        duration: Duration,
    },
    EaseOut {
        duration: Duration,
    },
    EaseInOut {
        duration: Duration,
    },
    Spring {
        response: Duration,
        damping_ratio: f32,
    },
}

impl Animation {
    pub const fn linear(duration: Duration) -> Self {
        Self::Linear { duration }
    }

    pub const fn ease_in(duration: Duration) -> Self {
        Self::EaseIn { duration }
    }

    pub const fn ease_out(duration: Duration) -> Self {
        Self::EaseOut { duration }
    }

    pub const fn ease_in_out(duration: Duration) -> Self {
        Self::EaseInOut { duration }
    }

    pub fn spring(response: Duration, damping_ratio: f32) -> Self {
        assert!(
            damping_ratio.is_finite() && damping_ratio > 0.0,
            "Animation::spring damping_ratio must be finite and strictly positive"
        );
        Self::Spring {
            response,
            damping_ratio,
        }
    }

    pub fn duration(self) -> Option<Duration> {
        match self {
            Self::Linear { duration }
            | Self::EaseIn { duration }
            | Self::EaseOut { duration }
            | Self::EaseInOut { duration } => Some(duration),
            Self::Spring { response, .. } => Some(response),
        }
    }

    /// Samples a normalized scalar trajectory and its derivative at elapsed monotonic time.
    /// `velocity` is in value units per second and is used by spring retargeting.
    pub fn sample(self, start: f32, target: f32, velocity: f32, elapsed: Duration) -> (f32, f32) {
        match self {
            Self::Linear { duration } => sample_timed(start, target, elapsed, duration, |t| t),
            Self::EaseIn { duration } => sample_timed(start, target, elapsed, duration, |t| t * t),
            Self::EaseOut { duration } => sample_timed(start, target, elapsed, duration, |t| {
                1.0 - (1.0 - t) * (1.0 - t)
            }),
            Self::EaseInOut { duration } => sample_timed(start, target, elapsed, duration, |t| {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }),
            Self::Spring {
                response,
                damping_ratio,
            } => sample_spring(start, target, velocity, elapsed, response, damping_ratio),
        }
    }

    pub fn is_immediate(self) -> bool {
        self.duration().is_some_and(|duration| duration.is_zero())
    }
}

fn sample_timed(
    start: f32,
    target: f32,
    elapsed: Duration,
    duration: Duration,
    curve: impl Fn(f32) -> f32 + Copy,
) -> (f32, f32) {
    if duration.is_zero() {
        return (target, 0.0);
    }
    let total = duration.as_secs_f64();
    let seconds = elapsed.as_secs_f64();
    let t = (seconds / total).clamp(0.0, 1.0) as f32;
    let p = curve(t);
    let value = start + (target - start) * p;
    let velocity = if t >= 1.0 {
        0.0
    } else {
        let h = 1.0e-4_f32;
        let before = (t - h).max(0.0);
        let after = (t + h).min(1.0);
        let derivative = if after > before {
            (curve(after) - curve(before)) / (after - before)
        } else {
            0.0
        };
        (target - start) * derivative / total as f32
    };
    (value, velocity)
}

fn sample_spring(
    start: f32,
    target: f32,
    velocity: f32,
    elapsed: Duration,
    response: Duration,
    damping_ratio: f32,
) -> (f32, f32) {
    if response.is_zero() {
        return (target, 0.0);
    }
    let t = elapsed.as_secs_f64() as f32;
    let omega = std::f32::consts::TAU / response.as_secs_f32();
    let zeta = damping_ratio;
    let x0 = start - target;

    let (x, v) = if zeta < 1.0 {
        let wd = omega * (1.0 - zeta * zeta).sqrt();
        let envelope = (-zeta * omega * t).exp();
        let c = (velocity + zeta * omega * x0) / wd;
        let cos = (wd * t).cos();
        let sin = (wd * t).sin();
        let x = envelope * (x0 * cos + c * sin);
        let v =
            envelope * (-(zeta * omega) * (x0 * cos + c * sin) + (-x0 * wd * sin + c * wd * cos));
        (x, v)
    } else if zeta == 1.0 {
        let envelope = (-omega * t).exp();
        let c = velocity + omega * x0;
        let x = envelope * (x0 + c * t);
        let v = envelope * (c - omega * (x0 + c * t));
        (x, v)
    } else {
        let root = (zeta * zeta - 1.0).sqrt();
        let r1 = -omega * (zeta - root);
        let r2 = -omega * (zeta + root);
        let c2 = (velocity - r1 * x0) / (r2 - r1);
        let c1 = x0 - c2;
        let e1 = (r1 * t).exp();
        let e2 = (r2 * t).exp();
        (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
    };
    (target + x, v)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transaction {
    pub animation: Option<Animation>,
    pub disables_animations: bool,
}

thread_local! {
    static TRANSACTION_STACK: RefCell<Vec<Transaction>> = const { RefCell::new(Vec::new()) };
}

pub fn current_transaction() -> Transaction {
    TRANSACTION_STACK.with(|stack| stack.borrow().last().copied().unwrap_or_default())
}

pub fn with_transaction<R>(transaction: Transaction, body: impl FnOnce() -> R) -> R {
    let inherited = current_transaction();
    let effective = Transaction {
        animation: transaction.animation.or(inherited.animation),
        disables_animations: inherited.disables_animations || transaction.disables_animations,
    };
    TRANSACTION_STACK.with(|stack| stack.borrow_mut().push(effective));
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            TRANSACTION_STACK.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }
    let _pop = Pop;
    body()
}

pub fn with_animation<R>(animation: Animation, body: impl FnOnce() -> R) -> R {
    with_transaction(
        Transaction {
            animation: Some(animation),
            disables_animations: false,
        },
        body,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnimationChannel {
    Opacity,
    VisualTransform,
    TransitionOpacity,
    TransitionVisualTransform,
    Margin,
    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimatedValue {
    Scalar(f32),
    Transform(VisualTransform),
}

impl AnimatedValue {
    fn sample(
        self,
        target: Self,
        velocity: Self,
        animation: Animation,
        elapsed: Duration,
    ) -> (Self, Self) {
        match (self, target, velocity) {
            (Self::Scalar(start), Self::Scalar(target), Self::Scalar(velocity)) => {
                let (value, velocity) = animation.sample(start, target, velocity, elapsed);
                (Self::Scalar(value), Self::Scalar(velocity))
            }
            (Self::Transform(start), Self::Transform(target), Self::Transform(velocity)) => {
                let (translation_x, velocity_x) = animation.sample(
                    start.translation.x,
                    target.translation.x,
                    velocity.translation.x,
                    elapsed,
                );
                let (translation_y, velocity_y) = animation.sample(
                    start.translation.y,
                    target.translation.y,
                    velocity.translation.y,
                    elapsed,
                );
                let (scale, scale_velocity) =
                    animation.sample(start.scale, target.scale, velocity.scale, elapsed);
                let (rotation, rotation_velocity) =
                    animation.sample(start.rotation, target.rotation, velocity.rotation, elapsed);
                (
                    Self::Transform(VisualTransform {
                        translation: Vector {
                            x: translation_x,
                            y: translation_y,
                        },
                        scale,
                        rotation,
                    }),
                    Self::Transform(VisualTransform {
                        translation: Vector {
                            x: velocity_x,
                            y: velocity_y,
                        },
                        scale: scale_velocity,
                        rotation: rotation_velocity,
                    }),
                )
            }
            _ => panic!("animation channel value kinds must remain stable"),
        }
    }

    fn settled(
        self,
        target: Self,
        velocity: Self,
        animation: Animation,
        elapsed: Duration,
    ) -> bool {
        if animation.is_immediate() {
            return true;
        }
        match animation {
            Animation::Linear { duration }
            | Animation::EaseIn { duration }
            | Animation::EaseOut { duration }
            | Animation::EaseInOut { duration } => elapsed >= duration,
            Animation::Spring { .. } => match (self, target, velocity) {
                (Self::Scalar(value), Self::Scalar(target), Self::Scalar(velocity)) => {
                    (value - target).abs() < 1.0e-3 && velocity.abs() < 1.0e-3
                }
                (Self::Transform(value), Self::Transform(target), Self::Transform(velocity)) => {
                    (value.translation.x - target.translation.x).abs() < 1.0e-3
                        && (value.translation.y - target.translation.y).abs() < 1.0e-3
                        && (value.scale - target.scale).abs() < 1.0e-3
                        && (value.rotation - target.rotation).abs() < 1.0e-3
                        && velocity.translation.x.abs() < 1.0e-3
                        && velocity.translation.y.abs() < 1.0e-3
                        && velocity.scale.abs() < 1.0e-3
                        && velocity.rotation.abs() < 1.0e-3
                }
                _ => false,
            },
        }
    }
}

/// Runtime-internal channel key. Public callers address [`AnimationChannel`]s; the layout reflow
/// translation is a private channel so the public enum keeps its variant set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RuntimeChannel {
    Public(AnimationChannel),
    LayoutReflow,
}

type ChannelKey = (u64, RuntimeChannel);

/// How the next layout-consuming reconcile treats elements whose arranged position changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LayoutReflowIntent {
    Animate(Animation),
    Snap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PendingLayoutReflow {
    intent: LayoutReflowIntent,
    /// Set once `layout_root` has resolved the target layout after the latest structural change.
    armed: bool,
}

/// A detached channel's state at the current `tick` time, captured before any callback runs so a
/// callback may retarget any channel from its exact current value and velocity, independent of
/// callback order.
struct InFlightSample {
    generation: u64,
    value: AnimatedValue,
    /// The sampled (current) velocity at the tick time, not the trajectory's initial velocity.
    velocity: AnimatedValue,
}

struct ActiveAnimation {
    start: AnimatedValue,
    target: AnimatedValue,
    velocity: AnimatedValue,
    animation: Animation,
    started_at: Duration,
    value: AnimatedValue,
    generation: u64,
    callback: Box<dyn Fn(AnimatedValue, bool)>,
}

/// Deterministic, per-host animation state. A host owns one instance for its hosted tree and
/// provides explicit monotonic timestamps through [`Self::tick`].
pub struct AnimationRuntime {
    channels: RefCell<HashMap<ChannelKey, ActiveAnimation>>,
    /// The generation of the newest live channel per key. A channel taken out by `tick` is put back
    /// only if its generation is still current, so cancel/replace from a callback always wins.
    generations: RefCell<HashMap<ChannelKey, u64>>,
    next_generation: Cell<u64>,
    pending_layout_reflow: Cell<Option<PendingLayoutReflow>>,
    /// Elements whose layout reflow translation is driven by a live channel, so host
    /// unregistration can reset them even while their channel is detached by `tick`.
    layout_reflow_targets: RefCell<HashMap<u64, Weak<dyn crate::ui::UIElementExt>>>,
    /// The hosted root that last started a reflow. Semantic refresh after a reflow tick goes
    /// through it, so it never depends on the lifetime of any individual reflowing child.
    layout_reflow_accessibility_root: RefCell<Option<Weak<dyn crate::ui::UIElementExt>>>,
    /// Detached channels' samples for the tick in progress; empty outside `tick`.
    in_flight_samples: RefCell<HashMap<ChannelKey, InFlightSample>>,
    ticking: Cell<bool>,
    #[cfg(test)]
    layout_reflow_intent_records: Cell<usize>,
    now: RefCell<Duration>,
    frame_requested: RefCell<bool>,
    epoch: Instant,
}

impl AnimationRuntime {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            channels: RefCell::new(HashMap::new()),
            generations: RefCell::new(HashMap::new()),
            next_generation: Cell::new(0),
            pending_layout_reflow: Cell::new(None),
            layout_reflow_targets: RefCell::new(HashMap::new()),
            layout_reflow_accessibility_root: RefCell::new(None),
            in_flight_samples: RefCell::new(HashMap::new()),
            ticking: Cell::new(false),
            #[cfg(test)]
            layout_reflow_intent_records: Cell::new(0),
            now: RefCell::new(Duration::ZERO),
            frame_requested: RefCell::new(false),
            epoch: Instant::now(),
        })
    }

    pub fn animate(
        &self,
        owner_id: u64,
        channel: AnimationChannel,
        current: AnimatedValue,
        target: AnimatedValue,
        animation: Animation,
        callback: Box<dyn Fn(AnimatedValue)>,
    ) {
        self.animate_with_completion(
            owner_id,
            channel,
            current,
            target,
            animation,
            Box::new(move |value, _finished| callback(value)),
        );
    }

    pub fn animate_with_completion(
        &self,
        owner_id: u64,
        channel: AnimationChannel,
        current: AnimatedValue,
        target: AnimatedValue,
        animation: Animation,
        callback: Box<dyn Fn(AnimatedValue, bool)>,
    ) {
        let key = (owner_id, RuntimeChannel::Public(channel));
        let (start, velocity) = self
            .sample_active(key)
            .unwrap_or((current, zero_velocity(current)));
        self.insert_channel(key, start, target, velocity, animation, callback);
    }

    /// Samples a live channel at the runtime's current monotonic time.
    ///
    /// A channel already (re)inserted into the live map wins. Otherwise, during a `tick`, the
    /// detached channel's pre-callback sample is used, but only while its generation is still
    /// current, so a cancelled or replaced channel never provides a starting value.
    fn sample_active(&self, key: ChannelKey) -> Option<(AnimatedValue, AnimatedValue)> {
        let now = *self.now.borrow();
        let live = self.channels.borrow().get(&key).map(|active| {
            let elapsed = now.saturating_sub(active.started_at);
            active
                .start
                .sample(active.target, active.velocity, active.animation, elapsed)
        });
        if live.is_some() {
            return live;
        }
        let generation = self.generations.borrow().get(&key).copied()?;
        self.in_flight_samples
            .borrow()
            .get(&key)
            .filter(|sample| sample.generation == generation)
            .map(|sample| (sample.value, sample.velocity))
    }

    fn insert_channel(
        &self,
        key: ChannelKey,
        start: AnimatedValue,
        target: AnimatedValue,
        velocity: AnimatedValue,
        animation: Animation,
        callback: Box<dyn Fn(AnimatedValue, bool)>,
    ) {
        let now = *self.now.borrow();
        let generation = self.next_generation.get().wrapping_add(1);
        self.next_generation.set(generation);
        self.generations.borrow_mut().insert(key, generation);
        self.channels.borrow_mut().insert(
            key,
            ActiveAnimation {
                start,
                target,
                velocity,
                animation,
                started_at: now,
                value: start,
                generation,
                callback,
            },
        );
        *self.frame_requested.borrow_mut() = true;
    }

    /// Records the structural reflow intent of an effective mutation. The latest intent wins and
    /// waits for the next completed layout before a reconcile may consume it.
    pub(crate) fn record_layout_reflow_intent(&self, intent: LayoutReflowIntent) {
        #[cfg(test)]
        self.layout_reflow_intent_records
            .set(self.layout_reflow_intent_records.get() + 1);
        self.pending_layout_reflow.set(Some(PendingLayoutReflow {
            intent,
            armed: false,
        }));
    }

    /// Marks the pending intent as covered by a completed Measure/Arrange pass.
    pub(crate) fn arm_layout_reflow_intent(&self) {
        if let Some(mut pending) = self.pending_layout_reflow.get() {
            pending.armed = true;
            self.pending_layout_reflow.set(Some(pending));
        }
    }

    /// Consumes the pending intent only when a completed layout already covers it.
    pub(crate) fn take_armed_layout_reflow_intent(&self) -> Option<LayoutReflowIntent> {
        match self.pending_layout_reflow.get() {
            Some(pending) if pending.armed => {
                self.pending_layout_reflow.set(None);
                Some(pending.intent)
            }
            _ => None,
        }
    }

    pub(crate) fn discard_layout_reflow_intent(&self) {
        self.pending_layout_reflow.set(None);
    }

    #[cfg(test)]
    pub(crate) fn pending_layout_reflow_intent(&self) -> Option<LayoutReflowIntent> {
        self.pending_layout_reflow
            .get()
            .map(|pending| pending.intent)
    }

    /// Starts or retargets the layout reflow translation of `owner_id` toward zero.
    ///
    /// `current` is the translation currently shown when no reflow channel is live; a live channel
    /// is sampled at the runtime's current time instead and its velocity is preserved. The new
    /// start is `current + delta`, where `delta` is the old minus the new layout origin, so the
    /// visible position is continuous. Returns the start translation the caller must publish
    /// synchronously, or `None` (after cancelling) when the result would not be finite.
    pub(crate) fn rebase_layout_reflow(
        &self,
        owner_id: u64,
        current: Vector,
        delta: Vector,
        animation: Animation,
        target: Option<Weak<dyn crate::ui::UIElementExt>>,
        callback: Box<dyn Fn(AnimatedValue, bool)>,
    ) -> Option<Vector> {
        let key = (owner_id, RuntimeChannel::LayoutReflow);
        let (sampled, velocity) = match self.sample_active(key) {
            Some((AnimatedValue::Transform(value), velocity)) => (value.translation, velocity),
            _ => (
                current,
                zero_velocity(AnimatedValue::Transform(VisualTransform::IDENTITY)),
            ),
        };
        let start = Vector {
            x: sampled.x + delta.x,
            y: sampled.y + delta.y,
        };
        let velocity_finite = match velocity {
            AnimatedValue::Transform(velocity) => {
                velocity.translation.x.is_finite() && velocity.translation.y.is_finite()
            }
            AnimatedValue::Scalar(velocity) => velocity.is_finite(),
        };
        if !start.x.is_finite() || !start.y.is_finite() || !velocity_finite {
            self.cancel_layout_reflow(owner_id);
            return None;
        }
        let start_value = AnimatedValue::Transform(VisualTransform {
            translation: start,
            scale: 1.0,
            rotation: 0.0,
        });
        self.insert_channel(
            key,
            start_value,
            AnimatedValue::Transform(VisualTransform::IDENTITY),
            velocity,
            animation,
            callback,
        );
        if let Some(target) = target {
            self.layout_reflow_targets
                .borrow_mut()
                .insert(owner_id, target);
        }
        Some(start)
    }

    /// Cancels the layout reflow of `owner_id` and resets its translation to zero without sending
    /// an invalidation. Safe to call from a `tick` callback: the detached channel is not
    /// reinserted, and its own callback sees [`Self::has_layout_reflow`] as `false`.
    pub(crate) fn cancel_layout_reflow(&self, owner_id: u64) -> bool {
        let key = (owner_id, RuntimeChannel::LayoutReflow);
        let had_generation = self.generations.borrow_mut().remove(&key).is_some();
        let had_channel = self.channels.borrow_mut().remove(&key).is_some();
        let target: Option<Rc<dyn crate::ui::UIElementExt>> = self
            .layout_reflow_targets
            .borrow_mut()
            .remove(&owner_id)
            .and_then(|target| target.upgrade());
        if let Some(target) = target {
            target
                .as_ui_element()
                .layout_reflow_translation
                .set(Vector::default());
        }
        had_generation || had_channel
    }

    /// Drops the pending reflow intent, every layout reflow channel, and the semantic refresh
    /// anchor, resetting the driven translations to zero. Other animation channels are untouched.
    ///
    /// Backends call this when a host stops presenting its tree (unregistration, or
    /// `set_active(false)` and the following `set_active(true)`), so an inactive host never keeps
    /// stale reflow displacement or frames. No invalidation is sent. Idempotent.
    ///
    /// An unconsumed frame request is cleared when no public channel remains live (including a
    /// channel detached by an in-progress `tick`, recognized through its generation); otherwise it
    /// is preserved. This never creates a frame request.
    #[doc(hidden)]
    pub fn discard_layout_reflows(&self) {
        self.pending_layout_reflow.set(None);
        *self.layout_reflow_accessibility_root.borrow_mut() = None;
        self.channels
            .borrow_mut()
            .retain(|key, _| key.1 != RuntimeChannel::LayoutReflow);
        self.generations
            .borrow_mut()
            .retain(|key, _| key.1 != RuntimeChannel::LayoutReflow);
        let targets = std::mem::take(&mut *self.layout_reflow_targets.borrow_mut());
        for target in targets.into_values() {
            let target: Option<Rc<dyn crate::ui::UIElementExt>> = target.upgrade();
            if let Some(target) = target {
                target
                    .as_ui_element()
                    .layout_reflow_translation
                    .set(Vector::default());
            }
        }
        let has_public_channel = self
            .generations
            .borrow()
            .keys()
            .any(|key| matches!(key.1, RuntimeChannel::Public(_)));
        if !has_public_channel {
            *self.frame_requested.borrow_mut() = false;
        }
    }

    /// Registers the hosted root whose accessibility host is refreshed after reflow ticks. Only a
    /// weak reference is kept; a newer root replaces an older one.
    pub(crate) fn set_layout_reflow_accessibility_root(
        &self,
        root: Weak<dyn crate::ui::UIElementExt>,
    ) {
        *self.layout_reflow_accessibility_root.borrow_mut() = Some(root);
    }

    #[cfg(test)]
    pub(crate) fn has_layout_reflow_accessibility_root(&self) -> bool {
        self.layout_reflow_accessibility_root.borrow().is_some()
    }

    #[cfg(test)]
    pub(crate) fn in_flight_sample_count(&self) -> usize {
        self.in_flight_samples.borrow().len()
    }

    #[cfg(test)]
    pub(crate) fn layout_reflow_intent_records(&self) -> usize {
        self.layout_reflow_intent_records.get()
    }

    /// Whether `owner_id` has a live (not cancelled, not finished) layout reflow channel.
    pub(crate) fn has_layout_reflow(&self, owner_id: u64) -> bool {
        self.generations
            .borrow()
            .contains_key(&(owner_id, RuntimeChannel::LayoutReflow))
    }

    #[cfg(test)]
    pub(crate) fn layout_reflow_count(&self) -> usize {
        self.generations
            .borrow()
            .keys()
            .filter(|key| key.1 == RuntimeChannel::LayoutReflow)
            .count()
    }

    /// Advances every channel to an explicit monotonic timestamp and returns whether work remains.
    ///
    /// Every detached channel is sampled at the same `now` before any callback runs. Callbacks run
    /// outside the channel borrow, so a callback may safely retarget or cancel any channel: a
    /// retarget starts from the pre-callback sample (value and current velocity), a channel
    /// cancelled or replaced earlier in this tick never receives its callback and is never
    /// reinserted, and a replacement first runs on the next tick. A reentrant call from inside a
    /// callback does not sample again; it only requests another frame.
    pub fn tick(&self, now: Duration) -> bool {
        if self.ticking.get() {
            *self.frame_requested.borrow_mut() = true;
            return true;
        }
        let previous = *self.now.borrow();
        let now = now.max(previous);
        *self.now.borrow_mut() = now;
        *self.frame_requested.borrow_mut() = false;

        /// Clears the in-flight snapshot and the reentrancy flag on every exit, including unwind.
        struct TickGuard<'a>(&'a AnimationRuntime);
        impl Drop for TickGuard<'_> {
            fn drop(&mut self) {
                self.0.in_flight_samples.borrow_mut().clear();
                self.0.ticking.set(false);
            }
        }
        self.ticking.set(true);
        let _guard = TickGuard(self);

        let current = std::mem::take(&mut *self.channels.borrow_mut());
        let mut sampled = Vec::with_capacity(current.len());
        {
            let mut in_flight = self.in_flight_samples.borrow_mut();
            for (key, active) in current {
                let elapsed = now.saturating_sub(active.started_at);
                let (value, velocity) =
                    active
                        .start
                        .sample(active.target, active.velocity, active.animation, elapsed);
                // `active.velocity` is the trajectory's initial velocity at `started_at` and must
                // stay fixed: overwriting it with the sampled velocity would bend every later
                // sample of the same trajectory. The sampled velocity decides settlement and is
                // what a retarget during this tick continues from.
                let finished = value.settled(active.target, velocity, active.animation, elapsed);
                in_flight.insert(
                    key,
                    InFlightSample {
                        generation: active.generation,
                        value,
                        velocity,
                    },
                );
                sampled.push((key, active, value, finished));
            }
        }
        let mut keep = Vec::new();
        let mut reflow_applied = false;
        for (key, mut active, value, finished) in sampled {
            let current_generation =
                self.generations.borrow().get(&key) == Some(&active.generation);
            if !current_generation {
                // Cancelled or replaced by an earlier callback of this tick.
                continue;
            }
            active.value = value;
            if key.1 == RuntimeChannel::LayoutReflow {
                reflow_applied = true;
            }
            (active.callback)(value, finished);
            if finished {
                let mut generations = self.generations.borrow_mut();
                if generations.get(&key) == Some(&active.generation) {
                    generations.remove(&key);
                    if key.1 == RuntimeChannel::LayoutReflow {
                        self.layout_reflow_targets.borrow_mut().remove(&key.0);
                    }
                }
            } else {
                keep.push((key, active));
            }
        }
        // A callback may have installed a replacement under the same key or cancelled the channel.
        // Only a channel whose generation is still current goes back; a replacement always wins.
        let active = {
            let generations = self.generations.borrow();
            let mut channels = self.channels.borrow_mut();
            for (key, active) in keep {
                if generations.get(&key) == Some(&active.generation) {
                    channels.entry(key).or_insert(active);
                }
            }
            !channels.is_empty()
        };
        *self.frame_requested.borrow_mut() = active;
        // Reflow moves presentation geometry on Render-only passes, which do not refresh semantic
        // bounds by themselves. One refresh per tick, through the hosted root, covers every
        // reflowing element of this runtime's tree, including the completion frame.
        if reflow_applied {
            let root: Option<Rc<dyn crate::ui::UIElementExt>> = self
                .layout_reflow_accessibility_root
                .borrow()
                .as_ref()
                .and_then(|root| root.upgrade());
            if let Some(root) = root {
                crate::ui::UIElementExt::request_accessibility_update(root.as_ref());
            }
            let any_reflow = self
                .generations
                .borrow()
                .keys()
                .any(|key| key.1 == RuntimeChannel::LayoutReflow);
            if !any_reflow {
                *self.layout_reflow_accessibility_root.borrow_mut() = None;
            }
        }
        active
    }

    pub fn tick_now(&self) -> bool {
        self.tick(self.epoch.elapsed())
    }

    /// Synchronizes the explicit clock with the host's monotonic wall clock without sampling any
    /// channels. Hosts call this immediately before starting a new animation after an idle gap.
    pub fn sync_now(&self) {
        let wall_clock = self.epoch.elapsed();
        let mut now = self.now.borrow_mut();
        *now = (*now).max(wall_clock);
    }

    pub fn take_frame_request(&self) -> bool {
        std::mem::replace(&mut *self.frame_requested.borrow_mut(), false)
    }

    pub fn is_idle(&self) -> bool {
        self.channels.borrow().is_empty()
    }
}

fn zero_velocity(value: AnimatedValue) -> AnimatedValue {
    match value {
        AnimatedValue::Scalar(_) => AnimatedValue::Scalar(0.0),
        AnimatedValue::Transform(_) => AnimatedValue::Transform(VisualTransform {
            translation: Vector::default(),
            scale: 0.0,
            rotation: 0.0,
        }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualTransform {
    pub translation: Vector,
    pub scale: f32,
    pub rotation: f32,
}

impl Default for VisualTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl VisualTransform {
    pub const IDENTITY: Self = Self {
        translation: Vector { x: 0.0, y: 0.0 },
        scale: 1.0,
        rotation: 0.0,
    };

    pub fn new(translation: Vector, scale: f32, rotation: f32) -> Self {
        assert!(
            translation.x.is_finite()
                && translation.y.is_finite()
                && scale.is_finite()
                && scale >= 0.0
                && rotation.is_finite(),
            "VisualTransform values must be finite and scale must be non-negative"
        );
        Self {
            translation,
            scale,
            rotation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitPoint {
    pub x: f32,
    pub y: f32,
}

impl UnitPoint {
    pub const CENTER: Self = Self { x: 0.5, y: 0.5 };
    pub const TOP: Self = Self { x: 0.5, y: 0.0 };
    pub const BOTTOM: Self = Self { x: 0.5, y: 1.0 };
    pub const LEADING: Self = Self { x: 0.0, y: 0.5 };
    pub const TRAILING: Self = Self { x: 1.0, y: 0.5 };
    pub const TOP_LEADING: Self = Self { x: 0.0, y: 0.0 };
    pub const TOP_TRAILING: Self = Self { x: 1.0, y: 0.0 };
    pub const BOTTOM_LEADING: Self = Self { x: 0.0, y: 1.0 };
    pub const BOTTOM_TRAILING: Self = Self { x: 1.0, y: 1.0 };

    pub fn new(x: f32, y: f32) -> Self {
        assert!(
            x.is_finite() && y.is_finite(),
            "UnitPoint coordinates must be finite"
        );
        Self { x, y }
    }
}

pub fn local_transform(
    transform: VisualTransform,
    origin: UnitPoint,
    size: Size,
) -> AffineTransform {
    let pivot = (size.width * origin.x, size.height * origin.y);
    AffineTransform::translation(transform.translation.x, transform.translation.y).concat(
        &AffineTransform::translation(pivot.0, pivot.1).concat(
            &AffineTransform::rotation(transform.rotation).concat(
                &AffineTransform::scale(transform.scale, transform.scale)
                    .concat(&AffineTransform::translation(-pivot.0, -pivot.1)),
            ),
        ),
    )
}

#[derive(Clone, Debug, PartialEq)]
pub enum Transition {
    Identity,
    Opacity,
    Scale(f32),
    Offset(Vector),
    Combined(Box<Transition>, Box<Transition>),
    Asymmetric {
        insertion: Box<Transition>,
        removal: Box<Transition>,
    },
}

impl Transition {
    pub const fn identity() -> Self {
        Self::Identity
    }

    pub const fn opacity() -> Self {
        Self::Opacity
    }

    pub fn scale(scale: f32) -> Self {
        assert!(
            scale.is_finite() && scale >= 0.0,
            "transition scale must be finite and non-negative"
        );
        Self::Scale(scale)
    }

    pub fn offset(offset: Vector) -> Self {
        assert!(
            offset.x.is_finite() && offset.y.is_finite(),
            "transition offset must be finite"
        );
        Self::Offset(offset)
    }

    pub fn combined(self, other: Self) -> Self {
        Self::Combined(Box::new(self), Box::new(other))
    }

    pub fn asymmetric(insertion: Self, removal: Self) -> Self {
        Self::Asymmetric {
            insertion: Box::new(insertion),
            removal: Box::new(removal),
        }
    }

    pub fn effect(&self, insertion: bool) -> (f32, VisualTransform) {
        match self {
            Self::Identity => (1.0, VisualTransform::IDENTITY),
            Self::Opacity => (0.0, VisualTransform::IDENTITY),
            Self::Scale(scale) => (1.0, VisualTransform::new(Vector::default(), *scale, 0.0)),
            Self::Offset(offset) => (1.0, VisualTransform::new(*offset, 1.0, 0.0)),
            Self::Combined(first, second) => {
                let (first_opacity, first_transform) = first.effect(insertion);
                let (second_opacity, second_transform) = second.effect(insertion);
                (
                    first_opacity * second_opacity,
                    compose_visual_transform(first_transform, second_transform),
                )
            }
            Self::Asymmetric {
                insertion: enter,
                removal: exit,
            } => {
                if insertion {
                    enter.effect(true)
                } else {
                    exit.effect(false)
                }
            }
        }
    }
}

pub fn compose_visual_transform(
    first: VisualTransform,
    second: VisualTransform,
) -> VisualTransform {
    let a = first.translation;
    let b = second.translation;
    let (sin_a, cos_a) = first.rotation.sin_cos();
    VisualTransform {
        translation: Vector {
            x: a.x + first.scale * (cos_a * b.x - sin_a * b.y),
            y: a.y + first.scale * (sin_a * b.x + cos_a * b.y),
        },
        scale: first.scale * second.scale,
        rotation: first.rotation + second.rotation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::Point;

    #[test]
    fn transaction_scope_restores_after_panic_safe_drop() {
        assert_eq!(current_transaction(), Transaction::default());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_animation(Animation::linear(Duration::from_millis(10)), || {
                assert!(current_transaction().animation.is_some());
                panic!("test");
            });
        }));
        assert!(result.is_err());
        assert_eq!(current_transaction(), Transaction::default());
    }

    #[test]
    fn inherited_disable_cannot_be_cleared() {
        with_transaction(
            Transaction {
                animation: None,
                disables_animations: true,
            },
            || {
                with_animation(Animation::linear(Duration::from_secs(1)), || {
                    let transaction = current_transaction();
                    assert!(transaction.disables_animations);
                    assert!(transaction.animation.is_some());
                });
            },
        );
    }

    #[test]
    fn transform_origin_round_trips() {
        let transform = local_transform(
            VisualTransform::new(Vector { x: 4.0, y: -2.0 }, 2.0, 0.25),
            UnitPoint::CENTER,
            Size {
                width: 20.0,
                height: 10.0,
            },
        );
        let inverse = transform.invert().expect("finite non-singular transform");
        let point = Point { x: 3.0, y: 4.0 };
        let round_trip = inverse.transform_point(transform.transform_point(point));
        assert!((round_trip.x - point.x).abs() < 1e-4);
        assert!((round_trip.y - point.y).abs() < 1e-4);
    }

    #[test]
    fn spring_is_deterministic_and_moves_toward_target() {
        let animation = Animation::spring(Duration::from_millis(300), 0.8);
        let first = animation.sample(0.0, 1.0, 0.0, Duration::from_millis(100));
        let second = animation.sample(0.0, 1.0, 0.0, Duration::from_millis(100));
        assert_eq!(first, second);
        assert!(first.0 > 0.0);
    }

    #[test]
    fn runtime_starts_from_presentation_and_retargets_from_sampled_value() {
        let runtime = AnimationRuntime::new();
        let values = Rc::new(RefCell::new(Vec::new()));
        let first_values = Rc::clone(&values);
        runtime.animate(
            1,
            AnimationChannel::Width,
            AnimatedValue::Scalar(0.0),
            AnimatedValue::Scalar(10.0),
            Animation::linear(Duration::from_secs(1)),
            Box::new(move |value| {
                if let AnimatedValue::Scalar(value) = value {
                    first_values.borrow_mut().push(value);
                }
            }),
        );
        assert!(runtime.take_frame_request());
        assert!(runtime.tick(Duration::from_millis(500)));
        assert_eq!(values.borrow().last().copied(), Some(5.0));

        let second_values = Rc::clone(&values);
        runtime.animate(
            1,
            AnimationChannel::Width,
            AnimatedValue::Scalar(5.0),
            AnimatedValue::Scalar(20.0),
            Animation::linear(Duration::from_secs(1)),
            Box::new(move |value| {
                if let AnimatedValue::Scalar(value) = value {
                    second_values.borrow_mut().push(value);
                }
            }),
        );
        runtime.tick(Duration::from_millis(750));
        assert_eq!(values.borrow().last().copied(), Some(8.75));
    }

    #[test]
    fn runtime_completion_callback_is_emitted_once_at_settlement() {
        let runtime = AnimationRuntime::new();
        let completions = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::clone(&completions);
        runtime.animate_with_completion(
            2,
            AnimationChannel::Opacity,
            AnimatedValue::Scalar(0.0),
            AnimatedValue::Scalar(1.0),
            Animation::linear(Duration::from_millis(10)),
            Box::new(move |_value, finished| {
                observed.borrow_mut().push(finished);
            }),
        );
        assert!(!runtime.tick(Duration::from_millis(10)));
        assert!(!runtime.tick(Duration::from_millis(11)));
        assert_eq!(completions.borrow().as_slice(), &[true]);
    }

    #[test]
    fn runtime_starts_idle_animation_from_current_wall_clock() {
        let runtime = AnimationRuntime::new();
        let values = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::clone(&values);

        runtime.tick(Duration::ZERO);
        std::thread::sleep(Duration::from_millis(40));
        runtime.sync_now();
        runtime.animate(
            3,
            AnimationChannel::Width,
            AnimatedValue::Scalar(0.0),
            AnimatedValue::Scalar(1.0),
            Animation::linear(Duration::from_millis(10)),
            Box::new(move |value| {
                if let AnimatedValue::Scalar(value) = value {
                    observed.borrow_mut().push(value);
                }
            }),
        );

        assert!(runtime.tick(Duration::from_millis(40)));
        assert!(values.borrow().last().copied().unwrap_or(1.0) < 1.0);
    }

    fn reflow_value(y: f32) -> AnimatedValue {
        AnimatedValue::Transform(VisualTransform {
            translation: Vector { x: 0.0, y },
            scale: 1.0,
            rotation: 0.0,
        })
    }

    #[test]
    fn spring_trajectory_stays_on_its_closed_form_across_ticks() {
        let spring = Animation::spring(Duration::from_millis(300), 0.5);
        let runtime = AnimationRuntime::new();
        let values = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::clone(&values);
        runtime.animate(
            4,
            AnimationChannel::Width,
            AnimatedValue::Scalar(0.0),
            AnimatedValue::Scalar(100.0),
            spring,
            Box::new(move |value| {
                if let AnimatedValue::Scalar(value) = value {
                    observed.borrow_mut().push(value);
                }
            }),
        );
        for ms in [16, 33, 50, 100, 150] {
            runtime.tick(Duration::from_millis(ms));
            let (expected, _) = spring.sample(0.0, 100.0, 0.0, Duration::from_millis(ms));
            let actual = values.borrow().last().copied().unwrap();
            assert!(
                (actual - expected).abs() < 1.0e-3,
                "t={ms}ms expected {expected}, got {actual}"
            );
        }
    }

    #[test]
    fn layout_reflow_intent_waits_for_layout_and_latest_record_wins() {
        let runtime = AnimationRuntime::new();
        let animation = Animation::linear(Duration::from_millis(10));
        runtime.record_layout_reflow_intent(LayoutReflowIntent::Animate(animation));
        assert_eq!(runtime.take_armed_layout_reflow_intent(), None);
        runtime.arm_layout_reflow_intent();
        runtime.record_layout_reflow_intent(LayoutReflowIntent::Snap);
        assert_eq!(
            runtime.take_armed_layout_reflow_intent(),
            None,
            "a record after the layout is not covered by it"
        );
        runtime.arm_layout_reflow_intent();
        assert_eq!(
            runtime.take_armed_layout_reflow_intent(),
            Some(LayoutReflowIntent::Snap)
        );
        assert_eq!(runtime.take_armed_layout_reflow_intent(), None);
        runtime.arm_layout_reflow_intent();
        assert_eq!(runtime.take_armed_layout_reflow_intent(), None);
    }

    #[test]
    fn layout_reflow_cancelled_from_its_own_callback_is_not_reinserted() {
        let runtime = AnimationRuntime::new();
        let weak = Rc::downgrade(&runtime);
        let calls = Rc::new(Cell::new(0));
        let observed = Rc::clone(&calls);
        runtime
            .rebase_layout_reflow(
                9,
                Vector::default(),
                Vector { x: 0.0, y: 20.0 },
                Animation::linear(Duration::from_millis(100)),
                None,
                Box::new(move |_, _| {
                    observed.set(observed.get() + 1);
                    weak.upgrade().unwrap().cancel_layout_reflow(9);
                }),
            )
            .unwrap();
        assert!(!runtime.tick(Duration::from_millis(10)));
        assert!(!runtime.has_layout_reflow(9));
        assert!(runtime.is_idle());
        assert!(!runtime.tick(Duration::from_millis(20)));
        assert_eq!(calls.get(), 1, "a cancelled channel never runs again");
    }

    #[test]
    fn replacement_installed_from_a_callback_wins_over_the_detached_channel() {
        let runtime = AnimationRuntime::new();
        let weak = Rc::downgrade(&runtime);
        let values = Rc::new(RefCell::new(Vec::new()));
        let first = Rc::clone(&values);
        let replaced = Rc::new(Cell::new(false));
        runtime
            .rebase_layout_reflow(
                3,
                Vector::default(),
                Vector { x: 0.0, y: 10.0 },
                Animation::linear(Duration::from_millis(100)),
                None,
                Box::new(move |value, _| {
                    first.borrow_mut().push(("old", value));
                    if !replaced.replace(true) {
                        let runtime = weak.upgrade().unwrap();
                        runtime.cancel_layout_reflow(3);
                        let second = Rc::clone(&first);
                        runtime.rebase_layout_reflow(
                            3,
                            Vector::default(),
                            Vector { x: 0.0, y: 50.0 },
                            Animation::linear(Duration::from_millis(100)),
                            None,
                            Box::new(move |value, _| second.borrow_mut().push(("new", value))),
                        );
                    }
                }),
            )
            .unwrap();
        assert!(runtime.tick(Duration::from_millis(10)));
        assert_eq!(runtime.layout_reflow_count(), 1);
        assert!(runtime.tick(Duration::from_millis(60)));
        let values = values.borrow();
        assert_eq!(values.len(), 2);
        assert_eq!(values[1], ("new", reflow_value(25.0)));
    }

    #[test]
    fn non_finite_layout_reflow_snaps_without_a_channel() {
        let runtime = AnimationRuntime::new();
        for delta in [
            Vector {
                x: f32::NAN,
                y: 0.0,
            },
            Vector {
                x: 0.0,
                y: f32::INFINITY,
            },
        ] {
            assert_eq!(
                runtime.rebase_layout_reflow(
                    1,
                    Vector::default(),
                    delta,
                    Animation::linear(Duration::from_millis(10)),
                    None,
                    Box::new(|_, _| {}),
                ),
                None
            );
        }
        assert_eq!(runtime.layout_reflow_count(), 0);
        assert!(!runtime.take_frame_request());
    }

    type Log = Rc<RefCell<Vec<&'static str>>>;

    fn logging(log: &Log, name: &'static str) -> Box<dyn Fn(AnimatedValue, bool)> {
        let log = Rc::clone(log);
        Box::new(move |_, _| log.borrow_mut().push(name))
    }

    #[test]
    fn rem05_channels_cancelled_or_replaced_earlier_in_a_tick_get_no_stale_callback() {
        // HashMap iteration order differs per runtime instance, so repeat on fresh runtimes.
        for _ in 0..32 {
            let runtime = AnimationRuntime::new();
            let log: Log = Rc::new(RefCell::new(Vec::new()));
            let linear = Animation::linear(Duration::from_millis(100));
            for (owner, name) in [(1, "k1-old"), (2, "k2-old")] {
                runtime
                    .rebase_layout_reflow(
                        owner,
                        Vector::default(),
                        Vector { x: 0.0, y: 10.0 },
                        linear,
                        None,
                        logging(&log, name),
                    )
                    .unwrap();
            }
            let weak = Rc::downgrade(&runtime);
            let actor_log = Rc::clone(&log);
            let acted = Rc::new(Cell::new(false));
            let acted_flag = Rc::clone(&acted);
            runtime.animate(
                3,
                AnimationChannel::Opacity,
                AnimatedValue::Scalar(0.0),
                AnimatedValue::Scalar(1.0),
                linear,
                Box::new(move |_| {
                    if acted_flag.replace(true) {
                        return;
                    }
                    actor_log.borrow_mut().push("actor");
                    let runtime = weak.upgrade().unwrap();
                    runtime.cancel_layout_reflow(1);
                    runtime.rebase_layout_reflow(
                        2,
                        Vector::default(),
                        Vector { x: 0.0, y: 50.0 },
                        linear,
                        None,
                        logging(&actor_log, "k2-new"),
                    );
                }),
            );
            assert!(runtime.tick(Duration::from_millis(10)));
            let first = log.borrow().clone();
            let actor = first.iter().position(|name| *name == "actor").unwrap();
            assert!(
                first[actor + 1..].is_empty(),
                "no stale or replacement delivery after the actor: {first:?}"
            );
            assert!(
                !first.contains(&"k2-new"),
                "replacement waits for the next tick"
            );
            assert_eq!(
                runtime.layout_reflow_count(),
                1,
                "k1 cancelled, k2 replaced once"
            );

            log.borrow_mut().clear();
            runtime.tick(Duration::from_millis(20));
            let second = log.borrow().clone();
            assert_eq!(
                second.iter().filter(|name| **name == "k2-new").count(),
                1,
                "{second:?}"
            );
            assert!(!second.contains(&"k1-old") && !second.contains(&"k2-old"));
        }
    }

    #[test]
    fn rem06_a_panicking_callback_leaves_no_in_flight_sample_behind() {
        let runtime = AnimationRuntime::new();
        let linear = Animation::linear(Duration::from_millis(100));
        runtime
            .rebase_layout_reflow(
                5,
                Vector::default(),
                Vector { x: 0.0, y: 40.0 },
                linear,
                None,
                Box::new(|_, _| panic!("callback failure")),
            )
            .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.tick(Duration::from_millis(50));
        }));
        assert!(result.is_err());
        assert_eq!(runtime.in_flight_sample_count(), 0);

        // A new channel on the same key starts from the caller's value, not the stale sample of
        // the panicked tick (which was 20 with a non-zero velocity).
        let values = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::clone(&values);
        let start = runtime
            .rebase_layout_reflow(
                5,
                Vector::default(),
                Vector { x: 0.0, y: 8.0 },
                linear,
                None,
                Box::new(move |value, _| observed.borrow_mut().push(value)),
            )
            .unwrap();
        assert_eq!(start, Vector { x: 0.0, y: 8.0 });
        assert!(runtime.tick(Duration::from_millis(100)));
        assert_eq!(values.borrow().as_slice(), &[reflow_value(4.0)]);
    }

    #[test]
    fn reentrant_tick_from_a_callback_does_not_sample_again() {
        let runtime = AnimationRuntime::new();
        let weak = Rc::downgrade(&runtime);
        let calls = Rc::new(Cell::new(0));
        let observed = Rc::clone(&calls);
        runtime.animate(
            1,
            AnimationChannel::Opacity,
            AnimatedValue::Scalar(0.0),
            AnimatedValue::Scalar(1.0),
            Animation::linear(Duration::from_millis(100)),
            Box::new(move |_| {
                observed.set(observed.get() + 1);
                assert!(weak.upgrade().unwrap().tick(Duration::from_millis(90)));
            }),
        );
        assert!(runtime.tick(Duration::from_millis(10)));
        assert_eq!(calls.get(), 1);
        assert!(runtime.take_frame_request());
    }

    fn reflow_on(
        runtime: &Rc<AnimationRuntime>,
        owner: u64,
        target: Option<&Rc<dyn crate::ui::UIElementExt>>,
        calls: &Rc<Cell<usize>>,
    ) {
        let calls = Rc::clone(calls);
        let start = runtime
            .rebase_layout_reflow(
                owner,
                Vector::default(),
                Vector { x: 0.0, y: 20.0 },
                Animation::linear(Duration::from_millis(100)),
                target.map(Rc::downgrade),
                Box::new(move |_, _| calls.set(calls.get() + 1)),
            )
            .unwrap();
        if let Some(target) = target {
            target.as_ui_element().layout_reflow_translation.set(start);
        }
    }

    #[test]
    fn r2_t01_discard_clears_an_unconsumed_reflow_only_frame_request() {
        let runtime = AnimationRuntime::new();
        let leaf = crate::ui::testsupport::reflow_leaf("leaf");
        let calls = Rc::new(Cell::new(0));
        reflow_on(&runtime, 1, Some(&leaf), &calls);
        assert_eq!(
            leaf.as_ui_element().layout_reflow_translation.get(),
            Vector { x: 0.0, y: 20.0 }
        );
        runtime.discard_layout_reflows();
        assert!(!runtime.take_frame_request());
        assert!(runtime.is_idle());
        assert_eq!(runtime.layout_reflow_count(), 0);
        assert!(runtime.pending_layout_reflow_intent().is_none());
        assert!(!runtime.has_layout_reflow_accessibility_root());
        assert_eq!(
            leaf.as_ui_element().layout_reflow_translation.get(),
            Vector::default()
        );
        assert!(!runtime.tick(Duration::from_millis(50)));
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn r2_t02_discard_after_a_consumed_request_never_requests_again() {
        let runtime = AnimationRuntime::new();
        let calls = Rc::new(Cell::new(0));
        reflow_on(&runtime, 1, None, &calls);
        assert!(runtime.take_frame_request());
        runtime.discard_layout_reflows();
        assert!(!runtime.take_frame_request());
        runtime.discard_layout_reflows();
        assert!(!runtime.take_frame_request());
        assert!(runtime.is_idle());
    }

    #[test]
    fn r2_t03_discard_keeps_a_public_channel_and_its_frame_request() {
        let runtime = AnimationRuntime::new();
        let values = Rc::new(RefCell::new(Vec::new()));
        let observed = Rc::clone(&values);
        runtime.animate_with_completion(
            2,
            AnimationChannel::Opacity,
            AnimatedValue::Scalar(0.0),
            AnimatedValue::Scalar(1.0),
            Animation::linear(Duration::from_millis(100)),
            Box::new(move |value, finished| observed.borrow_mut().push((value, finished))),
        );
        let calls = Rc::new(Cell::new(0));
        reflow_on(&runtime, 1, None, &calls);
        runtime.discard_layout_reflows();
        assert!(runtime.take_frame_request(), "the public request survives");
        assert!(!runtime.is_idle());
        assert_eq!(runtime.layout_reflow_count(), 0);
        assert!(runtime.tick(Duration::from_millis(50)));
        assert!(runtime.take_frame_request());
        assert!(!runtime.tick(Duration::from_millis(100)));
        assert_eq!(
            values.borrow().as_slice(),
            &[
                (AnimatedValue::Scalar(0.5), false),
                (AnimatedValue::Scalar(1.0), true)
            ]
        );
        assert_eq!(calls.get(), 0, "the discarded reflow never runs");
    }

    #[test]
    fn r2_t04_discard_inside_a_public_tick_callback_sees_the_detached_public_channel() {
        let runtime = AnimationRuntime::new();
        let reflow_calls = Rc::new(Cell::new(0));
        reflow_on(&runtime, 1, None, &reflow_calls);
        let weak = Rc::downgrade(&runtime);
        let inner_calls = Rc::new(Cell::new(0));
        let inner = Rc::clone(&inner_calls);
        let observed_request = Rc::new(Cell::new(None));
        let observed = Rc::clone(&observed_request);
        let public_calls = Rc::new(Cell::new(0));
        let public = Rc::clone(&public_calls);
        runtime.animate(
            2,
            AnimationChannel::Opacity,
            AnimatedValue::Scalar(0.0),
            AnimatedValue::Scalar(1.0),
            Animation::linear(Duration::from_millis(100)),
            Box::new(move |_| {
                public.set(public.get() + 1);
                if observed.get().is_some() {
                    return;
                }
                let runtime = weak.upgrade().unwrap();
                // A reflow started from this callback raises the request while the public channel
                // itself is detached; only its generation shows it is still live.
                reflow_on(&runtime, 3, None, &inner);
                runtime.discard_layout_reflows();
                observed.set(Some(runtime.take_frame_request()));
            }),
        );
        let before = reflow_calls.get();
        assert!(runtime.tick(Duration::from_millis(10)));
        assert_eq!(
            observed_request.get(),
            Some(true),
            "a live in-flight public channel keeps the request"
        );
        assert!(reflow_calls.get() <= before + 1, "no extra stale delivery");
        let after_first = reflow_calls.get();
        assert_eq!(runtime.layout_reflow_count(), 0);
        assert!(
            runtime.take_frame_request(),
            "the public channel still needs frames"
        );
        assert!(runtime.tick(Duration::from_millis(20)));
        assert_eq!(public_calls.get(), 2, "the public channel keeps running");
        assert_eq!(
            reflow_calls.get(),
            after_first,
            "discarded reflow never resumes"
        );
        assert_eq!(inner_calls.get(), 0);
    }

    #[test]
    fn r2_t05_reflow_callback_discarding_its_own_host_leaves_nothing_behind() {
        let runtime = AnimationRuntime::new();
        let weak = Rc::downgrade(&runtime);
        let calls = Rc::new(Cell::new(0));
        let observed = Rc::clone(&calls);
        let leaf = crate::ui::testsupport::reflow_leaf("leaf");
        let start = runtime
            .rebase_layout_reflow(
                1,
                Vector::default(),
                Vector { x: 0.0, y: 20.0 },
                Animation::linear(Duration::from_millis(100)),
                Some(Rc::downgrade(&leaf)),
                Box::new(move |_, _| {
                    observed.set(observed.get() + 1);
                    weak.upgrade().unwrap().discard_layout_reflows();
                }),
            )
            .unwrap();
        leaf.as_ui_element().layout_reflow_translation.set(start);
        let other_calls = Rc::new(Cell::new(0));
        reflow_on(&runtime, 2, None, &other_calls);
        assert!(!runtime.tick(Duration::from_millis(10)));
        assert!(calls.get() == 1);
        assert!(other_calls.get() <= 1);
        assert_eq!(runtime.layout_reflow_count(), 0);
        assert!(!runtime.take_frame_request());
        assert!(runtime.is_idle());
        assert_eq!(
            leaf.as_ui_element().layout_reflow_translation.get(),
            Vector::default()
        );
        let other_after = other_calls.get();
        assert!(!runtime.tick(Duration::from_millis(20)));
        assert_eq!(calls.get(), 1, "no resurrection");
        assert_eq!(other_calls.get(), other_after);
    }
}
