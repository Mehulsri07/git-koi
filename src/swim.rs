//! Free swimming: a koi that steers anywhere in a pond, one small time step at a time,
//! toward food, wandering waypoints, or its resting place.
//!
//! Pond coordinates are native koi pixels from the pond's top left corner, with y
//! growing downward; times are seconds. A heading is the screen angle of travel, as
//! everywhere in this crate: zero points right and positive angles turn toward +y,
//! which is clockwise on screen, so the head moves along `[cos, sin]` of its heading.
//!
//! Each [`SwimState::step`] eases the speed toward the goal's target speed and turns the
//! heading toward the goal by at most [`MAX_TURN_RADIANS_PER_SECOND`], so the koi always
//! swims arcs. It also turns no tighter than a circle its body could follow without
//! wrapping around itself, so the body, which follows the head's own trail, never
//! folds. The goal is, in order: food the koi has not yet eaten; for cruising and
//! darting, a wandering waypoint drawn from the seed; for resting, deep rest, and
//! surface feeding, the pond's rest point, where the koi slows to a hover. The head
//! never leaves the pond inset by [`WALL_INSET_BODY_LENGTHS`] body lengths: the koi
//! turns away from an edge it is about to reach, and is held at the edge if it still
//! reaches it.
//!
//! The state keeps the head's recent path, as long as the body needs, so
//! [`crate::render::KoiRenderer::render_free_frame`] can lay the body back along it.
//! Like the rest of the crate, swimming is deterministic and reads no clock: the caller
//! passes each step's duration.

use std::collections::VecDeque;
use std::f64::consts::{PI, TAU};

use crate::behavior::Behavior;
use crate::motion::calculate_free_speed_share;

/// The fastest the heading turns, in radians per second.
pub const MAX_TURN_RADIANS_PER_SECOND: f64 = 2.4;
/// The speed cruising eases toward, in pixels per second.
pub const CRUISE_SPEED_PIXELS_PER_SECOND: f64 = 22.0;
/// Darting's mean speed in pixels per second; it surges after each dart stroke and
/// slows through the glide.
pub const DART_SPEED_PIXELS_PER_SECOND: f64 = 45.0;
/// The speed every behavior eases toward while it chases food, in pixels per second.
pub const FOOD_SPEED_PIXELS_PER_SECOND: f64 = 40.0;
/// The head may come no closer to a pond edge than this many body lengths.
pub const WALL_INSET_BODY_LENGTHS: f64 = 0.6;
/// A wandering koi picks its next waypoint once its head is this close to the current one.
pub const WAYPOINT_REACHED_PIXELS: f64 = 8.0;
/// The koi gulps food once its head is this close to it.
pub const GULP_REACH_PIXELS: f64 = 4.0;
/// How long the mouth stays in its gulp after the koi eats.
pub const GULP_SECONDS: f64 = 0.6;
/// The longest step [`SwimState::step`] takes; longer steps are cut to this.
pub const MAX_STEP_SECONDS: f64 = 0.1;

/// Speed closes on its target by about two thirds in this time.
const SPEED_EASE_SECONDS: f64 = 0.5;
/// A goal straight to the side or behind still gets this share of the target speed, so
/// the koi slows a little to turn but keeps swimming: its tightest turn is a circle
/// of fixed size, so slowing further would only make turns take longer.
const MISALIGNED_SPEED_SHARE: f64 = 0.75;
/// The tightest turn's circle is at least so wide that the body covers no more than this
/// share of it, the same limit the lap circle keeps.
const MAX_BODY_SHARE_OF_TURN: f64 = 0.6;
/// A resting koi slows over this last stretch to its rest point.
const REST_SLOWING_PIXELS: f64 = 40.0;
/// Within this of its rest point, a resting koi stops steering and lets its speed fade.
const REST_ARRIVED_PIXELS: f64 = 3.0;
/// The koi looks this far ahead, beyond its turning radius, for the pond edge.
const WALL_LOOKAHEAD_SECONDS: f64 = 0.5;
/// A goal found inside the turning circle is passed straight by until it is at least this
/// many turning radii from the circle's center, so that turning toward it afterwards
/// lines the koi up with it instead of circling it.
const GOAL_CLEARANCE_TURN_RADII: f64 = 1.5;
/// A wandering koi that has not reached its waypoint in this long picks another.
const WAYPOINT_PATIENCE_SECONDS: f64 = 60.0;
/// The trail keeps a new head position once the head has moved this far from the last.
const TRAIL_SPACING_PIXELS: f64 = 0.5;
/// The trail runs this much past the body length, so rounding never runs it short.
const TRAIL_MARGIN_PIXELS: f64 = 4.0;

