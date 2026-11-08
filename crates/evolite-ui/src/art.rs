//! NFT pictures on screen. The interface never decodes them: it asks the core for a picture
//! drawn as large as it is shown (`Command::DrawArt`), and the core's sandboxed renderer sends
//! packed frames back (`Event::Art`). Here each picture is one texture, refilled with the
//! frame whose time has come.

use crate::theme::{Palette, Weight, sans};
use egui::{Align2, ColorImage, CornerRadius, Rect, TextureHandle, TextureOptions, Ui, pos2, vec2};
use evolite_api::{Command, CoreError, Picture, PicturesOff};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Sides are asked for in steps of this many pixels, so a window resized by a few pixels does
/// not redraw every picture.
const STEP: u32 = 32;
const MAX_SIDE: u32 = 1024;
/// The clock animations are shown on: the renderer's frames lie on a grid of this step
/// (`Limits::animation_fps`, 20 a second).
const TICK_MS: u64 = 50;

enum Shown {
    /// Asked for; `urgent` when it was on screen then.
    Waiting {
        urgent: bool,
    },
    Ready {
        /// The frames, packed; only the one on screen is unpacked, into `texture`.
        picture: Arc<Picture>,
        texture: TextureHandle,
        /// Which frame `texture` holds.
        frame: usize,
        /// One loop, in milliseconds; 0 for a still picture.
        total: u64,
        /// The first frame of an animation that is still being drawn.
        partial: bool,
        /// When it was last on screen (egui time, seconds).
        seen: f64,
    },
    Failed(String),
}

#[derive(Default)]
pub struct ArtCache {
    pictures: HashMap<(String, u32), Shown>,
    /// One unpacked frame, reused.
    scratch: Vec<u8>,
    /// Animations stand on their first frame (Settings → Collection, or the system's choice).
    pub still: bool,
    /// This system cannot draw pictures from outside safely: said once, on the Collection.
    pub off: Option<PicturesOff>,
}

/// The pixel side for a picture `points` wide on this screen.
pub fn side_for(points: f32, pixels_per_point: f32) -> u32 {
    let pixels = (points * pixels_per_point).ceil().max(1.0) as u32;
    pixels.div_ceil(STEP).saturating_mul(STEP).min(MAX_SIDE)
}

impl ArtCache {
    /// Some picture asked for has not come back yet, or not whole (a screenshot waits).
    #[cfg(any(feature = "dev-tools", test))]
    pub fn busy(&self) -> bool {
        self.pictures.values().any(|s| {
            matches!(
                s,
                Shown::Waiting { .. } | Shown::Ready { partial: true, .. }
            )
        })
    }

    pub fn on_answer(
        &mut self,
        ctx: &egui::Context,
        art: &str,
        side: u32,
        partial: bool,
        result: &Result<Arc<Picture>, CoreError>,
    ) {
        let key = (art.to_owned(), side);
        // A first frame late after the whole picture changes nothing.
        if partial
            && matches!(
                self.pictures.get(&key),
                Some(Shown::Ready { partial: false, .. })
            )
        {
            return;
        }
        let shown = match result {
            Ok(picture) => self.first_frame(ctx, art, side, partial, picture),
            Err(CoreError::PicturesOff(why)) => {
                self.off = Some(*why);
                Shown::Failed(tr!("not drawn on this system").to_owned())
            }
            Err(CoreError::Failed(why) | CoreError::NotAvailable(why)) => {
                Shown::Failed(why.clone())
            }
            Err(_) => Shown::Failed(String::new()),
        };
        // A whole picture at a new size replaces the old ones: one texture per picture. A first
        // frame does not: the animation at the old size plays on until the new one is whole.
        if matches!(shown, Shown::Ready { partial: false, .. }) {
            self.pictures.retain(|(a, s), _| a != art || *s == side);
        }
        self.pictures.insert(key, shown);
    }

    /// Lets go of pictures not on screen for `secs`: their textures and frames. Drawn again
    /// when they come back, from the disk cache in a moment.
    pub fn forget_unseen(&mut self, now: f64, secs: f64) {
        self.pictures.retain(|_, shown| match shown {
            Shown::Ready { seen, .. } => now - *seen < secs,
            _ => true,
        });
    }

