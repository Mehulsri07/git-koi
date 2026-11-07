use std::collections::BTreeSet;
use std::f64::consts::{PI, TAU};

use git_koi::behavior::Behavior;
use git_koi::coat::KoiBody;
use git_koi::history::AccountSnapshot;
use git_koi::koi::{KoiDevelopment, derive_koi_development};
use git_koi::render::{FRAME_HEIGHT_PIXELS, FRAME_WIDTH_PIXELS, KoiRenderer};
use git_koi::swim::{Pond, SwimEvent, SwimState};

const STEP_SECONDS: f64 = 1.0 / 60.0;
const BODY_LENGTH_PIXELS: f64 = 100.0;
const POND: Pond = Pond {
    width_pixels: 480.0,
    height_pixels: 270.0,
    rest_point: [300.0, 180.0],
};
const BEHAVIORS: [Behavior; 5] = [
    Behavior::DeepRest,
    Behavior::Rest,
    Behavior::SurfaceFeeding,
    Behavior::Dart,
    Behavior::Cruise,
];

fn distance(from: [f64; 2], to: [f64; 2]) -> f64 {
    (to[0] - from[0]).hypot(to[1] - from[1])
}

/// The signed turn from one heading to the next, taken the short way around.
fn heading_change(from_radians: f64, to_radians: f64) -> f64 {
    (to_radians - from_radians + PI).rem_euclid(TAU) - PI
}

#[test]
fn a_fresh_koi_heads_right_from_the_pond_center() {
    let mut swim_state = SwimState::new(7, POND, BODY_LENGTH_PIXELS);
    assert_eq!(swim_state.head(), [240.0, 135.0]);
    assert_eq!(swim_state.heading_radians(), 0.0);
    // Heading is the screen angle of travel: zero is right, and positive turns toward
    // +y, which is clockwise on screen because y grows downward.
    for _ in 0..30 {
        let head_before = swim_state.head();
        swim_state.step(STEP_SECONDS, Behavior::Cruise, None);
        let head_after = swim_state.head();
        let travel_radians = (head_after[1] - head_before[1]).atan2(head_after[0] - head_before[0]);
        assert!(
            heading_change(swim_state.heading_radians(), travel_radians).abs() < 1e-9,
            "the head moves along its heading"
        );
    }
}

#[test]
fn turning_never_exceeds_the_turn_limit() {
    for body_length_pixels in [37.5, BODY_LENGTH_PIXELS, 167.5] {
        let mut swim_state = SwimState::new(3, POND, body_length_pixels);
        let food = [140.0, 135.0];
        let mut total_turn_radians = 0.0;
        for _ in 0..600 {
            let heading_before = swim_state.heading_radians();
            swim_state.step(STEP_SECONDS, Behavior::Cruise, Some(food));
            let turn_radians = heading_change(heading_before, swim_state.heading_radians());
            assert!(
                turn_radians.abs() <= 2.4 * STEP_SECONDS + 1e-9,
                "turned {turn_radians} in one step"
            );
            total_turn_radians += turn_radians.abs();
        }
        assert!(
            total_turn_radians > 2.0,
            "the koi turns toward food behind it"
        );
    }
}

#[test]
fn the_koi_reaches_food_and_gulps_once() {
    for behavior in BEHAVIORS {
        let mut swim_state = SwimState::new(11, POND, BODY_LENGTH_PIXELS);
        let food = [120.0, 135.0];
        assert_eq!(distance(swim_state.head(), food), 120.0);
        let mut gulp_steps = Vec::new();
        for step_index in 0..900 {
            if swim_state.step(STEP_SECONDS, behavior, Some(food)) == SwimEvent::Gulped {
                gulp_steps.push(step_index);
            }
        }
        assert_eq!(gulp_steps.len(), 1, "{behavior:?} gulps exactly once");
        assert!(gulp_steps[0] < 600, "{behavior:?} reaches food within 10 s");
        if matches!(behavior, Behavior::Cruise | Behavior::Dart) {
            // Without the food, the koi wanders off again.
            let mut farthest_pixels: f64 = 0.0;
            for _ in 0..1200 {
                assert_eq!(
                    swim_state.step(STEP_SECONDS, behavior, None),
                    SwimEvent::None
                );
                farthest_pixels = farthest_pixels.max(distance(swim_state.head(), food));
            }
            assert!(farthest_pixels > 40.0, "{behavior:?} wanders after eating");
        }
    }
}

#[test]
fn new_food_is_chased_after_a_gulp() {
    let mut swim_state = SwimState::new(5, POND, BODY_LENGTH_PIXELS);
    let mut gulp_count = 0;
    for (food, step_count) in [
        (Some([300.0, 140.0]), 600),
        (None, 1),
        (Some([150.0, 100.0]), 900),
    ] {
        for _ in 0..step_count {
            if swim_state.step(STEP_SECONDS, Behavior::Cruise, food) == SwimEvent::Gulped {
                gulp_count += 1;
            }
        }
    }
    assert_eq!(gulp_count, 2);
}