/// The pond a koi swims in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pond {
    /// The pond's width in native pixels.
    pub width_pixels: f64,
    /// The pond's height in native pixels.
    pub height_pixels: f64,
    /// Where resting, deep rest, and surface feeding hover, in pond coordinates. A point
    /// too near an edge is moved inside the swimming area.
    pub rest_point: [f64; 2],
}

/// What happened during one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwimEvent {
    /// Nothing the caller needs to act on.
    None,
    /// The koi ate the food it was given; the caller removes it.
    Gulped,
}

/// One koi swimming freely: where it is, where it is going, and the path behind it.
#[derive(Clone, Debug)]
pub struct SwimState {
    pond: Pond,
    body_length_pixels: f64,
    /// The swimming area's corners, the pond inset from every edge.
    min_corner: [f64; 2],
    max_corner: [f64; 2],
    head: [f64; 2],
    heading_radians: f64,
    speed_pixels_per_second: f64,
    /// Seconds swum since the state began; fins and wave follow it.
    clock_seconds: f64,
    random_state: u64,
    waypoint: [f64; 2],
    /// Seconds spent heading for the current waypoint.
    waypoint_seconds: f64,
    /// Whether the koi is swimming straight past a goal inside its turning circle.
    is_passing_goal: bool,
    /// The way the koi is turning the long way round to a goal behind it, away from an
    /// edge: one or minus one, or zero when it is not.
    long_turn_sign: f64,
    /// Whether the koi has arrived at its rest point and hovers there.
    is_hovering: bool,
    /// The food last eaten, ignored until the caller takes it away.
    eaten_food: Option<[f64; 2]>,
    /// Seconds since the last gulp, while the mouth is still in it.
    gulp_seconds: Option<f64>,
    /// Earlier head positions, newest first, at least TRAIL_SPACING_PIXELS apart.
    trail: VecDeque<[f64; 2]>,
}

impl SwimState {
    /// Koi at the pond center heading right; `seed` drives wandering; `body_length_pixels`
    /// is [`crate::render::KoiRenderer::body_length_pixels`] for the koi drawn. The trail
    /// starts straight behind the head, as if the koi had been swimming right.
    ///
    /// A pond size or body length that is negative or not a finite number counts as zero,
    /// and a rest point that is not finite is the pond center, so the state always stays
    /// finite.
    pub fn new(seed: u64, pond: Pond, body_length_pixels: f64) -> Self {
        let keep_size = |size: f64| if size.is_finite() { size.max(0.0) } else { 0.0 };
        let body_length_pixels = keep_size(body_length_pixels);
        let mut pond = Pond {
            width_pixels: keep_size(pond.width_pixels),
            height_pixels: keep_size(pond.height_pixels),
            ..pond
        };
        if !pond
            .rest_point
            .iter()
            .all(|coordinate| coordinate.is_finite())
        {
            pond.rest_point = [pond.width_pixels / 2.0, pond.height_pixels / 2.0];
        }
        let inset_pixels = WALL_INSET_BODY_LENGTHS * body_length_pixels;
        let pond_size = [pond.width_pixels, pond.height_pixels];
        let min_corner = [0, 1].map(|axis_index| inset_pixels.min(pond_size[axis_index] / 2.0));
        let max_corner = [0, 1].map(|axis_index| pond_size[axis_index] - min_corner[axis_index]);
        let head = [pond.width_pixels / 2.0, pond.height_pixels / 2.0];
        let trail_length_pixels = body_length_pixels + TRAIL_MARGIN_PIXELS;
        let trail = (1..=trail_length_pixels.ceil() as usize)
            .map(|step_pixels| [head[0] - step_pixels as f64, head[1]])
            .collect();
        let mut swim_state = Self {
            pond,
            body_length_pixels,
            min_corner,
            max_corner,
            head,
            heading_radians: 0.0,
            speed_pixels_per_second: 0.0,
            clock_seconds: 0.0,
            random_state: seed,
            waypoint: head,
            waypoint_seconds: 0.0,
            is_passing_goal: false,
            long_turn_sign: 0.0,
            is_hovering: false,
            eaten_food: None,
            gulp_seconds: None,
            trail,
        };
        swim_state.waypoint = swim_state.draw_waypoint();
        swim_state
    }

