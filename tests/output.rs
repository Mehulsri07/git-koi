use std::sync::OnceLock;

use git_koi::behavior::Behavior;
use git_koi::coat::KoiBody;
use git_koi::history::AccountSnapshot;
use git_koi::koi::derive_koi_development;
use git_koi::motion::FRAMES_PER_SECOND;
use git_koi::output::{GifEncodingError, STATIC_HEIGHT_PIXELS, STATIC_WIDTH_PIXELS, encode_gif};
use git_koi::render::{FRAME_WIDTH_PIXELS, KoiFrame, render_swim_frame};

#[test]
fn the_gif_rebuilds_every_frame_exactly_at_twice_the_size() {
    let (koi_frames, gif_bytes) = encoded_feeding_excerpt();
    assert_eq!(encode_gif(koi_frames).unwrap(), *gif_bytes);
    let mut gif_options = gif::DecodeOptions::new();
    gif_options.set_color_output(gif::ColorOutput::Indexed);
    let mut gif_decoder = gif_options.read_info(gif_bytes.as_slice()).unwrap();
    assert_eq!(
        (
            u32::from(gif_decoder.width()),
            u32::from(gif_decoder.height())
        ),
        (STATIC_WIDTH_PIXELS, STATIC_HEIGHT_PIXELS)
    );
    // Composite the way a browser does: draw each frame's box over the canvas, show
    // the result, then apply the frame's disposal before the next one.
    let mut canvas = vec![[0; 4]; (STATIC_WIDTH_PIXELS * STATIC_HEIGHT_PIXELS) as usize];
    for koi_frame in koi_frames {
        let gif_frame = gif_decoder.read_next_frame().unwrap().unwrap();
        let palette = gif_frame.palette.as_ref().unwrap();
        let box_pixels = |box_index: usize| {
            let canvas_x = usize::from(gif_frame.left) + box_index % usize::from(gif_frame.width);
            let canvas_y = usize::from(gif_frame.top) + box_index / usize::from(gif_frame.width);
            canvas_y * STATIC_WIDTH_PIXELS as usize + canvas_x
        };
        for (box_index, &palette_index) in gif_frame.buffer.iter().enumerate() {
            if Some(palette_index) != gif_frame.transparent {
                let color_start = usize::from(palette_index) * 3;
                let [red, green, blue] = palette[color_start..color_start + 3] else {
                    unreachable!()
                };
                canvas[box_pixels(box_index)] = [red, green, blue, 255];
            }
        }
        assert_eq!(canvas, scale_to_export(koi_frame));
        if gif_frame.dispose == gif::DisposalMethod::Background {
            for box_index in 0..gif_frame.buffer.len() {
                canvas[box_pixels(box_index)] = [0; 4];
            }
        }
    }
    assert!(gif_decoder.read_next_frame().unwrap().is_none());
}

#[test]
fn frames_store_only_the_box_around_the_koi() {
    let mut gif_decoder = gif::DecodeOptions::new()
        .read_info(encoded_feeding_excerpt().1.as_slice())
        .unwrap();
    while let Some(gif_frame) = gif_decoder.read_next_frame().unwrap() {
        assert!(u32::from(gif_frame.width) < STATIC_WIDTH_PIXELS);
        assert!(u32::from(gif_frame.height) < STATIC_HEIGHT_PIXELS);
    }
}

#[test]
fn the_gif_repeats_forever_at_fifteen_frames_per_second() {
    let mut gif_decoder = gif::DecodeOptions::new()
        .read_info(encoded_feeding_excerpt().1.as_slice())
        .unwrap();
    let mut frame_delays = Vec::new();
    while let Some(gif_frame) = gif_decoder.read_next_frame().unwrap() {
        frame_delays.push(gif_frame.delay);
    }
    // Each delay is a frame's share of a second, rounded either way, and the rounding
    // never accumulates: six frames at fifteen per second last exactly 0.4 s.
    let frame_hundredths = 100.0 / f64::from(FRAMES_PER_SECOND);
    assert!(
        frame_delays
            .iter()
            .all(|&delay| (f64::from(delay) - frame_hundredths).abs() < 1.0),
        "{frame_delays:?}"
    );
    let total_hundredths: u32 = frame_delays.iter().map(|&delay| u32::from(delay)).sum();
    assert_eq!(total_hundredths, 6 * 100 / FRAMES_PER_SECOND);
    assert_eq!(gif_decoder.repeat(), gif::Repeat::Infinite);
}

#[test]
fn an_empty_loop_is_an_error() {
    assert!(matches!(encode_gif(&[]), Err(GifEncodingError::NoFrames)));
}

/// The first six frames of surface feeding, which draws the mouth, ripples, and pellets,
/// and their GIF, rendered once for every test.
fn encoded_feeding_excerpt() -> &'static (Vec<KoiFrame>, Vec<u8>) {
    static FEEDING_EXCERPT: OnceLock<(Vec<KoiFrame>, Vec<u8>)> = OnceLock::new();
    FEEDING_EXCERPT.get_or_init(|| {
        let account_snapshot: AccountSnapshot =
            serde_json::from_str(include_str!("fixtures/account-with-history.json")).unwrap();
        let koi_development = derive_koi_development(&account_snapshot).unwrap();
        let koi_frames: Vec<KoiFrame> = (0..6)
            .map(|frame_index| {
                render_swim_frame(
                    &koi_development,
                    KoiBody::default(),
                    Behavior::SurfaceFeeding,
                    frame_index,
                )
            })
            .collect();
        let gif_bytes = encode_gif(&koi_frames).unwrap();
        (koi_frames, gif_bytes)
    })
}

/// Repeat each native pixel twice in both dimensions, as the export does.
fn scale_to_export(koi_frame: &KoiFrame) -> Vec<[u8; 4]> {
    let native_pixels = koi_frame.as_rgba_bytes().as_chunks::<4>().0;
    (0..STATIC_WIDTH_PIXELS * STATIC_HEIGHT_PIXELS)
        .map(|export_index| {
            let native_x = export_index % STATIC_WIDTH_PIXELS / 2;
            let native_y = export_index / STATIC_WIDTH_PIXELS / 2;
            native_pixels[(native_y * FRAME_WIDTH_PIXELS + native_x) as usize]
        })
        .collect()
}
