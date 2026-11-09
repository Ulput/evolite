use super::*;
use image::{GrayImage, Luma};

const ADDRESS: &str = "yXk3Tq9mD8vJw2RfL6pN4sB7cH1eZ5aG9f";

/// A code drawn as a screen draws it: `module` pixels a module, a quiet zone of 4, dark on
/// white, on a background of `back`, at (`x`, `y`) of a `w`×`h` picture.
fn draw(picture: &mut GrayImage, text: &str, module: u32, at: (u32, u32)) {
    let code = qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::M).unwrap();
    let n = code.width() as u32;
    let colors = code.to_colors();
    let side = (n + 8) * module;
    for y in 0..side {
        for x in 0..side {
            let (mx, my) = (x / module, y / module);
            let dark = (4..n + 4).contains(&mx)
                && (4..n + 4).contains(&my)
                && colors[((my - 4) * n + (mx - 4)) as usize] == qrcode::Color::Dark;
            picture.put_pixel(at.0 + x, at.1 + y, Luma([if dark { 13 } else { 255 }]));
        }
    }
}

fn picture(w: u32, h: u32, back: u8) -> GrayImage {
    GrayImage::from_pixel(w, h, Luma([back]))
}

fn png(picture: &GrayImage) -> Vec<u8> {
    let mut out = Vec::new();
    picture
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn found(text: &str, kind: Kind) -> Scanned {
    Scanned::Found {
        text: text.into(),
        kind,
    }
}

#[test]
fn texts_are_sorted_into_what_evolite_takes() {
    assert_eq!(classify(ADDRESS), Kind::Address);
    assert_eq!(
        classify("tdash1kz0xs3dle5xwmq4mk8rh9pkcr8txmqmeyk3xg6"),
        Kind::Address
    );
    assert_eq!(classify(&format!("DASH:{ADDRESS}?amount=1")), Kind::Uri);
    assert_eq!(classify("@satoshi"), Kind::Name);
    assert_eq!(classify("dash-key:yappr?key=MEDIUM"), Kind::AppLink);
    assert_eq!(classify("dash-st:jembe"), Kind::AppLink);
    for other in [
        "https://example.com",
        "@a",
        "hello",
        "",
        "bitcoin:1BoatSLRHtKNngkdXEeobR76b53LETtpyT",
    ] {
        assert_eq!(classify(other), Kind::NotDash, "{other}");
    }
}

#[test]
fn the_first_dash_code_is_taken() {
    assert_eq!(pick(vec![]), Scanned::NoQr);
    assert_eq!(pick(vec!["https://example.com".into()]), Scanned::NotDash);
    assert_eq!(
        pick(vec![
            "https://example.com".into(),
            "@bob".into(),
            ADDRESS.into()
        ]),
        found("@bob", Kind::Name)
    );
}

#[test]
fn a_screenshot_of_receive_is_read_in_both_themes() {
    let uri = format!("dash:{ADDRESS}?amount=0.25");
    // The Receive card: a white card on the theme's background, 1280×800 as a window.
    for back in [18u8, 246] {
        let mut shot = picture(1280, 800, back);
        draw(&mut shot, &uri, 6, (500, 200));
        assert_eq!(
            scan_bytes(&png(&shot)),
            found(&uri, Kind::Uri),
            "background {back}"
        );
    }
}

#[test]
fn a_dashpay_code_with_a_user_is_read() {
    let uri = format!("dash:{ADDRESS}?user=satoshi");
    let mut shot = picture(720, 1280, 255);
    draw(&mut shot, &uri, 9, (100, 300));
    assert_eq!(scan_bytes(&png(&shot)), found(&uri, Kind::Uri));
}

#[test]
fn a_noisy_code_at_half_the_size_is_read() {
    let mut shot = picture(1600, 1000, 30);
    draw(&mut shot, ADDRESS, 8, (300, 200));
    let mut half = image::imageops::resize(&shot, 800, 500, image::imageops::FilterType::Triangle);
    // Noise of ±24 by a fixed generator: the same picture every run.
    let mut seed = 0x2545_f491_u32;
    for p in half.pixels_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let noise = (seed % 49) as i32 - 24;
        p.0[0] = (p.0[0] as i32 + noise).clamp(0, 255) as u8;
    }
    assert_eq!(scan_bytes(&png(&half)), found(ADDRESS, Kind::Address));
}