    /// Advance by dt (clamped to at most 0.1 s per call); `food` is the current food point, if any.
    ///
    /// A step that is negative or not a number takes no time. Food beyond the swimming
    /// area is chased to the nearest point the head can reach and gulped there. Once the
    /// koi gulps a piece of food it ignores that same point until a step passes no food.
    /// Food that is not a finite point counts as no food.
    pub fn step(
        &mut self,
        dt_seconds: f64,
        behavior: Behavior,
        food: Option<[f64; 2]>,
    ) -> SwimEvent {
        let dt_seconds = if dt_seconds > 0.0 {
            dt_seconds.min(MAX_STEP_SECONDS)
        } else {
            0.0
        };
        self.clock_seconds += dt_seconds;
        self.gulp_seconds = self
            .gulp_seconds
            .map(|gulp_seconds| gulp_seconds + dt_seconds)
            .filter(|&gulp_seconds| gulp_seconds < GULP_SECONDS);
        let food =
            food.filter(|food_point| food_point.iter().all(|coordinate| coordinate.is_finite()));
        if food.is_none() {
            self.eaten_food = None;
        }
        let food = food.filter(|&food_point| self.eaten_food != Some(food_point));
        let reachable_food = food.map(|food_point| self.clamp_to_area(food_point));
        self.waypoint_seconds += dt_seconds;
        if reachable_food.is_none()
            && (distance(self.head, self.waypoint) < WAYPOINT_REACHED_PIXELS
                || self.waypoint_seconds >= WAYPOINT_PATIENCE_SECONDS)
        {
            self.waypoint = self.draw_waypoint();
            self.waypoint_seconds = 0.0;
        }
        let (goal, target_speed, is_steering) = match (reachable_food, behavior) {
            (Some(food_point), _) => (food_point, FOOD_SPEED_PIXELS_PER_SECOND, true),
            (None, Behavior::Cruise) => (self.waypoint, CRUISE_SPEED_PIXELS_PER_SECOND, true),
            (None, Behavior::Dart) => (
                self.waypoint,
                DART_SPEED_PIXELS_PER_SECOND
                    * calculate_free_speed_share(behavior, self.calculate_frame()),
                true,
            ),
            (None, Behavior::Rest | Behavior::DeepRest | Behavior::SurfaceFeeding) => {
                let rest_point = self.clamp_to_area(self.pond.rest_point);
                let rest_distance = distance(self.head, rest_point);
                (
                    rest_point,
                    CRUISE_SPEED_PIXELS_PER_SECOND * (rest_distance / REST_SLOWING_PIXELS).min(1.0),
                    rest_distance >= REST_ARRIVED_PIXELS,
                )
            }
        };
        self.is_hovering = !is_steering;
        let steering_goal = self.reach_past_edges(goal);
        let goal_turn_radians = if is_steering {
            wrap_angle(
                (steering_goal[1] - self.head[1]).atan2(steering_goal[0] - self.head[0])
                    - self.heading_radians,
            )
        } else {
            0.0
        };
        let target_speed = if is_steering {
            target_speed * goal_turn_radians.cos().max(MISALIGNED_SPEED_SHARE)
        } else {
            0.0
        };
        self.speed_pixels_per_second += (target_speed - self.speed_pixels_per_second)
            * (1.0 - (-dt_seconds / SPEED_EASE_SECONDS).exp());
        self.turn(steering_goal, goal, goal_turn_radians, dt_seconds);
        let forward_direction = [self.heading_radians.cos(), self.heading_radians.sin()];
        let travel_pixels = self.speed_pixels_per_second * dt_seconds;
        self.head = self.clamp_to_area([
            self.head[0] + forward_direction[0] * travel_pixels,
            self.head[1] + forward_direction[1] * travel_pixels,
        ]);
        self.record_trail();
        match (food, reachable_food) {
            (Some(food_point), Some(reachable_point))
                if distance(self.head, reachable_point) <= GULP_REACH_PIXELS =>
            {
                self.eaten_food = Some(food_point);
                self.gulp_seconds = Some(0.0);
                SwimEvent::Gulped
            }
            _ => SwimEvent::None,
        }
    }

