//! Today's pixels, pinned by hash, so speedups to the renderer cannot change one of them.

use git_koi::behavior::{Behavior, select_behavior};
use git_koi::coat::KoiBody;
use git_koi::history::AccountSnapshot;
use git_koi::koi::{KoiDevelopment, derive_koi_development};
use git_koi::motion::frame_count;
use git_koi::output::{encode_gif, encode_png, encode_svg};
use git_koi::render::{KoiFrame, render_koi, render_swim_frame};

const BODIES: [KoiBody; 6] = [
    KoiBody::White,
    KoiBody::Black,
    KoiBody::Asagi,
    KoiBody::Tea,
    KoiBody::Gold,
    KoiBody::Platinum,
];
const BEHAVIORS: [Behavior; 5] = [
    Behavior::DeepRest,
    Behavior::Rest,
    Behavior::SurfaceFeeding,
    Behavior::Dart,
    Behavior::Cruise,
];
const GROWTH_TOTALS: [u64; 4] = [14, 1_500, 12_750, 100_000];
const PINNED_FRAME_INDICES: [u32; 3] = [0, 7, 33];

/// FNV-1a hashes of each body's GIF, PNG, and SVG, in `BODIES` order.
const DEFAULT_OUTPUT_HASHES: [[u64; 3]; 6] = [
    [0x21afce715db77894, 0xa11a9e9f64b70bb8, 0x4a28b8c4abda4524],
    [0xed5c36cebe74689e, 0x324c2c6a77366ff2, 0x68f51428907d611a],
    [0xac80bbbe870229d8, 0x1778671044e60f1a, 0xa92e67b537d26dd4],
    [0x36665f0dfa026624, 0x17b66265ddae05bf, 0xaf1cfb5202a6e7a0],
    [0x807dd44f3c061182, 0x047a2003729fa18a, 0xf91a477279cc82ac],
    [0xbfdad3159cc0260f, 0x0c5fe71248481609, 0x9a87a77e2ce4e0f1],
];
/// FNV-1a hashes of frames 0, 7, and 33 combined, by `GROWTH_TOTALS` then `BEHAVIORS`.
const GROWTH_FRAME_HASHES: [[u64; 5]; 4] = [
    [
        0x59bf2dc37b17ca1f,
        0xc63a92c43c70e815,
        0x5588e89008146c81,
        0xedfcbd850387ed64,
        0x17ab99898fc79469,
    ],
    [
        0xffc67df120f2fe05,
        0xb423634462f4ce24,
        0x3a664a0d3b7cf3be,
        0x891f5194dc6f5750,
        0xc04a50b4f62f131f,
    ],
    [
        0x6c5a69d414915d55,
        0x437dc87dc52fa1af,
        0xbaaca8933a9cdf05,
        0x5d478b41096f5c25,
        0x0b743a66dfa9b36a,
    ],
    [
        0x0e2d301e3032a2ae,
        0x570af66da126e85a,
        0x1884e1356d266f42,
        0x16e89e008be65423,
        0x149545294a4e9df1,
    ],
];

#[test]
fn default_outputs_are_unchanged() {
    let koi_development = develop_fixture();
    let behavior = select_behavior(&koi_development);
    let mut output_hashes = Vec::new();
    for koi_body in BODIES {
        let swim_frames: Vec<KoiFrame> = (0..frame_count(behavior))
            .map(|frame_index| render_swim_frame(&koi_development, koi_body, behavior, frame_index))
            .collect();
        let gif_bytes = encode_gif(&swim_frames).unwrap();
        let still_frame = render_koi(&koi_development, koi_body);
        let png_bytes = encode_png(&still_frame).unwrap();
        let svg_text = encode_svg(&still_frame);
        output_hashes.push([
            hash_fnv1a(FNV_OFFSET_BASIS, &gif_bytes),
            hash_fnv1a(FNV_OFFSET_BASIS, &png_bytes),
            hash_fnv1a(FNV_OFFSET_BASIS, svg_text.as_bytes()),
        ]);
    }
    assert_eq!(output_hashes, DEFAULT_OUTPUT_HASHES);
}

#[test]
fn frames_across_growth_are_unchanged() {
    let mut koi_development = develop_fixture();
    let mut frame_hashes = Vec::new();
    for total_contributions in GROWTH_TOTALS {
        koi_development.total_contributions = total_contributions;
        let mut total_hashes = [0; 5];
        for (behavior_index, behavior) in BEHAVIORS.into_iter().enumerate() {
            let mut frames_hash = FNV_OFFSET_BASIS;
            for frame_index in PINNED_FRAME_INDICES {
                let koi_frame =
                    render_swim_frame(&koi_development, KoiBody::default(), behavior, frame_index);
                frames_hash = hash_fnv1a(frames_hash, koi_frame.as_rgba_bytes());
            }
            total_hashes[behavior_index] = frames_hash;
        }
        frame_hashes.push(total_hashes);
    }
    assert_eq!(frame_hashes, GROWTH_FRAME_HASHES);
}

fn develop_fixture() -> KoiDevelopment {
    let account_snapshot: AccountSnapshot =
        serde_json::from_str(include_str!("fixtures/account-with-history.json")).unwrap();
    derive_koi_development(&account_snapshot).unwrap()
}

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Continue a 64-bit FNV-1a hash over the bytes.
fn hash_fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
