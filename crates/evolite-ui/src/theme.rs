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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// [`mono`] for a line of figures with words around them («No. 12 of 500», «3 notes · last
/// checked …»), but [`sans`] in the Arabic interface: the mono sets Arabic letters apart and
/// stretched its words (BUG-118).
pub fn figures(size: f32, weight: Weight) -> FontId {
    if crate::i18n::rtl() {
        sans(size, weight)
    } else {
        mono(size, weight)
    }
}

/// The named type scale (FEAT-0033): every text of the app takes one of these steps, not a size
/// of its own. A step is a size, the line height of a paragraph in it (the designer's table: about 1.5 times the size), the
/// face and the weight it is set in. Screens move to it one by one (FEAT-0033 step 2); until
/// then the guard test lists the sizes still written as numbers.
// Unused until the first screen moves to it (step 2).
#[allow(dead_code)]
pub mod type_scale {
    use super::Weight;

    /// One step of the scale.
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub struct Step {
        pub size: f32,
        /// A paragraph's line height in this step.
        pub line: f32,
        /// JetBrains Mono, for amounts, addresses, ids, txids and keys only; Inter otherwise.
        pub mono: bool,
        pub weight: Weight,
    }

    impl Step {
        /// The same step in another weight: Body, UI and Small are set in several.
        pub const fn weight(self, weight: Weight) -> Step {
            Step { weight, ..self }
        }
    }

    const fn step(size: f32, line: f32, mono: bool, weight: Weight) -> Step {
        Step {
            size,
            line,
            mono,
            weight,
        }
    }

    /// Overview's total.
    pub const DISPLAY: Step = step(44.0, 55.0, true, Weight::Medium);
    /// A large amount: an operation's card, the pool, an app's request.
    pub const AMOUNT_L: Step = step(28.0, 35.0, true, Weight::Medium);
    /// An amount in a tile (20 in a narrow window, R4).
    pub const AMOUNT_M: Step = step(24.0, 30.0, true, Weight::Medium);
    /// A screen's title.
    pub const TITLE: Step = step(28.0, 35.0, false, Weight::Bold);
    /// A dialog's title.
    pub const HEADING: Step = step(20.0, 25.0, false, Weight::Bold);
    /// A card's or a section's title.
    pub const SUBHEADING: Step = step(16.0, 20.0, false, Weight::SemiBold);
    /// Fields, buttons, lines.
    pub const BODY: Step = step(14.0, 21.0, false, Weight::Regular);
    /// Labels and the values in rows.
    pub const UI: Step = step(13.0, 20.0, false, Weight::Regular);
    /// The lines under rows, explanations.
    pub const SMALL: Step = step(12.0, 18.0, false, Weight::Regular);
    /// Over groups; addresses under a source.
    pub const CAPTION: Step = step(11.0, 16.0, false, Weight::Regular);

    /// Every step, largest first.
    pub const ALL: [Step; 10] = [
        DISPLAY, AMOUNT_L, AMOUNT_M, TITLE, HEADING, SUBHEADING, BODY, UI, SMALL, CAPTION,
    ];
}

