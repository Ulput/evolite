//! Design tokens from the Figma variable collection `evolite` (modes Light and Dark) and fonts.

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub line: Color32,
    /// The edge of a field: dividers keep `line`, a field must stand out at 3:1.
    pub field_line: Color32,
    pub ink: Color32,
    pub on_ink: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// "Verified": InstantLock, ChainLock, checked proofs. Never for buttons or fills.
    pub accent: Color32,
    pub danger: Color32,
    /// Under white text (the "Delete" button): `danger` itself is too light in the dark theme.
    pub danger_fill: Color32,
    /// The logo's yellow and the sync mark's: the brand, not a label, so it keeps the mockup's
    /// shade where `accent` is darkened to be read as text.
    pub brand: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const LIGHT: Palette = Palette {
    dark: false,
    bg: hex(0xFAF7F0),
    surface: hex(0xFFFFFF),
    raised: hex(0xF1ECE2),
    line: hex(0xE6E1D6),
    // 3.1:1 on raised, 3.4:1 on the page (WCAG 1.4.11).
    field_line: hex(0x8A857A),
    ink: hex(0x14161C),
    on_ink: hex(0xFAF7F0),
    text: hex(0x14161C),
    // Darker than the mockup's #6B6F7A (4.3:1 on raised): 5.2:1 on raised, 5.7:1 on the page.
    muted: hex(0x5E626C),
    // Darker than the mockup's #C27C00 (2.9:1 on the cream): 5:1 on raised, 5.5:1 on the page.
    accent: hex(0x8A5B00),
    // 5.3:1 on raised (the mockup's #C9353B: 4.4:1).
    danger: hex(0xB32F35),
    danger_fill: hex(0xB32F35),
    brand: hex(0xC27C00),
};

pub const DARK: Palette = Palette {
    dark: true,
    bg: hex(0x0B0D12),
    surface: hex(0x14171F),
    raised: hex(0x1C202A),
    line: hex(0x262A35),
    // 3.1:1 on raised, 3.7:1 on the page (WCAG 1.4.11).
    field_line: hex(0x646C7E),
    ink: hex(0xEEF2FF),
    on_ink: hex(0x0B0D12),
    text: hex(0xEEF2FF),
    muted: hex(0x8B93A7),
    accent: hex(0xFFB000),
    danger: hex(0xFF6B6B),
    // White text on #FF6B6B is 2.8:1; on this red 5.5:1.
    danger_fill: hex(0xC42F38),
    brand: hex(0xFFB000),
};

/// The wallets' colours (Figma 26, 30): yellow, blue, green, red, violet, grey — a
/// wallet's circle, by the order it was added or as chosen. The same in both themes, the
/// letter on them dark.
pub const WALLET_COLORS: [Color32; 6] = [
    hex(0xFBBF24),
    hex(0x5B8DEF),
    hex(0x4CC38A),
    hex(0xEF5F6F),
    hex(0xA078F5),
    hex(0x9BA1AB),
];

/// The letter on a wallet's circle.
pub const WALLET_LETTER: Color32 = hex(0x14161C);

impl Palette {
    pub fn ink_hex(&self) -> String {
        to_hex(self.ink)
    }
    pub fn accent_hex(&self) -> String {
        to_hex(self.accent)
    }
    pub fn brand_hex(&self) -> String {
        to_hex(self.brand)
    }
}

pub fn to_hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

/// Font weights we ship. egui has no weight axis, so every weight is its own family.
#[derive(Clone, Copy)]
pub enum Weight {
    Regular,
    Medium,
    SemiBold,
    Bold,
}

pub fn sans(size: f32, weight: Weight) -> FontId {
    let name = match weight {
        Weight::Regular => return FontId::new(size, FontFamily::Proportional),
        Weight::Medium => "inter-500",
        Weight::SemiBold => "inter-600",
        Weight::Bold => "inter-700",
    };
    FontId::new(size, FontFamily::Name(name.into()))
}

pub fn mono(size: f32, weight: Weight) -> FontId {
    match weight {
        Weight::Regular => FontId::new(size, FontFamily::Monospace),
        _ => FontId::new(size, FontFamily::Name("mono-500".into())),
    }
}