    /// Where the head is, in pond coordinates.
    pub fn head(&self) -> [f64; 2] {
        self.head
    }

    /// The direction of travel in radians: zero is right, and positive turns clockwise on
    /// screen, toward +y. It stays within one turn either side of zero.
    pub fn heading_radians(&self) -> f64 {
        self.heading_radians
    }

    /// The loop frame the fins and wave are at: the swim clock at
    /// [`crate::motion::FRAMES_PER_SECOND`].
    pub(crate) fn calculate_frame(&self) -> f64 {
        self.clock_seconds * f64::from(crate::motion::FRAMES_PER_SECOND)
    }

    /// Whether the koi has arrived at its rest point and hovers there.
    pub(crate) fn is_hovering(&self) -> bool {
        self.is_hovering
    }

    /// How far the gulp opens the mouth now, from zero closed to one wide open.
    pub(crate) fn calculate_gulp_opening(&self) -> f64 {
        self.gulp_seconds.map_or(0.0, |gulp_seconds| {
            (PI * gulp_seconds / GULP_SECONDS).sin().max(0.0)
        })
    }

    /// Points along the swum path, one pixel of arc apart, from the head back: the first
    /// is the head itself. A path shorter than asked for continues straight back.
    pub(crate) fn trace_back(&self, point_count: usize) -> Vec<[f64; 2]> {
        let mut traced_points = Vec::with_capacity(point_count);
        if point_count == 0 {
            return traced_points;
        }
        traced_points.push(self.head);
        let mut segment_start = self.head;
        // Arc length from the head to the start of the current segment.
        let mut segment_start_arc = 0.0;
        let mut last_direction = [-self.heading_radians.cos(), -self.heading_radians.sin()];
        for &segment_end in &self.trail {
            let segment_pixels = distance(segment_start, segment_end);
            if segment_pixels == 0.0 {
                continue;
            }
            last_direction = [0, 1].map(|axis_index| {
                (segment_end[axis_index] - segment_start[axis_index]) / segment_pixels
            });
            let segment_end_arc = segment_start_arc + segment_pixels;
            while traced_points.len() < point_count && traced_points.len() as f64 <= segment_end_arc
            {
                let arc_into_segment = traced_points.len() as f64 - segment_start_arc;
                traced_points.push([0, 1].map(|axis_index| {
                    segment_start[axis_index] + last_direction[axis_index] * arc_into_segment
                }));
            }
            if traced_points.len() == point_count {
                return traced_points;
            }
            segment_start = segment_end;
            segment_start_arc = segment_end_arc;
        }
        while traced_points.len() < point_count {
            let arc_beyond = traced_points.len() as f64 - segment_start_arc;
            traced_points.push([0, 1].map(|axis_index| {
                segment_start[axis_index] + last_direction[axis_index] * arc_beyond
            }));
        }
        traced_points
    }