#[test]
fn the_koi_stays_in_the_pond() {
    for seed in 0..50_u64 {
        let behavior = if seed % 2 == 0 {
            Behavior::Cruise
        } else {
            Behavior::Dart
        };
        let body_length_pixels = [37.5, BODY_LENGTH_PIXELS, 167.5][seed as usize % 3];
        let inset_pixels = 0.6 * body_length_pixels;
        let mut swim_state = SwimState::new(seed, POND, body_length_pixels);
        let mut travelled_pixels = 0.0;
        for _ in 0..3600 {
            let head_before = swim_state.head();
            swim_state.step(STEP_SECONDS, behavior, None);
            let [head_x, head_y] = swim_state.head();
            travelled_pixels += distance(head_before, [head_x, head_y]);
            assert!(
                (inset_pixels..=POND.width_pixels - inset_pixels).contains(&head_x)
                    && (inset_pixels..=POND.height_pixels - inset_pixels).contains(&head_y),
                "seed {seed}: head left the pond at {head_x}, {head_y}"
            );
        }
        assert!(travelled_pixels > 600.0, "seed {seed} keeps swimming");
    }
}

#[test]
fn resting_settles_at_the_rest_point() {
    for behavior in [Behavior::Rest, Behavior::DeepRest, Behavior::SurfaceFeeding] {
        for seed in [1, 2, 3] {
            let mut swim_state = SwimState::new(seed, POND, BODY_LENGTH_PIXELS);
            for _ in 0..1200 {
                swim_state.step(STEP_SECONDS, behavior, None);
            }
            assert!(
                distance(swim_state.head(), POND.rest_point) < 6.0,
                "{behavior:?} rests at the rest point, not {:?}",
                swim_state.head()
            );
            let head_before = swim_state.head();
            swim_state.step(STEP_SECONDS, behavior, None);
            let speed_pixels_per_second = distance(head_before, swim_state.head()) / STEP_SECONDS;
            assert!(
                speed_pixels_per_second < 1.0,
                "{behavior:?} is still moving at {speed_pixels_per_second} px/s"
            );
        }
    }
}

