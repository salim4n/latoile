use super::package_copy;

#[test]
fn final_package_copy_uses_the_pinned_session_locale() {
    let english = package_copy("en-US", 2);
    assert_eq!(english.title, "Architecture package v2 ready");
    assert!(english.message.starts_with("The Architect"));
    assert_eq!(english.files, "files");

    let french = package_copy("fr-FR", 2);
    assert_eq!(french.title, "Paquet architecture v2 prêt");
    assert!(french.message.starts_with("L'Architecte"));
    assert_eq!(french.files, "fichiers");
}