    /// What to show for `art` at `side`: a whole picture before a first frame, this size
    /// before another.
    fn best(&self, art: &str, side: u32) -> Option<(String, u32)> {
        self.pictures
            .iter()
            .filter_map(|((a, s), shown)| match shown {
                Shown::Ready { partial, .. } if a == art => Some(((!partial, *s == side), *s)),
                _ => None,
            })
            .max()
            .map(|(_, s)| (art.to_owned(), s))
    }

    fn first_frame(
        &mut self,
        ctx: &egui::Context,
        art: &str,
        side: u32,
        partial: bool,
        picture: &Arc<Picture>,
    ) -> Shown {
        let Some(image) = unpack(picture, 0, &mut self.scratch) else {
            return Shown::Failed(String::new());
        };
        let total = if picture.frames.len() > 1 {
            picture.frames.iter().map(|f| u64::from(f.delay_ms)).sum()
        } else {
            0
        };
        Shown::Ready {
            picture: picture.clone(),
            texture: ctx.load_texture(
                format!("nft-art/{art}/{side}"),
                image,
                TextureOptions::LINEAR,
            ),
            frame: 0,
            total,
            partial,
            seen: ctx.input(|i| i.time),
        }
    }

    /// Paints `art` into `rect`, asking the core for it at this size when it has not been
    /// drawn yet. Until then, the same picture at another size, or a placeholder.
    pub fn paint(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        art: &str,
        rect: Rect,
        send: &mut impl FnMut(Command),
    ) {
        match self.texture(ui, art, rect, send) {
            Prepared::Failed(why) => placeholder(ui, p, rect, tr!("Could not show it"), &why),
            Prepared::Waiting => placeholder(ui, p, rect, tr!("Drawing…"), ""),
            Prepared::Ready(id, size) => {
                // The picture's own proportions, centred in the square.
                let scale = (rect.width() / size.x).min(rect.height() / size.y);
                let at = Rect::from_center_size(rect.center(), size * scale);
                ui.painter().image(
                    id,
                    at,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }
    }

    /// Paints `art` as a round picture filling `rect` (a profile picture), cut to the square
    /// in its middle. Returns false while it is not drawn, or could not be: the caller paints
    /// its own stand-in.
    pub fn paint_round(
        &mut self,
        ui: &mut Ui,
        art: &str,
        rect: Rect,
        send: &mut impl FnMut(Command),
    ) -> bool {
        let Prepared::Ready(id, size) = self.texture(ui, art, rect, send) else {
            return false;
        };
        // Cover: the shorter side fills the circle.
        let (w, h) = if size.x > size.y {
            (size.y / size.x, 1.0)
        } else {
            (1.0, size.x / size.y)
        };
        let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(w, h));
        let radius = (rect.width().min(rect.height()) / 2.0) as u8;
        ui.painter().add(
            egui::epaint::RectShape::filled(rect, CornerRadius::same(radius), egui::Color32::WHITE)
                .with_texture(id, uv),
        );
        true
    }

    /// The texture of `art` for `rect` this pass: asks the core when this size is not drawn
    /// yet, moves an animation to its current frame.
    fn texture(
        &mut self,
        ui: &mut Ui,
        art: &str,
        rect: Rect,
        send: &mut impl FnMut(Command),
    ) -> Prepared {
        let side = side_for(rect.width().max(rect.height()), ui.ctx().pixels_per_point());
        let key = (art.to_owned(), side);
        let on_screen = ui.is_rect_visible(rect);
        let ask = match self.pictures.get(&key) {
            None => true,
            // Asked for off screen, on screen now: asked again, before the others.
            Some(Shown::Waiting { urgent: false }) => on_screen,
            Some(Shown::Failed(why)) => return Prepared::Failed(why.clone()),
            Some(_) => false,
        };
        if ask {
            self.pictures
                .insert(key.clone(), Shown::Waiting { urgent: on_screen });
            send(Command::DrawArt {
                art: art.to_owned(),
                side,
                urgent: on_screen,
            });
        }
        let Some(key) = self.best(art, side) else {
            return Prepared::Waiting;
        };
        // Only a picture on screen moves, and none while the window is minimized.
        let visible =
            ui.is_rect_visible(rect) && !ui.input(|i| i.viewport().minimized.unwrap_or(false));
        let Some(Shown::Ready {
            picture,
            texture,
            frame,
            total,
            seen,
            ..
        }) = self.pictures.get_mut(&key)
        else {
            return Prepared::Waiting;
        };
        if visible {
            *seen = ui.input(|i| i.time);
        }
        // One frame unpacked when its time comes, on the clock all pictures share; with
        // animations off, the first frame and no more passes.
        let target = if self.still {
            Some((0, None))
        } else if visible && *total > 0 {
            // egui draws a pass one frame (`predicted_dt`) before it reaches the screen, and starts
            // a delayed pass that much early; a delay shorter than that is a pass now, twice. So
            // the frame is chosen for the moment it is shown, and the delay asked for has that
            // frame added back: the window is drawn once per tick, not on every refresh.
            let ahead = ui.input(|i| i.predicted_dt).max(0.0);
            let now_ms = ((ui.input(|i| i.time) + f64::from(ahead)) * 1000.0) as u64;
            let (now, wait) = current(now_ms, picture, *total);
            Some((now, Some(wait + Duration::from_secs_f32(ahead))))
        } else {
            None
        };
        if let Some((now, wait)) = target {
            if now != *frame {
                match unpack(picture, now, &mut self.scratch) {
                    Some(image) => {
                        texture.set(image, TextureOptions::LINEAR);
                        *frame = now;
                    }
                    None => {
                        self.pictures.insert(key, Shown::Failed(String::new()));
                        return Prepared::Failed(String::new());
                    }
                }
            }
            if let Some(wait) = wait {
                ui.ctx().request_repaint_after(wait);
            }
        }
        Prepared::Ready(texture.id(), texture.size_vec2())
    }
}

/// Why no picture from outside is drawn here, what it does not touch and what can be done.
pub fn pictures_off_text(why: PicturesOff) -> &'static str {
    match why {
        PicturesOff::NoFileIsolation => tr!(
            "This system cannot isolate pictures from outside safely, so NFT pictures and profile photos are not drawn. Your wallet and funds are not affected. To see them, use a Linux kernel with Landlock enabled (5.13 or newer)."
        ),
        PicturesOff::NoMemoryCap => tr!(
            "macOS would not limit the memory of the process that draws pictures from outside, so NFT pictures and profile photos are not drawn. Your wallet and funds are not affected. Updating macOS may help; if it does not, please report this."
        ),
        PicturesOff::NoSandbox => tr!(
            "This system would not set up the sandbox for pictures from outside, so NFT pictures and profile photos are not drawn. Your wallet and funds are not affected. Updating the system may help; if it does not, please report this."
        ),
    }
}

