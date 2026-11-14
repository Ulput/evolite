//! FEAT-0018: Imgur as an example of where to put a profile picture. evolite uploads nothing
//! (R-1): the hint opens Imgur's upload page in the browser on a click, and a link typed into
//! «Picture link» that points at an Imgur page rather than at the picture gets a warning (R-4)
//! and, where the picture's own address follows from it, «Use the direct link» (АРХ-6).

/// Imgur's upload page: fixed, nothing about the wallet in it (R-2).
pub const UPLOAD: &str = "https://imgur.com/upload";

/// A link to Imgur that is not the picture itself (R-4, АРХ-6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageLink {
    /// The page of one picture (`imgur.com/<id>`), or the picture's address without an
    /// extension (`i.imgur.com/<id>`, which Imgur sends to that page): the picture is at
    /// `direct`. `.png` gives the original bytes whatever the picture's format (АРХ-6).
    Picture { direct: String },
    /// An album or a gallery (`imgur.com/a/<id>`, `imgur.com/gallery/<id>`): its id is not a
    /// picture's, so no direct link follows from it.
    Album,
}

impl PageLink {
    /// The picture's own address, when the link gives it.
    pub fn direct(&self) -> Option<&str> {
        match self {
            PageLink::Picture { direct } => Some(direct),
            PageLink::Album => None,
        }
    }
}

/// Imgur's own pages, whose names look like ids.
const NOT_IDS: [&str; 8] = [
    "upload", "signin", "register", "search", "about", "privacy", "random", "removed",
];

/// An Imgur id: Latin letters and digits, 5–10 of them, and not one of Imgur's own pages.
fn is_id(s: &str) -> bool {
    (5..=10).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_alphanumeric())
        && !NOT_IDS.contains(&s.to_ascii_lowercase().as_str())
}

/// What `url` is, when it is a link to an Imgur page rather than to a picture; anything else —
/// a direct link with an extension, another site, a site whose name only looks like Imgur's —
/// is not touched (`None`).
pub fn page_link(url: &str) -> Option<PageLink> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let rest = ["https://", "http://"]
        .iter()
        .find(|scheme| lower.starts_with(*scheme))
        .map(|scheme| &url[scheme.len()..])?;
    // The query and the fragment say nothing about which picture.
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let direct = |id: &str| PageLink::Picture {
        direct: format!("https://i.imgur.com/{id}.png"),
    };
    match (host.to_ascii_lowercase().as_str(), parts.as_slice()) {
        ("imgur.com" | "www.imgur.com", [id]) if is_id(id) => Some(direct(id)),
        ("imgur.com" | "www.imgur.com", [kind, id])
            if is_id(id) && matches!(kind.to_ascii_lowercase().as_str(), "a" | "gallery") =>
        {
            Some(PageLink::Album)
        }
        // `i.imgur.com/<id>.png` (a dot: an extension) is the picture: left as it is.
        ("i.imgur.com", [id]) if is_id(id) => Some(direct(id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(id: &str) -> Option<PageLink> {
        Some(PageLink::Picture {
            direct: format!("https://i.imgur.com/{id}.png"),
        })
    }

    /// AC-T2: the page of one picture gives its direct link, however the link is written.
    #[test]
    fn the_page_of_one_picture_gives_its_direct_link() {
        for url in [
            "https://imgur.com/MXLYfwq",
            "http://imgur.com/MXLYfwq",
            "https://www.imgur.com/MXLYfwq",
            "HTTPS://Imgur.com/MXLYfwq",
            "https://imgur.com/MXLYfwq/",
            "https://imgur.com/MXLYfwq?utm_source=x",
            "https://imgur.com/MXLYfwq#top",
            "  https://imgur.com/MXLYfwq  ",
        ] {
            assert_eq!(page_link(url), picture("MXLYfwq"), "{url}");
        }
        // The id keeps its case: Imgur's ids are case-sensitive.
        assert_eq!(page_link("https://imgur.com/AbCdE"), picture("AbCdE"));
    }

    /// AC-T2: an album or a gallery is a page too, with no direct link.
    #[test]
    fn an_album_is_a_page_without_a_direct_link() {
        for url in [
            "https://imgur.com/a/Xq3b9Tz",
            "https://www.imgur.com/gallery/Xq3b9Tz/",
            "http://imgur.com/A/Xq3b9Tz?x=1",
        ] {
            assert_eq!(page_link(url), Some(PageLink::Album), "{url}");
            assert_eq!(page_link(url).unwrap().direct(), None);
        }
    }

    /// AC-T2: the picture's address without an extension gets `.png`; with one it is left.
    #[test]
    fn a_direct_address_without_an_extension_gets_png() {
        assert_eq!(page_link("https://i.imgur.com/MXLYfwq"), picture("MXLYfwq"));
        for url in [
            "https://i.imgur.com/MXLYfwq.png",
            "https://i.imgur.com/MXLYfwq.jpg",
            "https://i.imgur.com/MXLYfwq.gifv",
        ] {
            assert_eq!(page_link(url), None, "{url}");
        }
    }

    /// AC-T2: other sites, look-alikes, odd ids and other paths are not touched.
    #[test]
    fn anything_else_is_not_touched() {
        for url in [
            "",
            "imgur.com/MXLYfwq",
            "ftp://imgur.com/MXLYfwq",
            "https://imgur.com.evil.example/MXLYfwq",
            "https://notimgur.com/MXLYfwq",
            "https://evil.example/imgur.com/MXLYfwq",
            "https://imgur.com/",
            "https://imgur.com/upload",
            "https://imgur.com/abcd",
            "https://imgur.com/abcdefghijk",
            "https://imgur.com/abc-def",
            "https://imgur.com/a/b/MXLYfwq",
            "https://imgur.com/user/MXLYfwq",
            "https://example.com/picture.png",
        ] {
            assert_eq!(page_link(url), None, "{url:?}");
        }
    }
}