#[test]
fn a_picture_without_a_code_has_none() {
    let mut shot = picture(640, 480, 200);
    for x in 100..300 {
        for y in 100..140 {
            shot.put_pixel(x, y, Luma([10]));
        }
    }
    assert_eq!(scan_bytes(&png(&shot)), Scanned::NoQr);
}

#[test]
fn of_two_codes_the_dash_one_is_taken() {
    let mut shot = picture(1400, 700, 255);
    draw(&mut shot, "https://example.com", 6, (50, 100));
    draw(&mut shot, ADDRESS, 6, (800, 100));
    assert_eq!(scan_bytes(&png(&shot)), found(ADDRESS, Kind::Address));
}

#[test]
fn a_code_that_is_not_dash_says_so() {
    let mut shot = picture(600, 600, 255);
    draw(&mut shot, "https://example.com", 8, (100, 100));
    assert_eq!(scan_bytes(&png(&shot)), Scanned::NotDash);
}

#[test]
fn a_4k_screenshot_is_read_first_at_a_smaller_size() {
    let uri = format!("dash:{ADDRESS}?amount=0.25");
    let mut shot = picture(3840, 2160, 18);
    draw(&mut shot, &uri, 12, (1500, 500));
    let started = std::time::Instant::now();
    assert_eq!(scan_grey(&shot), found(&uri, Kind::Uri));
    // The measure for the handoff (debug here; AC-T4 asks ≤ 1 s in release).
    eprintln!("4K screenshot read in {:?}", started.elapsed());
}

#[test]
fn clipboard_pixels_are_read_with_transparency_as_white() {
    let mut shot = picture(500, 500, 255);
    draw(&mut shot, ADDRESS, 6, (60, 60));
    let rgba: Vec<u8> = shot
        .pixels()
        .flat_map(|p| {
            // White as transparent: a cut-out code pasted from a page.
            if p.0[0] == 255 {
                [0, 0, 0, 0]
            } else {
                [p.0[0], p.0[0], p.0[0], 255]
            }
        })
        .collect();
    assert_eq!(scan_rgba(500, 500, &rgba), found(ADDRESS, Kind::Address));
    assert_eq!(scan_rgba(500, 499, &rgba), Scanned::NoQr);
    assert_eq!(scan_rgba(20_000, 1, &vec![0; 80_000]), Scanned::NoQr);
}

