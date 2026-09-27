//! OPDS/Atom feed parsing, ported from `tools/opds-sync.py` (Atom namespace
//! `http://www.w3.org/2005/Atom`, navigation vs acquisition link detection,
//! extension guessing from content-type/URL, and urljoin).

use roxmltree::Node;

pub const ATOM_NS: &str = "http://www.w3.org/2005/Atom";

/// OPDS acquisition relation URIs (OPDS 1.x).
pub const ACQUISITION_RELS: [&str; 5] = [
    "http://opds-spec.org/acquisition",
    "http://opds-spec.org/acquisition/open-access",
    "http://opds-spec.org/acquisition/borrow",
    "http://opds-spec.org/acquisition/buy",
    "http://opds-spec.org/acquisition/sample",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub rel: String,
    pub href: String,
    pub link_type: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub title: String,
    pub links: Vec<Link>,
}

/// Atom direct children of `node` with the given tag name (namespace-aware).
fn atom_children<'a, 'i>(node: Node<'a, 'i>, tag: &str) -> Vec<Node<'a, 'i>> {
    node.children()
        .filter(|c| {
            c.is_element()
                && c.tag_name().name() == tag
                && c.tag_name().namespace() == Some(ATOM_NS)
        })
        .collect()
}

/// Parse an OPDS/Atom feed into `(feed_title, entries)`. `Err` on malformed
/// XML so the sync engine can distinguish "empty feed" from "parse failure".
pub fn parse_feed(body: &str) -> Result<(String, Vec<Entry>), String> {
    let doc = roxmltree::Document::parse(body).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if root.tag_name().name() != "feed" || root.tag_name().namespace() != Some(ATOM_NS) {
        return Err("not an Atom feed".to_string());
    }
    let feed_title = atom_children(root, "title")
        .first()
        .and_then(|t| t.text())
        .unwrap_or("")
        .trim()
        .to_string();

    let mut entries = Vec::new();
    for entry_el in atom_children(root, "entry") {
        let title = atom_children(entry_el, "title")
            .first()
            .and_then(|t| t.text())
            .unwrap_or("Untitled")
            .trim()
            .to_string();
        let mut links = Vec::new();
        for link_el in atom_children(entry_el, "link") {
            links.push(Link {
                rel: link_el.attribute("rel").unwrap_or("").to_string(),
                href: link_el.attribute("href").unwrap_or("").to_string(),
                link_type: link_el.attribute("type").unwrap_or("").to_string(),
            });
        }
        entries.push(Entry { title, links });
    }
    Ok((feed_title, entries))
}

/// Port of `is_navigation_link`: explicit `subsection`, or an un-typed /
/// `alternate` Atom link whose type is an OPDS catalog feed.
pub fn is_navigation_link(rel: &str, type_: &str) -> bool {
    if rel == "subsection" {
        return true;
    }
    (rel.is_empty() || rel == "alternate")
        && type_.starts_with("application/atom+xml")
        && type_.contains("profile=opds-catalog")
}

/// Port of `is_acquisition_link`: rel is one of the OPDS acquisition URIs.
pub fn is_acquisition_link(rel: &str) -> bool {
    ACQUISITION_RELS.contains(&rel)
}

