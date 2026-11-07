//! Time `render_swim_frame` per frame across growth, for the koi pond that renders every
//! animation frame, both as one call per frame and through a `KoiRenderer` built once
//! and kept across frames. Run with `cargo run --release --locked --example render-speed`.

use std::hint::black_box;
use std::time::Instant;

use git_koi::behavior::Behavior;
use git_koi::coat::KoiBody;
use git_koi::history::AccountSnapshot;
use git_koi::koi::derive_koi_development;
use git_koi::render::{KoiRenderer, render_swim_frame};

const GROWTH_TOTALS: [u64; 4] = [14, 1_500, 12_750, 100_000];
const BEHAVIORS: [(Behavior, &str); 2] = [(Behavior::Cruise, "Cruise"), (Behavior::Rest, "Rest")];
const TIMED_FRAME_COUNT: u32 = 200;

fn main() {
    let account_snapshot: AccountSnapshot =
        serde_json::from_str(include_str!("../tests/fixtures/account-with-history.json"))
            .expect("the fixture is a valid snapshot");
    let mut koi_development =
        derive_koi_development(&account_snapshot).expect("the fixture develops a koi");
    for total_contributions in GROWTH_TOTALS {
        koi_development.total_contributions = total_contributions;
        for (behavior, behavior_name) in BEHAVIORS {
            black_box(render_swim_frame(
                &koi_development,
                KoiBody::default(),
                behavior,
                0,
            ));
            let started_at = Instant::now();
            for frame_index in 0..TIMED_FRAME_COUNT {
                black_box(render_swim_frame(
                    black_box(&koi_development),
                    KoiBody::default(),
                    behavior,
                    frame_index,
                ));
            }
            let frame_milliseconds =
                started_at.elapsed().as_secs_f64() * 1_000.0 / f64::from(TIMED_FRAME_COUNT);
            println!(
                "total={total_contributions} {behavior_name}: {frame_milliseconds:.2} ms/frame"
            );
            let koi_renderer = KoiRenderer::new(&koi_development, KoiBody::default());
            black_box(koi_renderer.render_swim_frame(behavior, 0));
            let started_at = Instant::now();
            for frame_index in 0..TIMED_FRAME_COUNT {
                black_box(black_box(&koi_renderer).render_swim_frame(behavior, frame_index));
            }
            let frame_milliseconds =
                started_at.elapsed().as_secs_f64() * 1_000.0 / f64::from(TIMED_FRAME_COUNT);
            println!(
                "cached total={total_contributions} {behavior_name}: {frame_milliseconds:.2} ms/frame"
            );
        }
    }
}