/// A picture for one pass: its texture and pixel size, or why there is none.
enum Prepared {
    Waiting,
    Failed(String),
    Ready(egui::TextureId, egui::Vec2),
}

/// Frame `i` as an image; `None` if it does not unpack to its size.
fn unpack(picture: &Picture, i: usize, scratch: &mut Vec<u8>) -> Option<ColorImage> {
    picture.unpack(i, scratch).then(|| {
        // Frames come premultiplied, as textures take them: no pass over the pixels here.
        ColorImage::from_rgba_premultiplied(
            [picture.width as usize, picture.height as usize],
            scratch,
        )
    })
}

/// The frame to show at `now_ms`, and how long until the window must be drawn again for the
/// next one. Time goes in ticks of [`TICK_MS`], the step of the renderer's frame grid: every
/// animation changes on the same tick, so the window is drawn at most 20 times a second
/// however many of them play, and no frame is dropped or doubled.
fn current(now_ms: u64, picture: &Picture, total: u64) -> (usize, Duration) {
    let tick = now_ms / TICK_MS * TICK_MS;
    let pos = tick % total;
    let mut start = 0;
    for (i, frame) in picture.frames.iter().enumerate() {
        let end = start + u64::from(frame.delay_ms);
        if pos < end {
            // The first tick at or after this frame's end.
            let next = (tick - pos + end).div_ceil(TICK_MS) * TICK_MS;
            return (i, Duration::from_millis(next - now_ms));
        }
        start = end;
    }
    (0, Duration::from_millis(TICK_MS - now_ms % TICK_MS))
}