/// Port of `guess_ext_from_type_or_url`: map the content type, else the URL
/// path extension, else `.bin`. Returns an extension including the dot.
pub fn guess_ext(content_type: &str, href: &str) -> String {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    const MAPPING: [(&str, &str); 9] = [
        ("application/epub+zip", ".epub"),
        ("application/pdf", ".pdf"),
        ("application/x-mobipocket-ebook", ".mobi"),
        ("application/vnd.amazon.ebook", ".azw"),
        ("application/vnd.amazon.mobi8-ebook", ".azw3"),
        ("text/plain", ".txt"),
        ("application/x-cbz", ".cbz"),
        ("application/x-cbr", ".cbr"),
        ("application/x-fictionbook+xml", ".fb2"),
    ];
    for (key, ext) in MAPPING {
        if ct == key {
            return ext.to_string();
        }
    }
    // Fall back to the URL path extension (mirrors Python's urlparse + splitext).
    let path = match url::Url::parse(href) {
        Ok(u) => u.path().to_string(),
        Err(_) => href.to_string(),
    };
    let ext = std::path::Path::new(&path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext.is_empty() {
        ".bin".to_string()
    } else {
        format!(".{}", ext)
    }
}

/// Port of Python's `urllib.parse.urljoin`: resolve `href` against `base`
/// (handles relative, absolute and query-bearing hrefs). Falls back to `href`
/// itself if the base is not a valid URL.
pub fn urljoin(base: &str, href: &str) -> String {
    match url::Url::parse(base).and_then(|u| u.join(href)) {
        Ok(u) => u.to_string(),
        Err(_) => href.to_string(),
    }
}

/// Find the feed-level `<link rel="next">` href, resolved against `base`
/// (OPDS pagination; the next page is walked with the SAME folder).
pub fn next_href(body: &str, base: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(body).ok()?;
    let root = doc.root_element();
    for link_el in atom_children(root, "link") {
        if link_el.attribute("rel") == Some("next") {
            return link_el.attribute("href").map(|h| urljoin(base, h));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Root Catalog</title>
  <link rel="next" href="page2?offset=20" type="application/atom+xml;profile=opds-catalog"/>
  <entry>
    <title>Science Fiction</title>
    <link rel="subsection" href="sf.xml" type="application/atom+xml;profile=opds-catalog"/>
  </entry>
  <entry>
    <title>Dune</title>
    <link rel="http://opds-spec.org/acquisition" href="dune.epub" type="application/epub+zip"/>
    <link rel="alternate" href="https://cdn.example.com/dune.epub?token=1" type="application/epub+zip"/>
  </entry>
  <entry>
    <title>No links here</title>
  </entry>
</feed>"#;

    #[test]
    fn parses_feed_fixture() {
        let (title, entries) = parse_feed(FIXTURE).expect("parse");
        assert_eq!(title, "Root Catalog");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].title, "Science Fiction");
        assert_eq!(entries[0].links.len(), 1);
        assert_eq!(entries[0].links[0].rel, "subsection");
        assert_eq!(entries[0].links[0].href, "sf.xml");
        assert_eq!(
            entries[0].links[0].link_type,
            "application/atom+xml;profile=opds-catalog"
        );
        assert_eq!(entries[1].title, "Dune");
        assert_eq!(entries[1].links.len(), 2);
        assert_eq!(entries[2].links.len(), 0);
    }

    #[test]
    fn malformed_xml_is_an_error() {
        assert!(parse_feed("<feed><entry></feed>").is_err());
    }

    #[test]
    fn non_atom_xml_is_an_error() {
        assert!(parse_feed("<html><body>Sign in</body></html>").is_err());
        assert!(parse_feed("<feed><title>Missing namespace</title></feed>").is_err());
    }

    #[test]
    fn navigation_link_detection() {
        assert!(is_navigation_link("subsection", ""));
        assert!(is_navigation_link(
            "subsection",
            "application/atom+xml;profile=opds-catalog"
        ));
        assert!(is_navigation_link(
            "",
            "application/atom+xml;profile=opds-catalog"
        ));
        assert!(is_navigation_link(
            "alternate",
            "application/atom+xml; profile=opds-catalog"
        ));
        // type must actually be an OPDS catalog feed
        assert!(!is_navigation_link("alternate", "application/epub+zip"));
        assert!(!is_navigation_link("", "application/atom+xml"));
        assert!(!is_navigation_link(
            "self",
            "application/atom+xml;profile=opds-catalog"
        ));
        assert!(!is_navigation_link(
            "next",
            "application/atom+xml;profile=opds-catalog"
        ));
    }

    #[test]
    fn acquisition_link_detection() {
        for rel in ACQUISITION_RELS {
            assert!(is_acquisition_link(rel), "{rel}");
        }
        assert!(!is_acquisition_link("subsection"));
        assert!(!is_acquisition_link(""));
        assert!(!is_acquisition_link("alternate"));
    }

    #[test]
    fn guess_extension_from_type_and_url() {
        assert_eq!(guess_ext("application/epub+zip", "http://x/book"), ".epub");
        assert_eq!(guess_ext("text/html", "http://x/book.azw3"), ".azw3");
        assert_eq!(guess_ext("", "http://x/file.PDF"), ".pdf");
        assert_eq!(
            guess_ext("application/x-cbz; charset=utf-8", "http://x/c"),
            ".cbz"
        );
        assert_eq!(
            guess_ext("application/vnd.amazon.mobi8-ebook", "http://x/f"),
            ".azw3"
        );
        assert_eq!(guess_ext("", "http://x/noext"), ".bin");
        assert_eq!(
            guess_ext("", "https://cdn.example.com/dune.epub?token=1"),
            ".epub"
        );
    }

    #[test]
    fn urljoin_relative_absolute_query() {
        assert_eq!(
            urljoin("http://host/root/feed.xml", "sub.xml"),
            "http://host/root/sub.xml"
        );
        assert_eq!(
            urljoin("http://host/root/", "../x.epub"),
            "http://host/x.epub"
        );
        assert_eq!(
            urljoin("http://host/root/feed.xml", "https://other/x.azw3"),
            "https://other/x.azw3"
        );
        assert_eq!(
            urljoin("http://host/root/feed.xml", "next?page=2"),
            "http://host/root/next?page=2"
        );
        assert_eq!(
            urljoin("http://host/root/feed.xml", "//other/x.mobi"),
            "http://other/x.mobi"
        );
    }

    #[test]
    fn next_link_resolution() {
        assert_eq!(
            next_href(FIXTURE, "http://host/root/feed.xml"),
            Some("http://host/root/page2?offset=20".to_string())
        );
        assert_eq!(
            next_href(
                "<feed xmlns=\"http://www.w3.org/2005/Atom\"/>",
                "http://h/f"
            ),
            None
        );
    }
}
