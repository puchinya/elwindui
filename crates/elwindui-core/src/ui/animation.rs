//! Backend-neutral animation primitives and the synchronous transaction scope.
//!
//! Animation state is deliberately independent from any frame source. Hosts may use these
//! primitives from a per-tree runtime, while tests can sample them with explicit monotonic times.

use crate::base::{AffineTransform, Size, Vector};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
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

struct ActiveAnimation {
    start: AnimatedValue,
    target: AnimatedValue,
    velocity: AnimatedValue,
    animation: Animation,
    started_at: Duration,
    value: AnimatedValue,
    callback: Box<dyn Fn(AnimatedValue, bool)>,
}

/// Deterministic, per-host animation state. A host owns one instance for its hosted tree and
/// provides explicit monotonic timestamps through [`Self::tick`].
pub struct AnimationRuntime {
    channels: RefCell<HashMap<(u64, AnimationChannel), ActiveAnimation>>,
    now: RefCell<Duration>,
    frame_requested: RefCell<bool>,
    epoch: Instant,
}

impl AnimationRuntime {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            channels: RefCell::new(HashMap::new()),
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
        let now = *self.now.borrow();
        let key = (owner_id, channel);
        let (start, velocity) = self
            .channels
            .borrow()
            .get(&key)
            .map(|active| {
                let elapsed = now.saturating_sub(active.started_at);
                active
                    .start
                    .sample(active.target, active.velocity, active.animation, elapsed)
            })
            .unwrap_or((current, zero_velocity(current)));
        let value = start;
        self.channels.borrow_mut().insert(
            key,
            ActiveAnimation {
                start,
                target,
                velocity,
                animation,
                started_at: now,
                value,
                callback,
            },
        );
        *self.frame_requested.borrow_mut() = true;
    }

    /// Advances every channel to an explicit monotonic timestamp and returns whether work remains.
    /// Callbacks run outside the channel borrow, so a callback may safely retarget another channel.
    pub fn tick(&self, now: Duration) -> bool {
        let previous = *self.now.borrow();
        let now = now.max(previous);
        *self.now.borrow_mut() = now;
        *self.frame_requested.borrow_mut() = false;

        let current = std::mem::take(&mut *self.channels.borrow_mut());
        let mut keep = HashMap::new();
        for (key, mut active) in current {
            let elapsed = now.saturating_sub(active.started_at);
            let (value, velocity) =
                active
                    .start
                    .sample(active.target, active.velocity, active.animation, elapsed);
            active.value = value;
            active.velocity = velocity;
            let finished =
                active
                    .value
                    .settled(active.target, active.velocity, active.animation, elapsed);
            (active.callback)(value, finished);
            if !finished {
                keep.insert(key, active);
            }
        }
        // A callback may have installed a replacement under the same key. The replacement wins.
        let mut channels = self.channels.borrow_mut();
        for (key, active) in keep {
            channels.entry(key).or_insert(active);
        }
        let active = !channels.is_empty();
        *self.frame_requested.borrow_mut() = active;
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
}