    /// Turn toward the goal, or away from an edge the koi is about to reach. A goal
    /// inside the circle the koi would turn on cannot be reached by turning, so the koi
    /// swims straight on until the goal is well clear of that circle.
    fn turn(
        &mut self,
        goal: [f64; 2],
        reached_goal: [f64; 2],
        goal_turn_radians: f64,
        dt_seconds: f64,
    ) {
        let turn_radius_pixels = self.calculate_turn_radius();
        let turn_sign = if goal_turn_radians >= 0.0 { 1.0 } else { -1.0 };
        let turn_center = [
            self.head[0] - turn_sign * turn_radius_pixels * self.heading_radians.sin(),
            self.head[1] + turn_sign * turn_radius_pixels * self.heading_radians.cos(),
        ];
        let center_to_goal_pixels = distance(turn_center, goal);
        if goal != reached_goal {
            // A goal past an edge is reached by sliding along the edge, not by lining up
            // with it, so the koi always turns for it; held at the edge, it can.
            self.is_passing_goal = false;
        } else if center_to_goal_pixels < turn_radius_pixels {
            self.is_passing_goal = true;
        } else if center_to_goal_pixels > GOAL_CLEARANCE_TURN_RADII * turn_radius_pixels {
            self.is_passing_goal = false;
        }
        if self.is_passing_goal || goal_turn_radians.abs() < PI / 2.0 {
            self.long_turn_sign = 0.0;
        }
        let mut wanted_turn_radians = if self.is_passing_goal {
            0.0
        } else {
            goal_turn_radians
        };
        // Look ahead for the edge, and turn between the heading mirrored off it and the
        // way to the middle of the swimming area, which also picks a side in a corner.
        let forward_direction = [self.heading_radians.cos(), self.heading_radians.sin()];
        let mut lookahead_pixels =
            turn_radius_pixels + self.speed_pixels_per_second * WALL_LOOKAHEAD_SECONDS;
        // A goal ahead is inside the swimming area, so the edge beyond it does not
        // matter yet: the koi may swim up to a goal at the edge, held there by the clamp.
        // An edge the goal lies on is where the koi is headed, so it never turns away
        // from that edge: held there, it slides along it while it turns for the goal.
        let is_heading_for_goal = !self.is_passing_goal && goal_turn_radians.cos() > 0.0;
        if is_heading_for_goal {
            lookahead_pixels = lookahead_pixels.min(distance(self.head, reached_goal));
        }
        let mut mirrored_direction = forward_direction;
        let mut is_near_edge = false;
        for axis_index in 0..2 {
            let probe = self.head[axis_index] + forward_direction[axis_index] * lookahead_pixels;
            let is_goal_on_min_edge = reached_goal[axis_index] <= self.min_corner[axis_index];
            let is_goal_on_max_edge = reached_goal[axis_index] >= self.max_corner[axis_index];
            if (probe < self.min_corner[axis_index]
                && forward_direction[axis_index] < 0.0
                && !is_goal_on_min_edge)
                || (probe > self.max_corner[axis_index]
                    && forward_direction[axis_index] > 0.0
                    && !is_goal_on_max_edge)
            {
                mirrored_direction[axis_index] = -forward_direction[axis_index];
                is_near_edge = true;
            }
        }
        if is_near_edge {
            let to_middle = [0, 1].map(|axis_index| {
                (self.min_corner[axis_index] + self.max_corner[axis_index]) / 2.0
                    - self.head[axis_index]
            });
            let middle_pixels = to_middle[0].hypot(to_middle[1]);
            let escape_direction = if middle_pixels > 1.0 {
                [0, 1].map(|axis_index| {
                    mirrored_direction[axis_index] + to_middle[axis_index] / middle_pixels
                })
            } else {
                mirrored_direction
            };
            let escape_turn_radians =
                wrap_angle(escape_direction[1].atan2(escape_direction[0]) - self.heading_radians);
            // A goal behind that the short way round would turn the koi into the edge is
            // turned for the long way round instead, away from the edge, rather than
            // held off by the edge while the koi swims along it, away from the goal.
            if self.long_turn_sign == 0.0
                && !self.is_passing_goal
                && goal_turn_radians.abs() > PI / 2.0
                && goal_turn_radians.signum() != escape_turn_radians.signum()
            {
                self.long_turn_sign = escape_turn_radians.signum();
            }
            if self.long_turn_sign == 0.0 {
                wanted_turn_radians = escape_turn_radians;
            }
        }
        // Once begun, the long way round goes on until the goal is ahead, even after
        // the edge falls out of sight, so the koi does not swing back toward the edge.
        if self.long_turn_sign != 0.0 {
            wanted_turn_radians = if goal_turn_radians.signum() == self.long_turn_sign {
                goal_turn_radians
            } else {
                goal_turn_radians + self.long_turn_sign * TAU
            };
        }
        // Turning rate is bounded both by the limit and by the tightest circle.
        let max_turn_radians = MAX_TURN_RADIANS_PER_SECOND
            .min(self.speed_pixels_per_second / self.calculate_min_turn_radius())
            * dt_seconds;
        self.heading_radians = wrap_angle(
            self.heading_radians + wanted_turn_radians.clamp(-max_turn_radians, max_turn_radians),
        );
    }

