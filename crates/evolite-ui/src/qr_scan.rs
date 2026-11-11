//! A QR code from a picture (FEAT-0006): the clipboard, a file chosen or dropped. Everything
//! happens off the frame, on a thread of its own, in memory: the pixels and the text read are
//! never written to disk or to the journal, which gets only that a code was read and its kind
//! (FR-6, AC-T6). What is read goes into a field as if pasted; the field's checks do the rest.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// A bigger file is refused before it is read (FR-4).
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// The longest side a picture may have, and the most its decoder may take (AC-T3).
const MAX_SIDE: u32 = 16_384;
const MAX_ALLOC: u64 = 512 * 1024 * 1024;
/// A picture longer than this is first tried at [`SMALLER`]: a screenshot's QR is large.
const LARGE: u32 = 4096;
const SMALLER: u32 = 2048;

/// What a code holds, as far as evolite cares; its name is all the journal gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Address,
    Uri,
    Name,
    AppLink,
    NotDash,
    NoQr,
}

impl Kind {
    /// The journal's word (AC-T6).
    pub fn name(self) -> &'static str {
        match self {
            Kind::Address => "address",
            Kind::Uri => "uri",
            Kind::Name => "name",
            Kind::AppLink => "app_link",
            Kind::NotDash => "not_dash",
            Kind::NoQr => "no_qr",
        }
    }
}

/// The outcome of one scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scanned {
    /// The first code on the picture that is Dash's (analyst-31).
    Found { text: String, kind: Kind },
    /// Codes, none of them Dash's.
    NotDash,
    /// No code could be read (or the picture could not be decoded within the limits).
    NoQr,
    /// Not a PNG, JPEG or BMP, or larger than 20 MB.
    NotAPicture,
    /// The file dialog was closed without a choice: nothing to say.
    Cancelled,
}

impl Scanned {
    fn kind(&self) -> Option<Kind> {
        match self {
            Scanned::Found { kind, .. } => Some(*kind),
            Scanned::NotDash => Some(Kind::NotDash),
            Scanned::NoQr | Scanned::NotAPicture => Some(Kind::NoQr),
            Scanned::Cancelled => None,
        }
    }
}

/// What a code's text is to evolite.
pub fn classify(text: &str) -> Kind {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if crate::payment_uri::is_link(text) {
        return Kind::Uri;
    }
    // An App Connect v2 sign-in, or a v1 link the core refuses in words of its own.
    if lower.starts_with("dashpay://connect")
        || lower.starts_with("dash-key:")
        || lower.starts_with("dash-st:")
    {
        return Kind::AppLink;
    }
    if let Some(name) = text.strip_prefix('@') {
        let fits = (3..=63).contains(&name.len())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        return if fits { Kind::Name } else { Kind::NotDash };
    }
    if looks_like_address(text) {
        return Kind::Address;
    }
    Kind::NotDash
}

/// Written like a Core address (Base58, 25–36 characters) or a Platform one (`(t)dash1…`):
/// the field's own check says whether it is one of this network.
fn looks_like_address(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("tdash1") || lower.starts_with("dash1") {
        return text.len() >= 40 && text.chars().all(|c| c.is_ascii_alphanumeric());
    }
    (25..=36).contains(&text.len())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && !matches!(c, '0' | 'O' | 'I' | 'l'))
}

/// The first Dash text of those read, in the order found.
pub fn pick(texts: Vec<String>) -> Scanned {
    if texts.is_empty() {
        return Scanned::NoQr;
    }
    texts
        .into_iter()
        .map(|text| (classify(&text), text))
        .find(|(kind, _)| *kind != Kind::NotDash)
        .map_or(Scanned::NotDash, |(kind, text)| Scanned::Found {
            text: text.trim().to_string(),
            kind,
        })
}

/// Every code rqrr reads on a grey picture (0 = black).
fn texts_on(grey: &image::GrayImage) -> Vec<String> {
    let (w, h) = (grey.width() as usize, grey.height() as usize);
    let mut prepared =
        rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| grey.as_raw()[y * w + x]);
    prepared
        .detect_grids()
        .into_iter()
        .filter_map(|grid| grid.decode().ok().map(|(_, text)| text))
        .collect()
}

/// A grey picture → the first Dash code: a large one at [`SMALLER`] first, then whole (AC-T3).
pub fn scan_grey(grey: &image::GrayImage) -> Scanned {
    let long = grey.width().max(grey.height());
    if long > LARGE {
        let scale = SMALLER as f64 / long as f64;
        let small = image::imageops::resize(
            grey,
            ((grey.width() as f64 * scale).round() as u32).max(1),
            ((grey.height() as f64 * scale).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        );
        let found = pick(texts_on(&small));
        if matches!(found, Scanned::Found { .. }) {
            return found;
        }
    }
    pick(texts_on(grey))
}

/// The bytes of a PNG, JPEG or BMP file → the first Dash code. The sides and the memory are
/// checked from the header, before any pixel is decoded.
pub fn scan_bytes(bytes: &[u8]) -> Scanned {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Scanned::NotAPicture;
    }
    let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()
    else {
        return Scanned::NotAPicture;
    };
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Bmp)
    ) {
        return Scanned::NotAPicture;
    }
    let mut reader = reader;
    reader.limits(limits());
    match reader.decode() {
        Ok(picture) => {
            let grey = picture.to_luma8();
            drop(picture);
            scan_grey(&grey)
        }
        Err(_) => Scanned::NoQr,
    }
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    limits
}