macro_rules! font {
    ($file:literal) => {
        include_bytes!(concat!("../assets/fonts/", $file))
    };
}

/// A font file of the system's and the face in it (a `.ttc` holds several).
type Face = (&'static str, u32);

/// The system's Latin font, for what Inter's subsets lack (Vietnamese and Turkish letters):
/// Windows, macOS, then the usual Linux ones.
const LATIN: &[Face] = &[
    ("segoeui.ttf", 0),
    ("Arial.ttf", 0),
    ("Helvetica.ttc", 0),
    ("DejaVuSans.ttf", 0),
    ("NotoSans-Regular.ttf", 0),
    ("LiberationSans-Regular.ttf", 0),
];

/// Chinese (simplified), Japanese and Korean, each on Windows, macOS and Linux. Noto Sans CJK's
/// collection holds the languages as faces: Japanese 0, Korean 1, simplified Chinese 2.
const ZH: &[Face] = &[
    ("msyh.ttc", 0),
    ("PingFang.ttc", 0),
    ("Hiragino Sans GB.ttc", 0),
    ("STHeiti Light.ttc", 0),
    ("NotoSansCJKsc-Regular.otf", 0),
    ("NotoSansCJK-Regular.ttc", 2),
    ("NotoSansSC-Regular.otf", 0),
    ("wqy-microhei.ttc", 0),
    ("DroidSansFallbackFull.ttf", 0),
];
const JA: &[Face] = &[
    ("YuGothM.ttc", 0),
    // Hiragino Kaku Gothic W3, named in Japanese; macOS may keep the name decomposed.
    (
        "\u{30d2}\u{30e9}\u{30ae}\u{30ce}\u{89d2}\u{30b4}\u{30b7}\u{30c3}\u{30af} W3.ttc",
        0,
    ),
    (
        "\u{30d2}\u{30e9}\u{30ad}\u{3099}\u{30ce}\u{89d2}\u{30b3}\u{3099}\u{30b7}\u{30c3}\u{30af} W3.ttc",
        0,
    ),
    ("NotoSansCJKjp-Regular.otf", 0),
    ("NotoSansCJK-Regular.ttc", 0),
    ("NotoSansJP-Regular.otf", 0),
    ("DroidSansFallbackFull.ttf", 0),
];
const KO: &[Face] = &[
    ("malgun.ttf", 0),
    ("AppleSDGothicNeo.ttc", 0),
    ("NotoSansCJKkr-Regular.otf", 0),
    ("NotoSansCJK-Regular.ttc", 1),
    ("NotoSansKR-Regular.otf", 0),
    ("NanumGothic.ttf", 0),
];

/// Where the system keeps its fonts: `%WINDIR%\Fonts`; macOS's system, library and user
/// folders; on Linux the shared and the user's folders (fontconfig's usual places).
fn font_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if cfg!(windows) {
        dirs.push(
            std::env::var_os("WINDIR")
                .map(|w| PathBuf::from(w).join("Fonts"))
                .unwrap_or_else(|| PathBuf::from(r"C:\Windows\Fonts")),
        );
    } else if cfg!(target_os = "macos") {
        dirs.extend(
            ["/System/Library/Fonts", "/Library/Fonts"]
                .iter()
                .map(PathBuf::from),
        );
        dirs.extend(home.map(|h| h.join("Library").join("Fonts")));
    } else {
        dirs.extend(
            ["/usr/share/fonts", "/usr/local/share/fonts"]
                .iter()
                .map(PathBuf::from),
        );
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".local").join("share")));
        dirs.extend(data.map(|d| d.join("fonts")));
        dirs.extend(home.map(|h| h.join(".fonts")));
    }
    dirs
}