    /// Where to steer for a goal: the goal itself, or, for a goal on the edge of the
    /// swimming area, a point beyond that edge by a turning radius, or by the distance
    /// to the goal once that is shorter.
    ///
    /// A goal on an edge, and most of all in a corner, can lie inside the koi's turning
    /// circle with no room to swing out and come back, so steering at it can circle it
    /// for minutes. Aiming past the edge instead heads the koi into the edge near the
    /// goal; the edge holds the head, which slides along it, and the aim pulls it along
    /// the edge toward the goal, ever more gently as it nears. Goals inside the area are
    /// steered at directly.
    fn reach_past_edges(&self, goal: [f64; 2]) -> [f64; 2] {
        // Near the goal the aim comes in closer, so the slide along the edge stays brisk
        // to the end instead of fading with the shrinking angle.
        let reach_pixels = self.calculate_turn_radius().min(distance(self.head, goal));
        [0, 1].map(|axis_index| {
            if goal[axis_index] <= self.min_corner[axis_index] {
                goal[axis_index] - reach_pixels
            } else if goal[axis_index] >= self.max_corner[axis_index] {
                goal[axis_index] + reach_pixels
            } else {
                goal[axis_index]
            }
        })
    }

    /// The tightest circle the body can follow without wrapping around itself.
    fn calculate_min_turn_radius(&self) -> f64 {
        (self.body_length_pixels / (TAU * MAX_BODY_SHARE_OF_TURN)).max(1.0)
    }

    /// The radius of the circle the koi swims at its fastest turn at its current speed.
    fn calculate_turn_radius(&self) -> f64 {
        self.calculate_min_turn_radius()
            .max(self.speed_pixels_per_second / MAX_TURN_RADIANS_PER_SECOND)
    }

    fn clamp_to_area(&self, point: [f64; 2]) -> [f64; 2] {
        [0, 1].map(|axis_index| {
            point[axis_index].clamp(self.min_corner[axis_index], self.max_corner[axis_index])
        })
    }

    /// The next wandering waypoint, anywhere in the swimming area.
    fn draw_waypoint(&mut self) -> [f64; 2] {
        [0, 1].map(|axis_index| {
            let fraction = draw_fraction(&mut self.random_state);
            self.min_corner[axis_index]
                + (self.max_corner[axis_index] - self.min_corner[axis_index]) * fraction
        })
    }

    /// Keep the head's new position once it has moved far enough, and forget the path
    /// beyond what the body needs.
    fn record_trail(&mut self) {
        if self
            .trail
            .front()
            .is_none_or(|&newest_point| distance(newest_point, self.head) >= TRAIL_SPACING_PIXELS)
        {
            self.trail.push_front(self.head);
        }
        let needed_pixels = self.body_length_pixels + TRAIL_MARGIN_PIXELS;
        let mut covered_pixels = 0.0;
        let mut previous_point = self.head;
        for (point_index, &trail_point) in self.trail.iter().enumerate() {
            covered_pixels += distance(previous_point, trail_point);
            previous_point = trail_point;
            if covered_pixels >= needed_pixels {
                self.trail.truncate(point_index + 1);
                break;
            }
        }
    }
}

fn distance(from: [f64; 2], to: [f64; 2]) -> f64 {
    (to[0] - from[0]).hypot(to[1] - from[1])
}

/// An angle wrapped to within half a turn either side of zero.
fn wrap_angle(angle_radians: f64) -> f64 {
    (angle_radians + PI).rem_euclid(TAU) - PI
}