/// A file: refused by its size before it is read.
pub fn scan_file(path: &Path) -> Scanned {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_FILE_BYTES => Scanned::NotAPicture,
        Ok(_) => match std::fs::read(path) {
            Ok(bytes) => scan_bytes(&bytes),
            Err(_) => Scanned::NotAPicture,
        },
        Err(_) => Scanned::NotAPicture,
    }
}

/// RGBA pixels (the clipboard's): the same limits as a file.
pub fn scan_rgba(width: usize, height: usize, rgba: &[u8]) -> Scanned {
    let fits = width > 0
        && height > 0
        && width <= MAX_SIDE as usize
        && height <= MAX_SIDE as usize
        && rgba.len() == width * height * 4;
    if !fits {
        return Scanned::NoQr;
    }
    let grey = image::GrayImage::from_fn(width as u32, height as u32, |x, y| {
        let i = (y as usize * width + x as usize) * 4;
        let [r, g, b, a] = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
        // Transparent pixels as white, as the picture would be seen on a page.
        let lum = (299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000;
        let lum = (lum * a as u32 + 255 * (255 - a as u32)) / 255;
        image::Luma([lum as u8])
    });
    scan_grey(&grey)
}

/// Whether the clipboard holds a picture now. Muted in tests: the machine's clipboard is not
/// the test's.
fn clipboard_has_picture() -> bool {
    #[cfg(test)]
    return false;
    #[cfg(not(test))]
    arboard::Clipboard::new().is_ok_and(|mut board| board.get_image().is_ok())
}

fn scan_clipboard() -> Scanned {
    #[cfg(test)]
    return Scanned::NoQr;
    #[cfg(not(test))]
    {
        let Ok(mut board) = arboard::Clipboard::new() else {
            return Scanned::NoQr;
        };
        match board.get_image() {
            Ok(picture) => scan_rgba(picture.width, picture.height, &picture.bytes),
            Err(_) => Scanned::NoQr,
        }
    }
}

/// Where the picture comes from.
pub enum Picture {
    Clipboard,
    /// "Choose a picture…": the system's dialog, over the window.
    Choose(Parent),
    /// A file dropped on the window.
    File(PathBuf),
    /// Bytes already in memory (tests).
    #[cfg_attr(not(test), allow(dead_code))]
    Bytes(std::sync::Arc<[u8]>),
}

/// The window the file dialog belongs to: on Windows the foreground window at the click,
/// which is evolite's (the click was in it).
#[derive(Clone, Copy)]
pub struct Parent(#[cfg_attr(not(windows), allow(dead_code))] isize);

impl Parent {
    pub fn now() -> Self {
        #[cfg(windows)]
        {
            // SAFETY: no arguments; returns a handle or null.
            let hwnd =
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
            Parent(hwnd as isize)
        }
        #[cfg(not(windows))]
        Parent(0)
    }
}

#[cfg(windows)]
impl raw_window_handle::HasWindowHandle for Parent {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let hwnd = std::num::NonZeroIsize::new(self.0)
            .ok_or(raw_window_handle::HandleError::Unavailable)?;
        let raw = raw_window_handle::RawWindowHandle::Win32(
            raw_window_handle::Win32WindowHandle::new(hwnd),
        );
        // SAFETY: the window is evolite's own and outlives the modal dialog shown over it.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

#[cfg(windows)]
impl raw_window_handle::HasDisplayHandle for Parent {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::windows())
    }
}

fn choose(parent: Parent) -> Scanned {
    #[cfg(test)]
    {
        // In tests no dialog: the picture a test put here is «chosen».
        let _ = parent;
        let chosen = TEST_PICK.lock().unwrap_or_else(|e| e.into_inner()).take();
        chosen.map_or(Scanned::Cancelled, |bytes| scan_bytes(&bytes))
    }
    #[cfg(not(test))]
    {
        let words = crate::i18n::t(tr!("Choose a picture…")).into_owned();
        let dialog = rfd::AsyncFileDialog::new()
            .set_title(words.trim_end_matches('…'))
            .add_filter("PNG, JPG, BMP", &["png", "jpg", "jpeg", "bmp"]);
        #[cfg(windows)]
        let dialog = if parent.0 != 0 {
            dialog.set_parent(&parent)
        } else {
            dialog
        };
        #[cfg(not(windows))]
        let _ = parent;
        match block_on(dialog.pick_file()) {
            Some(file) => scan_file(file.path()),
            None => Scanned::Cancelled,
        }
    }
}

/// Waits for a future on this thread (the scan's): the dialog's, which needs no runtime.
#[cfg(not(test))]
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
        std::thread::park();
    }
}