/// The font of a step of the type scale (FEAT-0033): the one way a screen asks for a text's
/// size once it has moved to the scale.
#[allow(dead_code)] // Until the first screen moves to the scale (step 2).
pub fn style(step: type_scale::Step) -> FontId {
    if step.mono {
        mono(step.size, step.weight)
    } else {
        sans(step.size, step.weight)
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
    system_font_file(faces)
}

/// A system CJK font, its glyphs moved onto Inter's baseline ([`on_latin_baseline`]).
fn cjk_system_font(faces: &[Face]) -> Option<FontData> {
    system_font_file(faces).map(on_latin_baseline)
}

/// egui sets a fallback font's glyphs by that font's own ascent, centring the difference in line
/// height, so a system CJK font stands off Inter's baseline: Japanese sat higher than the Latin
/// letters beside it (FEAT-0033, AC-D5). The two baselines are measured once, in a layout of «H»
/// and «中» at a large size, and the CJK font is shifted by the difference. Only where its
/// glyphs are drawn: the line's height does not change.
fn on_latin_baseline(mut cjk: FontData) -> FontData {
    if let Some(shift) = baseline_gap(&cjk) {
        cjk.tweak.y_offset_factor = bounded_shift(shift);
    }
    cjk
}

/// The measured shift, trusted only within [`MAX_SHIFT`] of the size: a strange or damaged font
/// file could measure far off and push its text out of the line, so beyond that it is not
/// moved at all, as before the fix (FEAT-0033 r1 Should 1).
fn bounded_shift(gap: f32) -> f32 {
    if gap.abs() <= MAX_SHIFT { gap } else { 0.0 }
}

/// The largest shift trusted, as a fraction of the size.
const MAX_SHIFT: f32 = 0.3;

/// How far down (as a fraction of the size) `cjk`'s «中» must move to sit on Inter's baseline
/// beside an «H»; `None` when the font lacks it.
fn baseline_gap(cjk: &FontData) -> Option<f32> {
    const SIZE: f32 = 100.0;
    let mut defs = FontDefinitions::empty();
    defs.font_data.insert(
        "latin".into(),
        Arc::new(FontData::from_static(font!("Inter-latin-400.ttf"))),
    );
    defs.font_data.insert("cjk".into(), Arc::new(cjk.clone()));
    defs.families
        .insert(FontFamily::Proportional, vec!["latin".into(), "cjk".into()]);
    let mut fonts =
        egui::epaint::text::Fonts::new(egui::epaint::text::TextOptions::default(), defs);
    let mut view = fonts.with_pixels_per_point(1.0);
    let galley = view.layout_no_wrap(
        "H中".into(),
        FontId::new(SIZE, FontFamily::Proportional),
        Color32::WHITE,
    );
    let glyphs = &galley.rows.first()?.glyphs;
    let (latin, cjk) = (glyphs.first()?, glyphs.get(1)?);
    (cjk.chr == '中').then(|| (latin.pos.y - cjk.pos.y) / SIZE)
}

/// The first of `faces` found on disk, as it is.
fn system_font_file(faces: &[Face]) -> Option<FontData> {
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
        && let Some(data) = cjk_system_font(file)
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
        } else if let Some(data) = cjk_system_font(file) {
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

    // Arabic and Devanagari ship with the app, so they render the same on every OS. Arabic in
    // Medium, SemiBold and Bold too, for the labels, headings and buttons that have that weight
    // in every other language (BUG-124): Noto Sans Arabic 2.012, the Google Fonts statics, as
    // the 400 is.
    add("Noto-arabic", font!("NotoSansArabic-arabic-400.ttf"));
    add("Noto-arabic-500", font!("NotoSansArabic-500.ttf"));
    add("Noto-arabic-600", font!("NotoSansArabic-600.ttf"));
    add("Noto-arabic-700", font!("NotoSansArabic-700.ttf"));
    add(
        "Noto-devanagari",
        font!("NotoSansDevanagari-devanagari-400.ttf"),
    );

    let default_chain = fonts.families[&FontFamily::Proportional].clone();
    // The Arabic face of a family's weight: Medium, SemiBold and Bold have their own, Regular
    // the 400.
    let arabic_of = |names: &[&str]| {
        if names.iter().any(|n| n.ends_with("-700")) {
            "Noto-arabic-700"
        } else if names.iter().any(|n| n.ends_with("-600")) {
            "Noto-arabic-600"
        } else if names.iter().any(|n| n.ends_with("-500")) {
            "Noto-arabic-500"
        } else {
            "Noto-arabic"
        }
    };
    let chain = |names: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        v.extend([arabic_of(names).to_string(), "Noto-devanagari".to_string()]);
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
/// Collection at 1280 — the designer, 2026-11-07).
pub const TILE_MIN_W: f32 = 200.0;
pub const NFT_MIN_W: f32 = 195.0;

/// The smallest window (responsive spec §1; the customer, 2026-11-07, дизайнер-26): 1280 × 720
/// at 150 % is 853 wide; 440 is 1366 × 768 at 150 % less the taskbar and the title bar.
pub const MIN_WINDOW: [f32; 2] = [800.0, 440.0];
/// From these window widths on: S (standard), W (wide, the menu with its words), X (very wide).
pub const BP_STANDARD: f32 = 1024.0;
pub const BP_WIDE: f32 = 1280.0;
pub const BP_EXTRA_WIDE: f32 = 1600.0;
/// The menu with its words, and the rail of icons it folds to below [`BP_WIDE`].
pub const MENU_W: f32 = 248.0;
pub const RAIL_W: f32 = 76.0;
/// The widest the content of a screen gets, centred in a wider window (дизайнер-28).
pub const CONTENT_MAX_W: f32 = 1200.0;

/// The window's width class (responsive spec §1). Taken from the width of the **window**, not of
/// the content: the rail changes the content's width, and the content's width must not switch
/// the rail back (the architect, 2026-11-06). Once a frame in `App`, kept in `Ctx::bp`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Breakpoint {
    /// C, 800–1023: the rail, margins 24 (20 on top).
    Compact,
    /// S, 1024–1279: the rail, margins 40 (28 on top).
    #[default]
    Standard,
    /// W, 1280–1599: the menu with its words.
    Wide,
    /// X, 1600 and wider: the menu; the content as in W, no wider than [`CONTENT_MAX_W`].
    ExtraWide,
}

impl Breakpoint {
    /// The class of a window `width` points wide.
    pub fn of(width: f32) -> Self {
        if width >= BP_EXTRA_WIDE {
            Self::ExtraWide
        } else if width >= BP_WIDE {
            Self::Wide
        } else if width >= BP_STANDARD {
            Self::Standard
        } else {
            Self::Compact
        }
    }

    /// The menu is the rail of icons below W (дизайнер-27).
    pub fn rail(self) -> bool {
        self < Self::Wide
    }

    /// The menu's width.
    pub fn menu_width(self) -> f32 {
        if self.rail() { RAIL_W } else { MENU_W }
    }

    /// The screen's margins: (left and right, top and bottom) — 24 and 20 in C, 40 and 28 wider.
    pub fn margins(self) -> (i8, i8) {
        if self == Self::Compact {
            (24, 20)
        } else {
            (40, 28)
        }
    }
}

/// The content's place in the screen's `area` (inside its margins): all of it, or
/// [`CONTENT_MAX_W`] in the middle of a wider one (дизайнер-28).
pub fn content_rect(area: egui::Rect) -> egui::Rect {
    if area.width() <= CONTENT_MAX_W {
        return area;
    }
    let left = area.center().x - CONTENT_MAX_W / 2.0;
    egui::Rect::from_x_y_ranges(left..=left + CONTENT_MAX_W, area.y_range())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-124: Arabic headings (Bold), buttons (SemiBold) and field labels (Medium) are drawn in
    /// Noto Sans Arabic 700, 600 and 500, as Latin ones in Inter 700, 600 and 500: the 400 was the
    /// only Arabic face, so they came out as thin as the body text. Body text keeps the 400.
    #[test]
    fn arabic_headings_and_buttons_take_the_bold_faces() {
        let ctx = egui::Context::default();
        install_fonts(&ctx, crate::i18n::Lang::Ar, false);
        ctx.run_ui(egui::RawInput::default(), |_| {})
            .textures_delta
            .clear();
        let heading = sans(20.0, Weight::Bold);
        let button = sans(14.0, Weight::SemiBold);
        let arabic_of = |id: &FontId| {
            ctx.fonts(|f| {
                f.definitions().families[&id.family]
                    .iter()
                    .find(|k| k.starts_with("Noto-arabic"))
                    .cloned()
            })
        };
        assert_eq!(arabic_of(&heading).as_deref(), Some("Noto-arabic-700"));
        assert_eq!(arabic_of(&button).as_deref(), Some("Noto-arabic-600"));
        let label = sans(12.0, Weight::Medium);
        assert_eq!(arabic_of(&label).as_deref(), Some("Noto-arabic-500"));
        assert_eq!(
            arabic_of(&sans(14.0, Weight::Regular)).as_deref(),
            Some("Noto-arabic")
        );
        // Each face is the weight it says.
        let weight_class = |bytes: &[u8]| {
            let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
            (0..tables)
                .map(|i| &bytes[12 + 16 * i..28 + 16 * i])
                .find(|t| &t[..4] == b"OS/2")
                .map(|t| {
                    let at = u32::from_be_bytes([t[8], t[9], t[10], t[11]]) as usize;
                    u16::from_be_bytes([bytes[at + 4], bytes[at + 5]])
                })
        };
        assert_eq!(weight_class(font!("NotoSansArabic-500.ttf")), Some(500));
        assert_eq!(weight_class(font!("NotoSansArabic-600.ttf")), Some(600));
        assert_eq!(weight_class(font!("NotoSansArabic-700.ttf")), Some(700));
        // As drawn: the welcome back of the unlock screen, wider in each heavier face.
        let width = |id: FontId| {
            ctx.fonts_mut(|f| {
                f.layout_no_wrap("مرحبًا بعودتك".into(), id, egui::Color32::WHITE)
                    .size()
                    .x
            })
        };
        let regular = width(sans(20.0, Weight::Regular));
        let medium = width(sans(20.0, Weight::Medium));
        let semibold = width(sans(20.0, Weight::SemiBold));
        let bold = width(heading);
        assert!(
            regular < medium && medium < semibold && semibold < bold,
            "{regular} {medium} {semibold} {bold}"
        );
    }

    /// R2: the window's width class at the edges of the spec's table, the rail below 1280,
    /// margins 24 in C only, the content no wider than 1200 and in the middle.
    #[test]
    fn the_window_width_gives_its_class_menu_and_margins() {
        use Breakpoint::*;
        for (width, bp) in [
            (800.0, Compact),
            (1023.9, Compact),
            (1024.0, Standard),
            (1279.0, Standard),
            (1280.0, Wide),
            (1599.0, Wide),
            (1600.0, ExtraWide),
            (2560.0, ExtraWide),
        ] {
            assert_eq!(Breakpoint::of(width), bp, "{width}");
        }
        assert!(Compact.rail() && Standard.rail() && !Wide.rail() && !ExtraWide.rail());
        assert_eq!(Standard.menu_width(), 76.0);
        assert_eq!(Wide.menu_width(), 248.0);
        assert_eq!(Compact.margins(), (24, 20));
        assert_eq!(Standard.margins(), (40, 28));
        assert_eq!(ExtraWide.margins(), (40, 28));
        let narrow = egui::Rect::from_min_size(egui::pos2(76.0, 0.0), egui::vec2(900.0, 500.0));
        assert_eq!(content_rect(narrow), narrow);
        let wide = egui::Rect::from_min_size(egui::pos2(288.0, 28.0), egui::vec2(1592.0, 900.0));
        let content = content_rect(wide);
        assert_eq!(content.width(), CONTENT_MAX_W);
        assert_eq!(content.center().x, wide.center().x);
        assert_eq!(content.y_range(), wide.y_range());
    }

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

#[cfg(test)]
mod type_scale_tests {
    use super::*;

    /// FEAT-0033: the scale is the designer's: ten steps, the paragraphs' lines from the table,
    /// mono only for amounts.
    #[test]
    fn the_scale_is_the_designers() {
        use type_scale::*;
        let sizes: Vec<f32> = ALL.iter().map(|s| s.size).collect();
        assert_eq!(
            sizes,
            [44.0, 28.0, 24.0, 28.0, 20.0, 16.0, 14.0, 13.0, 12.0, 11.0]
        );
        assert_eq!(
            (BODY.line, UI.line, SMALL.line, CAPTION.line),
            (21.0, 20.0, 18.0, 16.0)
        );
        // Headings and amounts: 1.25 times the size, whole pixels (the designer, 2026-11-11).
        let lines: Vec<f32> = ALL[..6].iter().map(|s| s.line).collect();
        assert_eq!(lines, [55.0, 35.0, 30.0, 35.0, 25.0, 20.0]);
        let monos: Vec<bool> = ALL.iter().map(|s| s.mono).collect();
        assert_eq!(
            monos,
            [
                true, true, true, false, false, false, false, false, false, false
            ]
        );
        assert_eq!(style(HEADING), sans(20.0, Weight::Bold));
        assert_eq!(style(AMOUNT_L), mono(28.0, Weight::Medium));
        assert_eq!(
            style(BODY.weight(Weight::SemiBold)),
            sans(14.0, Weight::SemiBold)
        );
    }

    /// r1 Should 1: a shift beyond 0.3 of the size, or no number at all, moves nothing.
    #[test]
    fn a_shift_out_of_bounds_moves_nothing() {
        assert_eq!(bounded_shift(0.04), 0.04);
        assert_eq!(bounded_shift(-0.3), -0.3);
        assert_eq!(bounded_shift(0.31), 0.0);
        assert_eq!(bounded_shift(-2.0), 0.0);
        assert_eq!(bounded_shift(f32::NAN), 0.0);
    }

    /// FEAT-0033 AC-D5: shifted, a system Japanese font's «中» sits on Inter's baseline at the
    /// sizes the app uses. Skipped where the system has no Japanese font (a CI machine).
    #[test]
    fn a_system_japanese_font_sits_on_the_latin_baseline() {
        let Some(raw) = system_font_file(JA) else {
            eprintln!("no Japanese system font here: nothing to check");
            return;
        };
        let shifted = on_latin_baseline(raw.clone());
        for size in [13.0, 20.0] {
            let gap = |cjk: &FontData| {
                let mut defs = FontDefinitions::empty();
                defs.font_data.insert(
                    "latin".into(),
                    Arc::new(FontData::from_static(font!("Inter-latin-400.ttf"))),
                );
                defs.font_data.insert("cjk".into(), Arc::new(cjk.clone()));
                defs.families
                    .insert(FontFamily::Proportional, vec!["latin".into(), "cjk".into()]);
                let mut fonts = egui::epaint::text::Fonts::new(
                    egui::epaint::text::TextOptions::default(),
                    defs,
                );
                let mut view = fonts.with_pixels_per_point(1.0);
                let galley = view.layout_no_wrap(
                    "H中".into(),
                    FontId::new(size, FontFamily::Proportional),
                    Color32::WHITE,
                );
                // Where each glyph's ink starts: its baseline plus its font's own shift.
                let row = &galley.rows[0];
                let y = |i: usize, tweak: f32| row.glyphs[i].pos.y + tweak * size;
                y(0, 0.0) - y(1, cjk.tweak.y_offset_factor)
            };
            let before = gap(&raw);
            let after = gap(&shifted);
            eprintln!("size {size}: {before:.2} px off before, {after:.2} after");
            assert!(
                after.abs() <= 1.0,
                "size {size}: {after} px off the baseline"
            );
        }
    }
}

/// FEAT-0033 step 1: the files of `evolite-ui/src` that still give a text's size as a number
/// (`sans(13.0, …)`, `mono(…)`, `txt(…, 12.0, …)`), with how many. Printed only: when every
/// screen has moved to [`type_scale`] (step 3), this becomes an assert that the list is empty.
#[cfg(test)]
mod size_literals {
    use std::path::{Path, PathBuf};

    /// How many calls of `sans` / `mono` with a number first and of `txt` with a number second.
    pub(super) fn count(text: &str) -> usize {
        let mut n = 0;
        for (call, arg) in [("sans(", 0), ("mono(", 0), ("txt(", 1)] {
            let mut from = 0;
            while let Some(at) = text[from..].find(call) {
                let start = from + at;
                from = start + call.len();
                // A call, not the end of another name (`is_txt(`) or a definition.
                let before = text[..start].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                    || text[..start].ends_with("fn ")
                {
                    continue;
                }
                if nth_arg(&text[from..], arg)
                    .is_some_and(|a| a.starts_with(|c: char| c.is_ascii_digit()))
                {
                    n += 1;
                }
            }
        }
        n
    }

    /// The `n`th argument of a call whose text starts right after its `(`, trimmed.
    fn nth_arg(rest: &str, n: usize) -> Option<&str> {
        let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
        let (mut index, mut arg_start) = (0usize, 0usize);
        for (i, c) in rest.char_indices() {
            if quoted {
                match (escaped, c) {
                    (true, _) => escaped = false,
                    (false, '\\') => escaped = true,
                    (false, '"') => quoted = false,
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => quoted = true,
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth == 0 => {
                    return (index == n).then(|| rest[arg_start..i].trim());
                }
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    if index == n {
                        return Some(rest[arg_start..i].trim());
                    }
                    index += 1;
                    arg_start = i + 1;
                }
                _ => {}
            }
        }
        None
    }

    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn sizes_written_as_numbers_are_listed() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut all = Vec::new();
        files(&src, &mut all);
        let mut found: Vec<(String, usize)> = all
            .iter()
            .filter(|p| !p.ends_with("theme.rs"))
            .filter_map(|p| {
                let n = count(&std::fs::read_to_string(p).ok()?);
                let name = p.strip_prefix(&src).ok()?.display().to_string();
                (n > 0).then_some((name, n))
            })
            .collect();
        found.sort();
        let total: usize = found.iter().map(|(_, n)| n).sum();
        eprintln!(
            "FEAT-0033: {total} sizes as numbers in {} files (step 3 makes this fail):",
            found.len()
        );
        for (name, n) in &found {
            eprintln!("  {n:4}  {name}");
        }
        assert!(
            !found.is_empty(),
            "the scan found nothing: is it reading `src`?"
        );
    }

    /// The scan counts what it should and nothing else.
    #[test]
    fn the_scan_counts_calls_with_a_number() {
        let text = r#"
            sans(13.0, Weight::Medium); mono(28.0, w);
            txt("Hi, there", 12.0, Weight::Regular, p.muted);
            txt(&format!("{}, {}", a, b), 14.0, w, c);
            sans(size, w); txt(s, step.size, w, c);
            fn sans(size: f32) {} is_txt(1, 2);
        "#;
        assert_eq!(count(text), 4);
    }
}