#[test]
fn the_same_seed_swims_the_same_path() {
    let swim_heads = |seed: u64| {
        let mut swim_state = SwimState::new(seed, POND, BODY_LENGTH_PIXELS);
        (0..1500)
            .map(|step_index| {
                let food = (500..700).contains(&step_index).then_some([100.0, 80.0]);
                let behavior = if step_index < 1000 {
                    Behavior::Dart
                } else {
                    Behavior::Rest
                };
                swim_state.step(STEP_SECONDS, behavior, food);
                (swim_state.head(), swim_state.heading_radians())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(swim_heads(42), swim_heads(42));
    assert_ne!(swim_heads(42), swim_heads(43));
}

#[test]
fn long_steps_are_clamped() {
    let mut long_steps = SwimState::new(9, POND, BODY_LENGTH_PIXELS);
    let mut short_steps = SwimState::new(9, POND, BODY_LENGTH_PIXELS);
    long_steps.step(5.0, Behavior::Cruise, None);
    short_steps.step(0.1, Behavior::Cruise, None);
    assert_eq!(long_steps.head(), short_steps.head());
    let head_before = long_steps.head();
    long_steps.step(-1.0, Behavior::Cruise, None);
    long_steps.step(f64::NAN, Behavior::Cruise, None);
    assert_eq!(long_steps.head(), head_before);
}

fn develop_fixture(total_contributions: u64) -> KoiDevelopment {
    let account_snapshot: AccountSnapshot =
        serde_json::from_str(include_str!("fixtures/account-with-history.json")).unwrap();
    let mut koi_development = derive_koi_development(&account_snapshot).unwrap();
    koi_development.total_contributions = total_contributions;
    koi_development
}

/// Whether any opaque pixel of the frame lies within the radius of a pond point.
fn has_opaque_pixel_near(
    rgba_bytes: &[u8],
    top_left: [i32; 2],
    pond_point: [f64; 2],
    radius_pixels: f64,
) -> bool {
    (0..FRAME_HEIGHT_PIXELS).any(|pixel_y| {
        (0..FRAME_WIDTH_PIXELS).any(|pixel_x| {
            let pixel_index = (pixel_y * FRAME_WIDTH_PIXELS + pixel_x) as usize;
            let pixel_center = [
                f64::from(top_left[0]) + f64::from(pixel_x) + 0.5,
                f64::from(top_left[1]) + f64::from(pixel_y) + 0.5,
            ];
            rgba_bytes[pixel_index * 4 + 3] == 255
                && distance(pixel_center, pond_point) <= radius_pixels
        })
    })
}

/// The point the given arc length back along a path listed from its newest point.
fn walk_back(path_points: &[[f64; 2]], arc_pixels: f64) -> [f64; 2] {
    let mut remaining_pixels = arc_pixels;
    for end_points in path_points.windows(2) {
        let segment_pixels = distance(end_points[0], end_points[1]);
        if segment_pixels >= remaining_pixels {
            let fraction = remaining_pixels / segment_pixels;
            return [0, 1].map(|axis_index| {
                end_points[0][axis_index]
                    + (end_points[1][axis_index] - end_points[0][axis_index]) * fraction
            });
        }
        remaining_pixels -= segment_pixels;
    }
    panic!("the path is shorter than {arc_pixels} px");
}

#[test]
fn the_body_follows_the_swum_path() {
    for total_contributions in [14, 1_500, 100_000] {
        let koi_renderer =
            KoiRenderer::new(&develop_fixture(total_contributions), KoiBody::default());
        let body_length_pixels = koi_renderer.body_length_pixels();
        let mut swim_state = SwimState::new(21, POND, body_length_pixels);
        let start = swim_state.head();
        // A fresh trail runs straight back from the start, against the start heading.
        let mut path_points = vec![[start[0] - 2.0 * body_length_pixels, start[1]], start];
        for _ in 0..400 {
            swim_state.step(STEP_SECONDS, Behavior::Cruise, Some([150.0, 90.0]));
            path_points.push(swim_state.head());
        }
        path_points.reverse();
        let free_frame = koi_renderer.render_free_frame(&swim_state, Behavior::Cruise);
        let rgba_bytes = free_frame.frame.as_rgba_bytes();
        assert_eq!(
            rgba_bytes.len(),
            (FRAME_WIDTH_PIXELS * FRAME_HEIGHT_PIXELS * 4) as usize
        );
        let head = swim_state.head();
        assert!(
            has_opaque_pixel_near(rgba_bytes, free_frame.top_left, head, 2.0),
            "{total_contributions}: no koi at the head"
        );
        let body_point = walk_back(&path_points, 0.8 * body_length_pixels);
        assert!(
            has_opaque_pixel_near(rgba_bytes, free_frame.top_left, body_point, 2.0),
            "{total_contributions}: no koi 0.8 body lengths back along the path"
        );
    }
}

#[test]
fn the_frame_centers_the_koi_on_the_pixel_grid() {
    let koi_renderer = KoiRenderer::new(&develop_fixture(1_500), KoiBody::default());
    let mut swim_state = SwimState::new(4, POND, koi_renderer.body_length_pixels());
    for _ in 0..240 {
        swim_state.step(STEP_SECONDS, Behavior::Dart, None);
        let free_frame = koi_renderer.render_free_frame(&swim_state, Behavior::Dart);
        let rgba_bytes = free_frame.frame.as_rgba_bytes();
        let opaque_pixels: Vec<[u32; 2]> = (0..FRAME_HEIGHT_PIXELS)
            .flat_map(|pixel_y| (0..FRAME_WIDTH_PIXELS).map(move |pixel_x| [pixel_x, pixel_y]))
            .filter(|&[pixel_x, pixel_y]| {
                rgba_bytes[((pixel_y * FRAME_WIDTH_PIXELS + pixel_x) * 4 + 3) as usize] == 255
            })
            .collect();
        for axis_index in 0..2 {
            let low = opaque_pixels
                .iter()
                .map(|pixel| pixel[axis_index])
                .min()
                .unwrap();
            let high = opaque_pixels
                .iter()
                .map(|pixel| pixel[axis_index])
                .max()
                .unwrap();
            let middle = f64::from(low + high + 1) / 2.0;
            assert!(
                (middle - f64::from(FRAME_WIDTH_PIXELS) / 2.0).abs() <= 6.0,
                "the koi is off center: {low}..={high}"
            );
            assert!(
                low > 0 && high + 1 < FRAME_WIDTH_PIXELS,
                "the koi is clipped"
            );
        }
    }
}

#[test]
fn free_frames_stay_in_one_gif_palette() {
    for total_contributions in [14, 100_000] {
        let koi_renderer =
            KoiRenderer::new(&develop_fixture(total_contributions), KoiBody::default());
        for behavior in BEHAVIORS {
            let mut swim_state = SwimState::new(8, POND, koi_renderer.body_length_pixels());
            for step_index in 0..240 {
                let food = (step_index < 120).then_some([200.0, 120.0]);
                swim_state.step(STEP_SECONDS, behavior, food);
                if step_index % 20 != 0 {
                    continue;
                }
                let free_frame = koi_renderer.render_free_frame(&swim_state, behavior);
                let mut opaque_colors = BTreeSet::new();
                for pixel_channels in free_frame.frame.as_rgba_bytes().chunks_exact(4) {
                    match pixel_channels[3] {
                        0 => {}
                        255 => {
                            opaque_colors.insert([
                                pixel_channels[0],
                                pixel_channels[1],
                                pixel_channels[2],
                            ]);
                        }
                        alpha => panic!("{behavior:?}: partly transparent pixel {alpha}"),
                    }
                }
                assert!(!opaque_colors.is_empty());
                assert!(
                    opaque_colors.len() <= 255,
                    "{behavior:?}: {} colors",
                    opaque_colors.len()
                );
            }
        }
    }
}