fn placeholder(ui: &Ui, p: &Palette, rect: Rect, title: &str, why: &str) {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(12), p.raised);
    painter.text(
        rect.center() - vec2(0.0, if why.is_empty() { 0.0 } else { 9.0 }),
        Align2::CENTER_CENTER,
        crate::i18n::t(title),
        sans(13.0, Weight::Medium),
        p.muted,
    );
    if !why.is_empty() {
        painter.text(
            rect.center() + vec2(0.0, 10.0),
            Align2::CENTER_CENTER,
            crate::i18n::t(why),
            sans(11.0, Weight::Regular),
            p.muted,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::side_for;

    #[test]
    fn frames_change_on_the_shared_tick() {
        use evolite_api::{Picture, PictureFrame};
        let picture = Picture {
            width: 1,
            height: 1,
            frames: [100, 50, 80]
                .into_iter()
                .map(|d| PictureFrame::pack(&[0; 4], d))
                .collect(),
        };
        let at = |ms| {
            let (i, wait) = super::current(ms, &picture, 230);
            (i, wait.as_millis() as u64)
        };
        // Frame 0 until 100 ms; the window wakes on the tick it ends.
        assert_eq!(at(0), (0, 100));
        assert_eq!(at(30), (0, 70));
        assert_eq!(at(120), (1, 30));
        // 80 ms of frame 2 ends at 230, between ticks: the next draw is on the tick at 250.
        assert_eq!(at(160), (2, 90));
        // The second loop starts at 230: the tick at 250 shows its frame 0, which ends at 330,
        // so the next draw is on the tick at 350.
        assert_eq!(at(250), (0, 100));
    }

    #[test]
    fn the_whole_picture_plays_on_while_a_new_size_is_drawn() {
        use evolite_api::{Picture, PictureFrame};
        use std::sync::Arc;
        let ctx = egui::Context::default();
        let picture = |frames: usize| {
            Ok(Arc::new(Picture {
                width: 1,
                height: 1,
                frames: (0..frames)
                    .map(|_| PictureFrame::pack(&[9; 4], 50))
                    .collect(),
            }))
        };
        let mut cache = super::ArtCache::default();
        cache.on_answer(&ctx, "a", 64, false, &picture(3));
        // The window grew: the first frame at 96 comes, the animation at 64 stays on screen.
        cache.on_answer(&ctx, "a", 96, true, &picture(1));
        assert_eq!(cache.best("a", 96), Some(("a".into(), 64)));
        assert!(cache.busy());
        // Whole at 96: it is shown, and the old size is let go.
        cache.on_answer(&ctx, "a", 96, false, &picture(3));
        assert_eq!(cache.best("a", 96), Some(("a".into(), 96)));
        assert_eq!(cache.pictures.len(), 1);
        assert!(!cache.busy());
        // A first frame late after the whole picture changes nothing.
        cache.on_answer(&ctx, "a", 96, true, &picture(1));
        assert!(!cache.busy());
        // Not on screen for a minute: let go.
        cache.forget_unseen(30.0, 60.0);
        assert_eq!(cache.pictures.len(), 1);
        cache.forget_unseen(61.0, 60.0);
        assert!(cache.pictures.is_empty());
    }

    #[test]
    fn sides_go_up_in_steps_and_stop_at_the_cap() {
        assert_eq!(side_for(150.0, 1.0), 160);
        assert_eq!(side_for(150.0, 1.5), 256);
        assert_eq!(side_for(160.0, 1.0), 160);
        assert_eq!(side_for(2000.0, 2.0), 1024);
        assert_eq!(side_for(0.0, 1.0), 32);
    }
}
