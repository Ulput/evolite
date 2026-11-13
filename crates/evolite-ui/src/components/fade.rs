//! A fade at the bottom of a scrolled list or part: there is more below (FEAT-0036, BUG-122).

use super::prelude::*;

/// A fade `height` high from clear to `color`, its bottom on the bottom of `view`. Only painted,
/// so what is under it stays clickable. Its rect.
pub fn fade_up(ui: &Ui, view: Rect, color: Color32, height: f32) -> Rect {
    let band = Rect::from_min_max(egui::pos2(view.left(), view.bottom() - height), view.max);
    let mut mesh = egui::Mesh::default();
    let clear = Color32::TRANSPARENT;
    mesh.colored_vertex(band.left_top(), clear);
    mesh.colored_vertex(band.right_top(), clear);
    mesh.colored_vertex(band.left_bottom(), color);
    mesh.colored_vertex(band.right_bottom(), color);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    ui.painter().add(egui::Shape::mesh(mesh));
    band
}

/// Whether a scroll area's content goes on below what it shows.
pub fn more_below<R>(shown: &egui::scroll_area::ScrollAreaOutput<R>) -> bool {
    shown.state.offset.y + shown.inner_rect.height() < shown.content_size.y - 0.5
}
