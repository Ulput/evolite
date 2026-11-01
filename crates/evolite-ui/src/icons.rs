//! Interface icons and the logo as SVG, recoloured for the current theme.
//! Source: assets/icons — the same files as brand/icons/icons.json and brand/svg in spv-wallet.

use crate::theme::Palette;
use egui::load::Bytes;
use egui::{Image, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

const ICONS_JSON: &str = include_str!("../assets/icons/icons.json");
const MARK_SVG: &str = include_str!("../assets/icons/evolite-mark.svg");
const WORDMARK_SVG: &str = include_str!("../assets/icons/evolite-wordmark.svg");
/// The Dash Evolution lockup as the Dash brand guidelines give it
/// (https://www.dash.org/brand-guidelines/): "Transparent White Full" for the dark theme,
/// "Transparent Black Full" for the light one. Used as it is, never recoloured.
const DASH_EVOLUTION_WHITE: &str = include_str!("../assets/brand/dash-evolution-white.svg");
const DASH_EVOLUTION_BLACK: &str = include_str!("../assets/brand/dash-evolution-black.svg");
/// Its viewBox, 554.67 × 230.37.
const DASH_EVOLUTION_ASPECT: f32 = 554.67 / 230.37;

const INK: &str = "#14161C";
const ACCENT: &str = "#C27C00";

pub struct Icons {
    inner: HashMap<String, String>,
    mark: String,
    wordmark: String,
    cache: HashMap<String, Arc<[u8]>>,
}

fn paths(svg: &str) -> String {
    let mut out = String::new();
    let mut rest = svg;
    while let Some(start) = rest.find("<path ") {
        let end = rest[start..]
            .find("/>")
            .map(|e| start + e + 2)
            .unwrap_or(rest.len());
        out.push_str(&rest[start..end]);
        rest = &rest[end..];
    }
    out
}

impl Icons {
    pub fn load() -> Self {
        let json: serde_json::Value = serde_json::from_str(ICONS_JSON).expect("icons.json");
        let inner = json
            .as_object()
            .expect("icons.json is an object")
            .iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
            .collect();
        Self {
            inner,
            mark: paths(MARK_SVG),
            wordmark: paths(WORDMARK_SVG),
            cache: HashMap::new(),
        }
    }

    fn bytes(&mut self, key: String, svg: impl FnOnce() -> String) -> (String, Bytes) {
        // '#' would start a URI fragment and the loader would not see the `.svg` ending.
        let key = key.replace('#', "");
        let data = self
            .cache
            .entry(key.clone())
            .or_insert_with(|| svg().into_bytes().into());
        (format!("bytes://{key}.svg"), Bytes::Shared(data.clone()))
    }

    /// A 24 × 24 icon drawn in `ink` (or another colour) with its accent parts in `accent`.
    /// Right to left (Arabic) pictures are not mirrored (`rtl`), so the ones that point along
    /// the line swap here: "›" opens to the left, "‹" goes back to the right.
    pub fn icon(
        &mut self,
        name: &str,
        color: egui::Color32,
        p: &Palette,
        size: f32,
    ) -> Image<'static> {
        let name = match name {
            "chevron-left" if crate::i18n::rtl() => "chevron-right",
            "chevron-right" if crate::i18n::rtl() => "chevron-left",
            other => other,
        };
        let ink = crate::theme::to_hex(color);
        let accent = p.accent_hex();
        let body = self.inner.get(name).cloned().unwrap_or_default();
        let (uri, bytes) = self.bytes(format!("icon-{name}-{ink}-{accent}"), || {
            format!(
                r#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">{}</svg>"#,
                body.replace(INK, &ink).replace(ACCENT, &accent)
            )
        });
        Image::from_bytes(uri, bytes).fit_to_exact_size(Vec2::splat(size))
    }

    /// Lockups as in brand/README.md: horizontal 212 × 56, stacked 117 × 70, mark 64 × 64.
    pub fn logo(&mut self, kind: Logo, p: &Palette, height: f32) -> Image<'static> {
        let (ink, accent) = (p.ink_hex(), p.brand_hex());
        let mark = self.mark.replace(INK, &ink).replace(ACCENT, &accent);
        let word = self.wordmark.replace(INK, &ink).replace(ACCENT, &accent);
        let (w, h, body) = match kind {
            Logo::Horizontal => (
                212.0,
                56.0,
                format!(
                    r#"<g transform="scale(0.875)">{mark}</g><g transform="translate(56 6.76)">{word}</g>"#
                ),
            ),
            Logo::Stacked => (
                117.0,
                70.0,
                format!(
                    r#"<g transform="translate(22.5 -16.875) scale(1.125)">{mark}</g><g transform="translate(0 44.37) scale(0.75)">{word}</g>"#
                ),
            ),
            Logo::Mark => (64.0, 64.0, mark.clone()),
        };
        let (uri, bytes) = self.bytes(format!("logo-{kind:?}-{ink}-{accent}"), || {
            format!(
                r#"<svg width="{w}" height="{h}" viewBox="0 0 {w} {h}" fill="none" xmlns="http://www.w3.org/2000/svg">{body}</svg>"#
            )
        });
        Image::from_bytes(uri, bytes).fit_to_exact_size(egui::vec2(height * w / h, height))
    }

    /// The Dash Evolution logo, white on the dark theme and black on the light one.
    pub fn dash_evolution(&mut self, p: &Palette, height: f32) -> Image<'static> {
        let (key, svg) = if p.dark {
            ("dash-evolution-white", DASH_EVOLUTION_WHITE)
        } else {
            ("dash-evolution-black", DASH_EVOLUTION_BLACK)
        };
        let (uri, bytes) = self.bytes(key.into(), || svg.to_owned());
        Image::from_bytes(uri, bytes)
            .fit_to_exact_size(egui::vec2(height * DASH_EVOLUTION_ASPECT, height))
    }

    /// The sync mark: bowl, bar and block light up as sync goes; the rest at 22 %.
    pub fn sync_mark(&mut self, stage: u8, p: &Palette, size: f32) -> Image<'static> {
        let ink = p.ink_hex();
        let accent = p.brand_hex();
        let parts: Vec<&str> = self.mark.split("/>").filter(|s| !s.is_empty()).collect();
        let body: String = parts
            .iter()
            .enumerate()
            .map(|(i, part)| {
                let opacity = if (i as u8) < stage { 1.0 } else { 0.22 };
                format!(r#"<g opacity="{opacity}">{part}/></g>"#)
            })
            .collect::<String>()
            .replace(INK, &ink)
            .replace(ACCENT, &accent);
        let (uri, bytes) = self.bytes(format!("sync-{stage}-{ink}"), || {
            format!(r#"<svg width="64" height="64" viewBox="0 0 64 64" fill="none" xmlns="http://www.w3.org/2000/svg">{body}</svg>"#)
        });
        Image::from_bytes(uri, bytes).fit_to_exact_size(Vec2::splat(size))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Logo {
    Horizontal,
    Stacked,
    Mark,
}
