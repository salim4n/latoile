use super::html_is_self_contained;

#[test]
fn mockup_can_navigate_to_its_package_gallery_without_becoming_external() {
    let mockup = r#"<!doctype html>
<html data-latoile-token-digest="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa">
  <body><a href="../gallery.html">Back to gallery</a></body>
</html>"#;

    assert!(html_is_self_contained(&mockup.to_ascii_lowercase()));
}

#[test]
fn mockup_cannot_use_parent_navigation_for_any_other_resource() {
    for unsafe_href in [
        "../secrets.html",
        "../../gallery.html",
        "../assets/theme.css",
    ] {
        let mockup = format!(r#"<html><body><a href="{unsafe_href}">escape</a></body></html>"#);
        assert!(
            !html_is_self_contained(&mockup.to_ascii_lowercase()),
            "accepted unsafe package navigation: {unsafe_href}"
        );
    }
}