/// A PNG of `w`×`h` whose header alone is right: what a size bomb looks like before decoding.
fn bomb(w: u32, h: u32) -> Vec<u8> {
    fn crc(bytes: &[u8]) -> u32 {
        let mut c = !0u32;
        for b in bytes {
            c ^= *b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc(&body).to_be_bytes());
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 0, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(
        &mut out,
        b"IDAT",
        &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    );
    chunk(&mut out, b"IEND", &[]);
    out.resize(out.len().max(1024), 0);
    out
}

#[test]
fn a_size_bomb_is_refused_from_its_header() {
    let started = std::time::Instant::now();
    assert_eq!(scan_bytes(&bomb(20_000, 20_000)), Scanned::NoQr);
    assert_eq!(scan_bytes(&bomb(16_384, 16_384)), Scanned::NoQr);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn a_file_over_20_mb_is_refused_before_it_is_read() {
    let dir = std::env::temp_dir().join(format!("evolite-qr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let big = dir.join("big.png");
    let file = std::fs::File::create(&big).unwrap();
    // Sparse where the file system allows: the length is what is checked.
    file.set_len(MAX_FILE_BYTES + 1).unwrap();
    drop(file);
    assert_eq!(scan_file(&big), Scanned::NotAPicture);
    assert_eq!(
        scan_bytes(&vec![0; MAX_FILE_BYTES as usize + 1]),
        Scanned::NotAPicture
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn other_formats_are_not_pictures_here() {
    let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec();
    assert_eq!(scan_bytes(&gif), Scanned::NotAPicture);
    assert_eq!(scan_bytes(b"not a picture"), Scanned::NotAPicture);
}

#[test]
fn a_jpeg_and_a_bmp_are_read() {
    let mut shot = picture(500, 500, 255);
    draw(&mut shot, ADDRESS, 6, (60, 60));
    for format in [image::ImageFormat::Jpeg, image::ImageFormat::Bmp] {
        let mut bytes = Vec::new();
        shot.write_to(&mut std::io::Cursor::new(&mut bytes), format)
            .unwrap();
        assert_eq!(
            scan_bytes(&bytes),
            found(ADDRESS, Kind::Address),
            "{format:?}"
        );
    }
}

/// The pictures in `tests/data/qr`: `<name>.png|jpg|bmp` and beside it `<name>.txt` with the
/// text the code holds, empty for a picture with no Dash code ([`write_the_test_pictures`]
/// made them; a real screenshot can be put beside them the same way).
#[test]
fn the_pictures_in_the_test_data_are_read() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/qr");
    let mut read = 0;
    for entry in std::fs::read_dir(&dir).expect("tests/data/qr").flatten() {
        let path = entry.path();
        if !is_picture_name(&path) {
            continue;
        }
        let want = std::fs::read_to_string(path.with_extension("txt"))
            .map(|t| t.trim().to_string())
            .ok();
        let got = scan_file(&path);
        match want.as_deref() {
            Some("") | None => assert!(
                matches!(got, Scanned::NoQr | Scanned::NotDash),
                "{}: {got:?}",
                path.display()
            ),
            Some(text) => assert_eq!(got, found(text, classify(text)), "{}", path.display()),
        }
        read += 1;
    }
    assert!(read >= 8, "{read} pictures in {}", dir.display());
}

/// Writes the pictures of `tests/data/qr` and the text each holds: screenshots of a window as
/// evolite's Receive draws its code (`receive::qr_card`: a white card, a quiet zone of two
/// modules, ink #0B0D12) at 100 % and 150 % in both themes, a photo of a screen (smaller,
/// darker, noisy, JPEG), DashPay's code with a user, a code that is not Dash and a window with
/// no code. Again after a change of the look: `cargo test -p evolite-ui --
/// --ignored write_the_test_pictures`.
#[test]
#[ignore = "writes the test data"]
fn write_the_test_pictures() {
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage, imageops};
    const INK: [u8; 3] = [0x0B, 0x0D, 0x12];
    fn fill(img: &mut RgbImage, at: (u32, u32), size: (u32, u32), color: [u8; 3]) {
        for y in at.1..(at.1 + size.1).min(img.height()) {
            for x in at.0..(at.0 + size.0).min(img.width()) {
                img.put_pixel(x, y, Rgb(color));
            }
        }
    }
    // A window at `scale`: its background, the sidebar and bars as lines of text.
    fn window(w: u32, h: u32, scale: f32, dark: bool) -> RgbImage {
        let (back, side, text) = if dark {
            ([0x12, 0x14, 0x1A], [0x1A, 0x1D, 0x25], [0x3A, 0x3F, 0x4B])
        } else {
            ([0xF6, 0xF7, 0xF9], [0xFF, 0xFF, 0xFF], [0xC9, 0xCD, 0xD6])
        };
        let s = |v: f32| (v * scale) as u32;
        let mut img = RgbImage::from_pixel(w, h, Rgb(back));
        fill(&mut img, (0, 0), (s(220.0), h), side);
        for i in 0..8 {
            fill(
                &mut img,
                (s(24.0), s(80.0 + 44.0 * i as f32)),
                (s(150.0), s(14.0)),
                text,
            );
        }
        fill(&mut img, (s(300.0), s(40.0)), (s(260.0), s(22.0)), text);
        img
    }
    // Receive's card with its top left corner at `at`.
    fn card(img: &mut RgbImage, text: &str, at: (u32, u32), scale: f32, line: [u8; 3]) {
        let side = (301.0 * scale) as u32;
        fill(img, at, (side, side), line);
        fill(img, (at.0 + 1, at.1 + 1), (side - 2, side - 2), [255; 3]);
        let code = qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::M).unwrap();
        let n = code.width() as u32;
        let module = ((261.0 * scale) / (n + 4) as f32).floor() as u32;
        let start = (side - module * (n + 4)) / 2 + module * 2;
        for (i, c) in code.to_colors().iter().enumerate() {
            if *c == qrcode::Color::Dark {
                let (x, y) = (i as u32 % n, i as u32 / n);
                let at = (at.0 + start + x * module, at.1 + start + y * module);
                fill(img, at, (module, module), INK);
            }
        }
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/qr");
    std::fs::create_dir_all(&dir).unwrap();
    let save = |name: &str, img: DynamicImage, text: &str| {
        let path = dir.join(name);
        let format = ImageFormat::from_path(&path).unwrap();
        img.save_with_format(&path, format).unwrap();
        std::fs::write(path.with_extension("txt"), text).unwrap();
    };
    let amount = format!("dash:{ADDRESS}?amount=0.25");
    let mut shot = window(1280, 800, 1.0, true);
    card(&mut shot, &amount, (520, 180), 1.0, [0x2A, 0x2E, 0x38]);
    save("receive-dark.png", shot.into(), &amount);

    let plain = format!("dash:{ADDRESS}");
    let mut shot = window(1920, 1200, 1.5, false);
    card(&mut shot, &plain, (780, 270), 1.5, [0xE3, 0xE5, 0xEA]);
    save("receive-light-150.jpg", shot.into(), &plain);

    // A phone's photo of the screen: smaller, darker, noisy (a fixed generator).
    let mut shot = window(1280, 800, 1.0, true);
    card(&mut shot, ADDRESS, (520, 180), 1.0, [0x2A, 0x2E, 0x38]);
    let mut photo = imageops::resize(&shot, 704, 440, imageops::FilterType::Triangle);
    let mut seed = 0x9e37_79b9_u32;
    for p in photo.pixels_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let noise = (seed % 33) as i32 - 16;
        for c in &mut p.0 {
            *c = ((*c as i32 * 4 / 5) + noise).clamp(0, 255) as u8;
        }
    }
    save("photo-of-a-screen.jpg", photo.into(), ADDRESS);

    let user = format!("dash:{ADDRESS}?user=satoshi");
    let mut shot = RgbImage::from_pixel(330, 330, Rgb([255; 3]));
    card(&mut shot, &user, (14, 14), 1.0, [0xE3, 0xE5, 0xEA]);
    save(
        "dashpay-user.bmp",
        DynamicImage::ImageRgb8(shot).grayscale(),
        &user,
    );

    let mut shot = window(1280, 800, 1.0, false);
    card(
        &mut shot,
        "https://example.com/pay?id=7",
        (520, 180),
        1.0,
        [0xE3, 0xE5, 0xEA],
    );
    save("not-dash.png", shot.into(), "");

    save("no-code.png", window(1280, 800, 1.0, true).into(), "");
}

#[test]
fn a_result_after_a_lock_is_dropped() {
    let ctx = egui::Context::default();
    let mut scanner = Scanner::default();
    let mut shot = picture(300, 300, 255);
    draw(&mut shot, ADDRESS, 4, (40, 40));
    let bytes: std::sync::Arc<[u8]> = png(&shot).into();
    scanner.start(&ctx, Picture::Bytes(bytes.clone()));
    assert!(scanner.busy());
    let wait = |s: &mut Scanner| {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let got = s.poll();
            if got.is_some() || !s.busy() || std::time::Instant::now() > until {
                return got;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    assert_eq!(wait(&mut scanner), Some(found(ADDRESS, Kind::Address)));
    scanner.start(&ctx, Picture::Bytes(bytes));
    on_lock();
    assert_eq!(wait(&mut scanner), None);
    assert!(!scanner.busy());
}

#[test]
fn the_clipboard_is_looked_at_only_when_the_menu_opens() {
    let ctx = egui::Context::default();
    let mut scanner = Scanner::default();
    assert_eq!(scanner.clip_picture, None);
    scanner.poll();
    assert_eq!(scanner.clip_picture, None, "no look without the menu");
    scanner.open_menu(&ctx);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while scanner.clip_picture.is_none() && std::time::Instant::now() < until {
        scanner.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(scanner.clip_picture, Some(false));
}