thread_local! {
    /// Locks of the wallet so far, on the UI thread (which starts and polls every scan): a scan
    /// started before a lock is dropped when it ends.
    static LOCKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// From the app at every lock.
pub fn on_lock() {
    LOCKS.with(|l| l.set(l.get() + 1));
}

/// One place's scanning (the «To» field, the contact's name, «Apps»): its menu, the clipboard
/// check made when the menu opens and the work in progress. Holds no pixels.
#[derive(Default)]
pub struct Scanner {
    pub menu: bool,
    work: Option<(Receiver<Scanned>, u64)>,
    clip: Option<Receiver<bool>>,
    /// Whether the clipboard holds a picture, as checked at the menu's opening; `None` while
    /// it is being checked.
    pub clip_picture: Option<bool>,
}

impl Scanner {
    /// A picture is being read: the button shows it and takes no click.
    pub fn busy(&self) -> bool {
        self.work.is_some()
    }

    /// Opens the menu and looks once, off the frame, whether the clipboard has a picture (AC-T5).
    pub fn open_menu(&mut self, ctx: &egui::Context) {
        self.menu = true;
        self.clip_picture = None;
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("qr-clipboard".into())
            .spawn(move || {
                let _ = tx.send(clipboard_has_picture());
                ctx.request_repaint();
            });
        self.clip = spawned.is_ok().then_some(rx);
        if self.clip.is_none() {
            self.clip_picture = Some(false);
        }
    }

    /// Starts reading a picture; ignored while one is being read.
    pub fn start(&mut self, ctx: &egui::Context, from: Picture) {
        if self.busy() {
            return;
        }
        self.menu = false;
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("qr-scan".into())
            .spawn(move || {
                let scanned = match from {
                    Picture::Clipboard => scan_clipboard(),
                    Picture::Choose(parent) => choose(parent),
                    Picture::File(path) => scan_file(&path),
                    Picture::Bytes(bytes) => scan_bytes(&bytes),
                };
                let _ = tx.send(scanned);
                ctx.request_repaint();
            });
        if spawned.is_ok() {
            self.work = Some((rx, LOCKS.with(|l| l.get())));
        }
    }

    /// What the work brought, once: dropped if the wallet was locked meanwhile. The journal
    /// gets the kind only.
    pub fn poll(&mut self) -> Option<Scanned> {
        if let Some(rx) = &self.clip {
            match rx.try_recv() {
                Ok(has) => {
                    self.clip_picture = Some(has);
                    self.clip = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.clip_picture = Some(false);
                    self.clip = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let (rx, locks) = self.work.as_ref()?;
        let scanned = match rx.try_recv() {
            Ok(scanned) => scanned,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Scanned::NoQr,
        };
        let locked = *locks != LOCKS.with(|l| l.get());
        self.work = None;
        if locked {
            return None;
        }
        if let Some(kind) = scanned.kind() {
            evolite_core::diag::record(evolite_core::diag::Record::Answer {
                what: "qr_scanned",
                outcome: kind.name(),
            });
        }
        Some(scanned)
    }
}

/// Whether a file being dragged over the window is a picture (by its name or type): only
/// then the drop overlay shows.
pub fn dragging_picture(ctx: &egui::Context) -> bool {
    ctx.input(|i| {
        i.raw.hovered_files.first().is_some_and(|f| {
            f.path.as_deref().map_or_else(
                || f.mime.is_empty() || f.mime.starts_with("image/"),
                is_picture_name,
            )
        })
    })
}

fn is_picture_name(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["png", "jpg", "jpeg", "bmp"].contains(&e.to_ascii_lowercase().as_str()))
}

/// The picture dropped on the window this frame, if any.
pub fn dropped(ctx: &egui::Context) -> Option<Picture> {
    ctx.input(|i| {
        let file = i.raw.dropped_files.first()?;
        Some(Picture::File(file.path().to_path_buf()))
    })
}

/// The picture «Choose a picture…» gives in tests (the UI tests' file dialog).
#[cfg(test)]
pub static TEST_PICK: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

/// A PNG of `text` as a QR code on a white page: for tests.
#[cfg(test)]
#[cfg_attr(not(demo_core), allow(dead_code))] // the UI tests run on the demo core only
pub fn test_png(text: &str) -> Vec<u8> {
    let code = qrcode::QrCode::new(text).expect("a QR code");
    let n = code.width() as u32;
    let colors = code.to_colors();
    let module = 6;
    let side = (n + 8) * module;
    let picture = image::GrayImage::from_fn(side, side, |x, y| {
        let (mx, my) = (x / module, y / module);
        let dark = (4..n + 4).contains(&mx)
            && (4..n + 4).contains(&my)
            && colors[((my - 4) * n + (mx - 4)) as usize] == qrcode::Color::Dark;
        image::Luma([if dark { 0 } else { 255 }])
    });
    let mut out = Vec::new();
    picture
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("a PNG");
    out
}

#[cfg(test)]
mod tests;
