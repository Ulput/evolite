//! A profile picture, as the Figma component `avatar` (S 36, M 44, L 72): the picture the core
//! drew from the profile's link, or until then (and without one) the initial on a gradient
//! picked by the identity, so a person keeps their colours.

use super::prelude::*;
use evolite_api::Command;

/// Gradients from the top left to the bottom right, as on the Figma screens.
const TONES: [([u8; 3], [u8; 3]); 5] = [
    ([245, 158, 46], [219, 69, 115]),
    ([64, 128, 242], [26, 191, 179]),
    ([89, 77, 230], [158, 89, 242]),
    ([51, 179, 102], [158, 204, 51]),
    ([250, 191, 38], [242, 115, 26]),
];

/// Paints the avatar of identity `id` in `rect` (a square). `label`: the name its initial
/// comes from; `picture`: the core's key of its picture, when the profile has one.
pub fn avatar(
    ui: &mut Ui,
    a: &mut Assets,
    rect: Rect,
    id: &str,
    label: &str,
    picture: Option<&str>,
    send: &mut impl FnMut(Command),
) {
    if let Some(art) = picture
        && a.avatars.paint_round(ui, art, rect, send)
    {
        return;
    }
    let (from, to) = tone(id);
    gradient_circle(ui.painter(), rect, from, to);
    let initial: String = label
        .trim_start_matches('@')
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        crate::i18n::t(&initial),
        sans((rect.width() * 0.39).round(), Weight::SemiBold),
        Color32::WHITE,
    );
}

/// The gradient of an identity: the same id, the same colours, on every screen.
fn tone(id: &str) -> (Color32, Color32) {
    // FNV-1a: ids differ in every byte, and every byte moves the result.
    let hash = id.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    let (from, to) = TONES[(hash >> 8) as usize % TONES.len()];
    (
        Color32::from_rgb(from[0], from[1], from[2]),
        Color32::from_rgb(to[0], to[1], to[2]),
    )
}

/// A circle filling `rect` with a diagonal gradient: a fan of triangles whose corners carry
/// their colour, then a thin ring for a smooth edge.
fn gradient_circle(painter: &egui::Painter, rect: Rect, from: Color32, to: Color32) {
    const SEGMENTS: u32 = 64;
    let center = rect.center();
    let r = rect.width().min(rect.height()) / 2.0;
    let color_at = |pos: egui::Pos2| {
        let t = ((pos.x - center.x + pos.y - center.y) / (4.0 * r) + 0.5).clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Color32::from_rgb(
            mix(from.r(), to.r()),
            mix(from.g(), to.g()),
            mix(from.b(), to.b()),
        )
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color_at(center));
    for i in 0..=SEGMENTS {
        let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let pos = center + vec2(angle.cos(), angle.sin()) * r;
        mesh.colored_vertex(pos, color_at(pos));
        if i > 0 {
            mesh.add_triangle(0, i, i + 1);
        }
    }
    painter.add(mesh);
    painter.circle_stroke(center, r - 0.5, Stroke::new(1.0, color_at(center)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_person_keeps_their_colours() {
        let id = "3Ah7Kp2Qx9Lm4Vt8Rn5Wc1Jd6Yb2Fs7Ge4Hu3Mz9Lp1";
        assert_eq!(tone(id), tone(id));
        let tones: std::collections::HashSet<_> = ["a", "b", "c", "d", "e", "f", "g", "h"]
            .iter()
            .map(|id| tone(id))
            .collect();
        assert!(tones.len() > 2, "ids spread over the gradients");
    }
}
