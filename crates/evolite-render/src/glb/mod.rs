//! 3D models (binary glTF, `.glb`) as a turntable: the model turns once about its vertical axis
//! in [`TURN_MS`], drawn as frames on the same 50 ms grid as animated SVG, so the app plays it
//! like any other animation. [`parse`] reads the file with every way out closed, [`draw`]
//! rasterizes on the processor.

mod draw;
mod parse;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::frames::{Limits, PackedArt, PackedFrame, RenderError};

/// One full turn.
const TURN_MS: u64 = 6_000;

/// Each thread holds a frame in the making (8 MB at 512 px): four at most, so they and the
/// finished frames stay within the worker's memory cap.
fn threads(limits: &Limits) -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    limits.threads.clamp(1, cores).min(4)
}

/// The turn's frames, each packed by the thread that drew it: a frame is unpacked only for the
/// moment it is drawn and packed.
pub(crate) fn render(bytes: &[u8], limits: &Limits) -> Result<PackedArt, RenderError> {
    let model = parse::parse(bytes)?;
    let frame = draw::Frame::of(&model);
    let mut side = limits
        .svg_size
        .unwrap_or(limits.max_animation_side)
        .min(limits.max_animation_side)
        .min(limits.max_side)
        .max(1) as usize;
    let frame_ms = (1000 / limits.animation_fps.max(1)).max(1) as u64;
    let max_frames = limits.max_frames.max(1) as u64;
    // The step a whole number of grid steps, the frames within the limits.
    let mut step = frame_ms * TURN_MS.div_ceil(frame_ms).div_ceil(max_frames).max(1);
    let angle = |t_ms: u64| (t_ms as f32 / TURN_MS as f32) * std::f32::consts::TAU;

    let started = Instant::now();
    let first = draw::draw(&model, &frame, 0.0, &mut draw::Canvas::new(side));
    if limits.first_frame_only {
        return Ok(PackedArt {
            width: side as u32,
            height: side as u32,
            frames: vec![PackedFrame::pack(&first, step as u32)],
        });
    }
    // Fewer frames when drawing is slow, as for SVG: the turn keeps its pace.
    let per_frame = started.elapsed().as_millis().max(1) as u64;
    let budget = limits.max_render_ms.max(1) as u64;
    let threads = threads(limits);
    let count = TURN_MS.div_ceil(step);
    if count * per_frame > budget * threads as u64 {
        step *= (count * per_frame).div_ceil(budget * threads as u64);
    }
    let count = TURN_MS.div_ceil(step) as usize;
    while side * side * 4 * count > limits.max_total && side > 1 {
        side = side * 3 / 4;
    }
    let first = if first.len() == side * side * 4 {
        lz4_flex::block::compress(&first)
    } else {
        lz4_flex::block::compress(&draw::draw(
            &model,
            &frame,
            0.0,
            &mut draw::Canvas::new(side),
        ))
    };

    let mut drawn: Vec<Option<Vec<u8>>> = vec![None; count];
    drawn[0] = Some(first);
    let next = AtomicUsize::new(1);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    // One canvas for all the frames this thread draws.
                    let mut canvas = draw::Canvas::new(side);
                    while started.elapsed().as_millis() as u64 <= budget {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= count {
                            break;
                        }
                        let rgba = draw::draw(&model, &frame, angle(i as u64 * step), &mut canvas);
                        mine.push((i, lz4_flex::block::compress(&rgba)));
                    }
                    mine
                })
            })
            .collect();
        for worker in workers {
            for (i, packed) in worker
                .join()
                .unwrap_or_else(|e| std::panic::resume_unwind(e))
            {
                drawn[i] = Some(packed);
            }
        }
    });

    // Frames up to the first not drawn in time; the last one stays to the end of the turn.
    // Packing is deterministic: the same picture packs to the same bytes.
    let mut frames: Vec<PackedFrame> = Vec::new();
    for (i, lz4) in drawn.into_iter().map_while(|f| f).enumerate() {
        let end = ((i as u64 + 1) * step).min(TURN_MS);
        let delay = (end - i as u64 * step) as u32;
        match frames.last_mut() {
            Some(last) if last.lz4 == lz4 => last.delay_ms += delay,
            _ => frames.push(PackedFrame {
                lz4,
                delay_ms: delay,
            }),
        }
    }
    let shown: u64 = frames.iter().map(|f| u64::from(f.delay_ms)).sum();
    if let Some(last) = frames.last_mut() {
        last.delay_ms += (TURN_MS - shown.min(TURN_MS)) as u32;
    }
    Ok(PackedArt {
        width: side as u32,
        height: side as u32,
        frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring() -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../evolite-core/assets/nft/diamond-ring-3d.glb"),
        )
        .unwrap()
    }

    fn limits(side: u32) -> Limits {
        Limits::default().for_side(side)
    }

    #[test]
    fn the_ring_turns_once_in_six_seconds() {
        let art = render(&ring(), &limits(128)).unwrap().unpacked().unwrap();
        assert_eq!((art.width, art.height), (128, 128));
        assert!(art.frames.len() > 20, "{} frames", art.frames.len());
        assert_eq!(art.frames.iter().map(|f| f.delay_ms).sum::<u32>(), 6000);
        // On the app's 50 ms clock.
        assert!(art.frames.iter().all(|f| f.delay_ms % 50 == 0));
        // Something is drawn, and the corners stay transparent.
        let rgba = &art.frames[0].rgba;
        let inked = rgba.chunks(4).filter(|p| p[3] > 0).count();
        assert!(inked > 128 * 128 / 10, "{inked} pixels");
        assert_eq!(&rgba[..4], &[0, 0, 0, 0]);
        // It turns: the frames differ.
        assert_ne!(art.frames[0].rgba, art.frames[art.frames.len() / 3].rgba);
    }

    #[test]
    fn the_still_pass_is_one_frame_with_a_delay() {
        let first = Limits {
            first_frame_only: true,
            ..limits(64)
        };
        let art = render(&ring(), &first).unwrap();
        assert_eq!(art.frames.len(), 1);
        assert!(art.frames[0].delay_ms > 0);
    }
}