/// A fraction from zero up to one, from the SplitMix64 sequence, which every seed
/// starts well.
fn draw_fraction(random_state: &mut u64) -> f64 {
    *random_state = random_state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut mixed = *random_state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^= mixed >> 31;
    (mixed >> 11) as f64 / (1_u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Waypoints a wandering koi reaches in each minute of ten.
    fn count_waypoints_per_minute(
        seed: u64,
        pond: Pond,
        body_length_pixels: f64,
        behavior: Behavior,
    ) -> Vec<usize> {
        const STEP_SECONDS: f64 = 1.0 / 60.0;
        let mut swim_state = SwimState::new(seed, pond, body_length_pixels);
        (0..10)
            .map(|_| {
                let mut reached_count = 0;
                for _ in 0..60 * 60 {
                    // Count only waypoints reached, not ones given up on.
                    if distance(swim_state.head, swim_state.waypoint) < WAYPOINT_REACHED_PIXELS {
                        reached_count += 1;
                    }
                    swim_state.step(STEP_SECONDS, behavior, None);
                }
                reached_count
            })
            .collect()
    }

    #[test]
    fn wandering_keeps_reaching_new_waypoints() {
        // The longest leg, corner to corner of a 960x540 pond's swimming area, takes under
        // 50 s at cruising speed, so any two minutes of healthy wandering hold at least one
        // whole leg; healthy koi average 4.5 to 10 waypoints a minute across these ponds
        // and sizes and reach at least 3 in every two minutes. A koi circling a waypoint
        // reaches none for minutes on end.
        const MIN_WAYPOINTS_PER_TWO_MINUTES: usize = 2;
        for (width_pixels, height_pixels) in [(480.0, 270.0), (960.0, 540.0)] {
            let pond = Pond {
                width_pixels,
                height_pixels,
                rest_point: [0.0; 2],
            };
            for body_length_pixels in [42.0, 99.4, 167.5] {
                for seed in 0..20 {
                    for behavior in [Behavior::Cruise, Behavior::Dart] {
                        let per_minute =
                            count_waypoints_per_minute(seed, pond, body_length_pixels, behavior);
                        assert!(
                            per_minute.windows(2).all(|two_minutes| {
                                two_minutes[0] + two_minutes[1] >= MIN_WAYPOINTS_PER_TWO_MINUTES
                            }),
                            "{behavior:?} L={body_length_pixels} seed {seed} in \
                             {width_pixels}x{height_pixels}: waypoints per minute {per_minute:?}"
                        );
                    }
                }
            }
        }
    }

    const STEP_SECONDS: f64 = 1.0 / 60.0;
    const PONDS: [[f64; 2]; 2] = [[480.0, 270.0], [960.0, 540.0]];
    /// A young koi, a grown one, the adult length, and the largest koi.
    const BODY_LENGTHS: [f64; 4] = [42.0, 99.4, 137.5, 167.5];
    const BEHAVIORS: [Behavior; 5] = [
        Behavior::Cruise,
        Behavior::Dart,
        Behavior::Rest,
        Behavior::DeepRest,
        Behavior::SurfaceFeeding,
    ];
    /// Wandered start states per pond and size. The sweeps are heavy for unoptimized
    /// builds, so debug test runs take every eighth state; `cargo test --release` runs
    /// them all.
    const WANDERED_STATES: u64 = if cfg!(debug_assertions) { 25 } else { 200 };
    const WANDERED_STRIDE: u64 = 200 / WANDERED_STATES;

    /// A koi that has swum freely for up to two minutes, from varied seeds, behaviors,
    /// and rest points, so tests start from wherever wandering leaves it.
    fn wander(state_index: u64, pond_size: [f64; 2], body_length_pixels: f64) -> SwimState {
        let mut random_state = state_index.wrapping_mul(0x5851_f42d_4c95_7f2d);
        let pond = Pond {
            width_pixels: pond_size[0],
            height_pixels: pond_size[1],
            rest_point: pond_size.map(|size| draw_fraction(&mut random_state) * size),
        };
        let mut swim_state = SwimState::new(state_index, pond, body_length_pixels);
        let behavior = BEHAVIORS[state_index as usize % BEHAVIORS.len()];
        let wander_steps = (draw_fraction(&mut random_state) * 120.0 * 60.0) as usize;
        for _ in 0..wander_steps {
            swim_state.step(STEP_SECONDS, behavior, None);
        }
        swim_state
    }

    /// Corners, edge midpoints, and points anywhere in the pond, in turn.
    fn choose_point(state_index: u64, pond_size: [f64; 2], random_state: &mut u64) -> [f64; 2] {
        let [width, height] = pond_size;
        let turn = (state_index / 3 % 4) as usize;
        match state_index % 3 {
            0 => [[0.0, 0.0], [width, 0.0], [0.0, height], [width, height]][turn],
            1 => [
                [width / 2.0, 0.0],
                [width / 2.0, height],
                [0.0, height / 2.0],
                [width, height / 2.0],
            ][turn],
            _ => pond_size.map(|size| draw_fraction(random_state) * size),
        }
    }

    #[test]
    fn food_anywhere_is_eaten_from_wandered_states() {
        // Even a pond diagonal, about 1,100 px, takes under 30 s at food speed.
        const GULP_SECONDS_LIMIT: f64 = 30.0;
        for pond_size in PONDS {
            for body_length_pixels in BODY_LENGTHS {
                let mut random_state = 5;
                for state_index in (0..200).step_by(WANDERED_STRIDE as usize) {
                    let food = choose_point(state_index, pond_size, &mut random_state);
                    let behavior = BEHAVIORS[(state_index / 2) as usize % BEHAVIORS.len()];
                    let mut swim_state = wander(state_index, pond_size, body_length_pixels);
                    let gulped = (0..(GULP_SECONDS_LIMIT * 60.0) as usize).any(|_| {
                        swim_state.step(STEP_SECONDS, behavior, Some(food)) == SwimEvent::Gulped
                    });
                    assert!(
                        gulped,
                        "{pond_size:?} L={body_length_pixels} state {state_index} {behavior:?}: \
                         food {food:?} not eaten, head at {:?}",
                        swim_state.head
                    );
                }
            }
        }
    }

    #[test]
    fn rest_points_at_edges_settle_from_wandered_states() {
        for pond_size in PONDS {
            for body_length_pixels in BODY_LENGTHS {
                for state_index in (0..120).step_by(WANDERED_STRIDE as usize) {
                    let [width, height] = pond_size;
                    let rest_point = [
                        [0.0, 0.0],
                        [width, 0.0],
                        [0.0, height],
                        [width, height],
                        [width / 2.0, 0.0],
                        [width / 2.0, height],
                        [0.0, height / 2.0],
                        [width, height / 2.0],
                    ][state_index as usize / WANDERED_STRIDE as usize % 8];
                    let behavior = BEHAVIORS[2 + state_index as usize % 3];
                    let mut swim_state = wander(state_index, pond_size, body_length_pixels);
                    swim_state.pond.rest_point = rest_point;
                    let settled_point = swim_state.clamp_to_area(rest_point);
                    // Time to swim there at cruising speed, plus time to slow and line up.
                    let limit_seconds = (distance(swim_state.head, settled_point)
                        / CRUISE_SPEED_PIXELS_PER_SECOND
                        + 20.0)
                        .max(60.0);
                    for _ in 0..(limit_seconds * 60.0) as usize {
                        swim_state.step(STEP_SECONDS, behavior, None);
                    }
                    let head_before = swim_state.head;
                    swim_state.step(STEP_SECONDS, behavior, None);
                    let speed = distance(head_before, swim_state.head) / STEP_SECONDS;
                    assert!(
                        distance(swim_state.head, settled_point) < 6.0 && speed < 1.0,
                        "{pond_size:?} L={body_length_pixels} state {state_index} {behavior:?}: \
                         rest {rest_point:?} -> {settled_point:?}, at {:?} moving {speed} px/s \
                         after {limit_seconds:.0} s",
                        swim_state.head
                    );
                }
            }
        }
    }

    #[test]
    fn food_behind_beyond_an_edge_is_turned_for_the_long_way_round() {
        // Heading down and right toward the bottom edge, with food far behind on the left
        // edge near the bottom corner: the short way round turns into the bottom edge,
        // which used to hold the koi swimming along it, away from the food, for 10 s.
        let pond = Pond {
            width_pixels: 960.0,
            height_pixels: 540.0,
            rest_point: [480.0, 270.0],
        };
        let mut swim_state = SwimState::new(110, pond, 137.5);
        swim_state.head = [565.0, 391.1];
        swim_state.heading_radians = 0.8;
        swim_state.speed_pixels_per_second = 22.3;
        let food = [44.8, 454.9];
        // About 480 px at food speed is 12 s; turning around adds a few more.
        let gulp_step = (0..25 * 60).find(|_| {
            swim_state.step(STEP_SECONDS, Behavior::Cruise, Some(food)) == SwimEvent::Gulped
        });
        assert!(
            gulp_step.is_some_and(|step_index| step_index < 20 * 60),
            "gulped after {gulp_step:?} steps, head at {:?}",
            swim_state.head
        );
    }
}