/// Every font file under the system's font folders, by its name in lower case — a few levels
/// deep (Linux sorts fonts into folders by format and family; macOS keeps some in
/// `Supplemental`). Read once.
fn font_files() -> &'static std::collections::HashMap<String, std::path::PathBuf> {
    static FILES: std::sync::OnceLock<std::collections::HashMap<String, std::path::PathBuf>> =
        std::sync::OnceLock::new();
    FILES.get_or_init(|| {
        fn walk(
            dir: &std::path::Path,
            depth: u8,
            out: &mut std::collections::HashMap<String, std::path::PathBuf>,
        ) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if depth > 0 {
                        walk(&path, depth - 1, out);
                    }
                } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    out.entry(name.to_lowercase()).or_insert(path);
                }
            }
        }
        let mut out = std::collections::HashMap::new();
        for dir in font_dirs() {
            walk(&dir, 4, &mut out);
        }
        out
    })
}

/// The first of `faces` the system has, read from disk; `None` where it has none of them.
/// What the app does not ship: the Latin letters of Vietnamese and Turkish beyond Inter's
/// subset, and Chinese, Japanese and Korean (each font is megabytes).
fn system_font(faces: &[Face]) -> Option<FontData> {
    let files = font_files();
    faces.iter().find_map(|(name, index)| {
        let bytes = std::fs::read(files.get(&name.to_lowercase())?).ok()?;
        let mut data = FontData::from_owned(bytes);
        data.index = *index;
        Some(data)
    })
}

/// The system font for the ideographs of `lang`: the same character is drawn differently in
/// Chinese, Japanese and Korean, so the interface language picks the font. Only that one is
/// loaded.
fn cjk_font(lang: crate::i18n::Lang) -> Option<(&'static str, &'static [Face])> {
    use crate::i18n::Lang;
    match lang {
        Lang::ZhHans => Some(("System-cjk", ZH)),
        Lang::Ja => Some(("System-cjk", JA)),
        Lang::Ko => Some(("System-cjk", KO)),
        _ => None,
    }
}

/// The languages written in ideographs or Hangul, each with its own system font.
const CJK: [crate::i18n::Lang; 3] = [
    crate::i18n::Lang::ZhHans,
    crate::i18n::Lang::Ja,
    crate::i18n::Lang::Ko,
];

/// The language picker shows 简体中文, 日本語 and 한국어 whatever the interface language, each in
/// its own language's font: the interface's font may lack the script or draw the ideographs the
/// other way. These fonts are big, so they are loaded only while the picker is open: it asks
/// each frame it is drawn, the app reads the request once per frame.
static WANT_NAMES: AtomicBool = AtomicBool::new(false);
/// Which of [`CJK`] have a family for their name in the fonts in use, a bit each.
static NAMES_INSTALLED: AtomicU8 = AtomicU8::new(0);
/// The same for the fonts installed last: egui puts them to use at the next frame's start.
static NAMES_PENDING: AtomicU8 = AtomicU8::new(0);

/// Called at the start of each frame: the fonts installed during the last one are in use now.
pub fn fonts_applied() {
    NAMES_INSTALLED.store(NAMES_PENDING.load(Ordering::Relaxed), Ordering::Relaxed);
}

/// The language picker is open: the next frame needs the font of every language's name.
pub fn want_name_fonts() {
    WANT_NAMES.store(true, Ordering::Relaxed);
}

/// Whether the last frame asked for the fonts of the names; the request is cleared.
pub fn take_name_fonts() -> bool {
    WANT_NAMES.swap(false, Ordering::Relaxed)
}

fn name_bit(lang: crate::i18n::Lang) -> u8 {
    CJK.iter().position(|l| *l == lang).map_or(0, |i| 1 << i)
}

fn name_family(lang: crate::i18n::Lang) -> FontFamily {
    FontFamily::Name(format!("name-{}", lang.code()).into())
}

/// The font for a language's own name: its own font once installed, the interface's otherwise
/// (and for every language that is not [`CJK`]).
pub fn name_font(lang: crate::i18n::Lang, size: f32, weight: Weight) -> FontId {
    let bit = name_bit(lang);
    if bit != 0 && NAMES_INSTALLED.load(Ordering::Relaxed) & bit != 0 {
        FontId::new(size, name_family(lang))
    } else {
        sans(size, weight)
    }
}

