//! FEAT-0018: under «Picture link» in «Edit profile», Imgur as an example of where to put a
//! picture (frame AV1) and the warning for a link to an Imgur page (frame AV2).

use super::*;

const LINK: &str = "imgur.com, opens in the browser";
const PAGE: &str = "This is a link to an Imgur page";

/// The form's link as typed now.
fn typed(kit: &Kit) -> String {
    kit.state()
        .identity
        .editing
        .as_ref()
        .map(|f| f.avatar_url.clone())
        .unwrap_or_default()
}

/// AC-T3, AC-T5 (R-2, R-7): «imgur.com ↗» opens exactly Imgur's upload page, with nothing
/// added, and does nothing else: no command, the profile not saved, the dialog still open.
#[test]
fn the_imgur_link_opens_the_upload_page_and_nothing_else() {
    let (_turn, mut kit) = demo_app("edit-profile", "en");
    kit.set_size(egui::vec2(1024.0, 900.0));
    settle(&mut kit);
    assert!(has_words(&kit, "No link? Upload the picture"), "the hint");
    assert!(has_words(&kit, "Imgur is a third-party service"), "R-5");
    assert!(!has_words(&kit, PAGE), "no warning for an ordinary link");
    let before = crate::links::opened().len();
    kit.get_by_label(LINK).click();
    kit.run_steps(2);
    assert_eq!(
        crate::links::opened()[before..],
        ["https://imgur.com/upload".to_string()],
        "the upload page, nothing added"
    );
    let form = kit.state().identity.editing.as_ref().expect("still open");
    assert!(!form.waiting && form.result.is_none(), "nothing saved");
}

/// AC-T4 (R-4): a link to the page of one picture gets the warning at once, before «Save», in
/// place of the three lines; «Use the direct link» puts the picture's own address in the field
/// and saves nothing; then the warning goes and the hint is back.
#[test]
fn a_page_link_warns_and_the_direct_link_replaces_it() {
    let (_turn, mut kit) = demo_app("edit-profile", "en");
    kit.set_size(egui::vec2(1024.0, 900.0));
    settle(&mut kit);
    kit.state_mut()
        .identity
        .editing
        .as_mut()
        .unwrap()
        .avatar_url = "https://imgur.com/MXLYfwq".into();
    kit.run_steps(2);
    assert!(has_words(&kit, PAGE), "the warning at once");
    assert!(
        !has_words(&kit, "No link? Upload the picture"),
        "instead of the hint"
    );
    assert!(
        !has_words(&kit, "Imgur is a third-party service"),
        "and R-5"
    );
    click_words_of(&mut kit, "Use the direct link");
    assert_eq!(typed(&kit), "https://i.imgur.com/MXLYfwq.png");
    let form = kit.state().identity.editing.as_ref().unwrap();
    assert!(!form.waiting && form.result.is_none(), "nothing saved");
    kit.run_steps(2);
    assert!(!has_words(&kit, PAGE), "a direct link: no warning");
    assert!(has_words(&kit, "No link? Upload the picture"));
}

/// AC-T4 (R-4, frame AV2): an album gets the warning but no direct link: its id is not a
/// picture's.
#[test]
fn an_album_link_warns_without_a_direct_link() {
    let (_turn, mut kit) = demo_app("edit-profile-imgur-page", "en");
    kit.set_size(egui::vec2(1024.0, 900.0));
    settle(&mut kit);
    assert!(has_words(&kit, PAGE));
    assert!(
        kit.query_all_by_label("Use the direct link")
            .next()
            .is_none(),
        "no direct link from an album"
    );
}

/// AC-T4, AC-A6: Tab goes from the field to «imgur.com ↗»; with a page link, from the field to
/// «Use the direct link»; Enter on it uses the direct link.
#[test]
fn tab_reaches_the_link_and_the_direct_link() {
    let (_turn, mut kit) = demo_app("edit-profile", "en");
    kit.set_size(egui::vec2(1024.0, 900.0));
    settle(&mut kit);
    tab_to(&mut kit, "Picture link");
    tab(&mut kit, false);
    assert_eq!(focused(&kit).as_deref(), Some(LINK), "after the field");
    // A link without an extension, as if typed (the focus away from the hint, which goes).
    tab(&mut kit, true);
    kit.state_mut()
        .identity
        .editing
        .as_mut()
        .unwrap()
        .avatar_url = "https://i.imgur.com/MXLYfwq".into();
    kit.run_steps(2);
    assert_eq!(focused(&kit).as_deref(), Some("Picture link"));
    tab(&mut kit, false);
    assert_eq!(focused(&kit).as_deref(), Some("Use the direct link"));
    enter(&mut kit);
    assert_eq!(typed(&kit), "https://i.imgur.com/MXLYfwq.png");
}

/// AC-T4, AC-A6: in Arabic the warning is there and the field's address is kept as typed (left
/// to right), not mirrored.
#[test]
fn in_arabic_the_page_link_warns_and_the_address_is_not_mirrored() {
    let (_turn, mut kit) = demo_app("edit-profile-imgur-page", "ar");
    kit.set_size(egui::vec2(1024.0, 900.0));
    settle(&mut kit);
    assert!(
        has_words(&kit, "هذا رابط لصفحة على Imgur"),
        "the warning in Arabic"
    );
    assert_eq!(typed(&kit), "https://imgur.com/a/Xq3b9Tz");
    assert!(
        kit.query_all(By::new().value("https://imgur.com/a/Xq3b9Tz"))
            .next()
            .is_some(),
        "the field shows the address as typed"
    );
}