/// Inter and JetBrains Mono come as Latin and Cyrillic subsets: each family is a fallback chain.
/// Noto Sans Arabic and Devanagari, the system's Latin font and — for Chinese, Japanese and
/// Korean — the language's own font, then egui's default fonts close every chain. With `names`
/// every language of [`CJK`] gets a family for its own name ([`name_font`]). Installed again
/// when the interface language changes and when the language picker opens or closes.
pub fn install_fonts(ctx: &egui::Context, lang: crate::i18n::Lang, names: bool) {
    let mut fonts = FontDefinitions::default();
    let mut extra: Vec<&'static str> = Vec::new();
    if let Some(data) = system_font(LATIN) {
        fonts
            .font_data
            .insert("System-latin".into(), Arc::new(data));
        extra.push("System-latin");
    }
    if let Some((key, file)) = cjk_font(lang)
        && let Some(data) = system_font(file)
    {
        fonts.font_data.insert(key.into(), Arc::new(data));
        extra.push(key);
    }
    // The interface language's own font is already loaded; the others only for the picker.
    let mut name_keys: Vec<(crate::i18n::Lang, String)> = Vec::new();
    for other in CJK.into_iter().filter(|_| names) {
        let Some((own_key, file)) = cjk_font(other) else {
            continue;
        };
        if other == lang {
            if fonts.font_data.contains_key(own_key) {
                name_keys.push((other, own_key.to_string()));
            }
        } else if let Some(data) = system_font(file) {
            let key = format!("System-cjk-{}", other.code());
            fonts.font_data.insert(key.clone(), Arc::new(data));
            name_keys.push((other, key));
        }
    }
    let mut add = |key: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(key.into(), Arc::new(FontData::from_static(bytes)));
    };
    add("Inter-latin-400", font!("Inter-latin-400.ttf"));
    add("Inter-cyrillic-400", font!("Inter-cyrillic-400.ttf"));
    add("Inter-latin-500", font!("Inter-latin-500.ttf"));
    add("Inter-cyrillic-500", font!("Inter-cyrillic-500.ttf"));
    add("Inter-latin-600", font!("Inter-latin-600.ttf"));
    add("Inter-cyrillic-600", font!("Inter-cyrillic-600.ttf"));
    add("Inter-latin-700", font!("Inter-latin-700.ttf"));
    add("Inter-cyrillic-700", font!("Inter-cyrillic-700.ttf"));
    add("Mono-latin-400", font!("JetBrainsMono-latin-400.ttf"));
    add("Mono-cyrillic-400", font!("JetBrainsMono-cyrillic-400.ttf"));
    add("Mono-latin-500", font!("JetBrainsMono-latin-500.ttf"));
    add("Mono-cyrillic-500", font!("JetBrainsMono-cyrillic-500.ttf"));

    // Arabic and Devanagari ship with the app, so they render the same on every OS.
    add("Noto-arabic", font!("NotoSansArabic-arabic-400.ttf"));
    add(
        "Noto-devanagari",
        font!("NotoSansDevanagari-devanagari-400.ttf"),
    );

    let default_chain = fonts.families[&FontFamily::Proportional].clone();
    let chain = |names: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        v.extend(["Noto-arabic".to_string(), "Noto-devanagari".to_string()]);
        v.extend(extra.iter().map(|s| s.to_string()));
        // egui's own fonts as the last resort: maths and arrows (≈, →) that our subsets lack.
        v.extend(default_chain.iter().cloned());
        v
    };
    let families = &mut fonts.families;
    families.insert(
        FontFamily::Proportional,
        chain(&["Inter-latin-400", "Inter-cyrillic-400"]),
    );
    families.insert(
        FontFamily::Monospace,
        chain(&["Mono-latin-400", "Mono-cyrillic-400"]),
    );
    for (name, list) in [
        ("inter-500", ["Inter-latin-500", "Inter-cyrillic-500"]),
        ("inter-600", ["Inter-latin-600", "Inter-cyrillic-600"]),
        ("inter-700", ["Inter-latin-700", "Inter-cyrillic-700"]),
        ("mono-500", ["Mono-latin-500", "Mono-cyrillic-500"]),
    ] {
        families.insert(FontFamily::Name(name.into()), chain(&list));
    }
    let mut installed = 0;
    for (other, key) in &name_keys {
        families.insert(name_family(*other), chain(&[key.as_str()]));
        installed |= name_bit(*other);
    }
    ctx.set_fonts(fonts);
    NAMES_PENDING.store(installed, Ordering::Relaxed);
}

/// egui's own widgets (scroll bars, text edits, tooltips) in our colours.
pub fn apply_visuals(ctx: &egui::Context, p: &Palette) {
    let mut v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.raised;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.ink.gamma_multiply(0.25);
    v.selection.stroke.color = p.ink;
    v.widgets.noninteractive.bg_stroke.color = p.line;
    v.widgets.inactive.bg_stroke.color = p.line;
    v.widgets.hovered.bg_stroke.color = p.muted;
    v.widgets.active.bg_stroke.color = p.ink;
    // The keyboard focus ring (`focus`) takes the accent from here.
    v.hyperlink_color = p.accent;
    // Field hints: egui paints them in this colour whatever the hint's own (BUG-50); without it,
    // the text colour at 60 %, darker than the field labels.
    v.weak_text_color = Some(p.muted);
    // The scroll bar's handle (BUG-49): a quiet line, lighter under the pointer and while dragged.
    // `bg_fill` is also the fill of every built-in egui widget (checkbox, radio, slider, combo
    // box; a framed `Button` takes `weak_bg_fill`). evolite draws its own and uses only
    // `Button::frame(false)`, which paints no fill; a built-in widget added later gets these.
    v.widgets.inactive.bg_fill = p.field_line;
    v.widgets.hovered.bg_fill = p.muted;
    v.widgets.active.bg_fill = p.text;
    // Both of egui's styles, and ours chosen: egui picks a style by the system's theme, and ours
    // may be the other one (dark evolite on a light Windows got egui's light defaults: a white
    // scroll bar, dark text in History's search; BUG-31, BUG-49).
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.set_theme(if p.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.all_styles_mut(|s| s.spacing.scroll = scroll_style());
}

/// A thin bar always shows: content below the window's edge is not a secret. No track until the
/// pointer is on the bar, and 8 px between the content and the resting bar (BUG-49).
pub fn scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        floating_allocated_width: 10.0,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        ..egui::style::ScrollStyle::thin()
    }
}

/// Least widths of the cells of `components::grid` (responsive spec 2026-11-06, «Минимальные
/// ширины»): a balance tile (Overview, the pool), an NFT card (195, not 200: three in
/// Collection at 1280 — the designer, 2026-11-07), an app card.
pub const TILE_MIN_W: f32 = 200.0;
pub const NFT_MIN_W: f32 = 195.0;
pub const APP_CARD_MIN_W: f32 = 240.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-31, 49, 50: dark evolite on a light system. egui's active style is ours whatever
    /// the system says: text in our colour, hints `muted`, the scroll bar `field_line`.
    #[test]
    fn our_theme_wins_over_the_systems() {
        for (p, system) in [(DARK, egui::Theme::Light), (LIGHT, egui::Theme::Dark)] {
            let ctx = egui::Context::default();
            // As at the app's start: egui follows the system (unknown before the first frame).
            apply_visuals(&ctx, &p);
            // A frame, as the app has: the system's theme arrives with the input.
            let input = egui::RawInput {
                system_theme: Some(system),
                ..Default::default()
            };
            ctx.run_ui(input, |_| {}).textures_delta.clear();
            let v = ctx.global_style().visuals.clone();
            assert_eq!(v.dark_mode, p.dark);
            assert_eq!(v.text_color(), p.text, "typed text (History's search)");
            assert_eq!(v.weak_text_color(), p.muted, "field hints");
            assert_eq!(
                v.widgets.inactive.bg_fill, p.field_line,
                "scroll bar at rest"
            );
            assert_eq!(
                v.widgets.hovered.bg_fill, p.muted,
                "scroll bar under the pointer"
            );
            assert_eq!(v.extreme_bg_color, p.raised);
            let scroll = ctx.global_style().spacing.scroll;
            assert_eq!(scroll.dormant_background_opacity, 0.0, "no white track");
            assert!(
                scroll.floating_allocated_width - scroll.floating_width >= 8.0,
                "8 px between the content and the bar"
            );
        }
    }
}
