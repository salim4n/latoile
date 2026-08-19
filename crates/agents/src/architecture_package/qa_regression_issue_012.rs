use super::{validate_requested_locale, ValidatedPackage};
use latoile_core::ArchitectureVisualScenario;

fn package(locale: &str) -> ValidatedPackage {
    ValidatedPackage {
        package_digest: "a".repeat(64),
        manifest_digest: "b".repeat(64),
        file_count: 1,
        scenarios: vec![ArchitectureVisualScenario {
            comparison_id: "gate.pass.mobile".into(),
            screen: "release-gate".into(),
            state: "pass".into(),
            locale: locale.into(),
            theme: "light".into(),
            route: "/gate".into(),
            fixture: "synthetic-pass".into(),
            readiness_selector: "main".into(),
            stable_selectors: vec!["main".into()],
            allowed_masks: vec![],
            viewport_width: 390,
            viewport_height: 844,
            device_scale_factor_milli: 1000,
            mockup: "mockups/pass.html".into(),
        }],
    }
}

#[test]
fn package_scenarios_must_keep_the_owner_selected_locale() {
    assert!(validate_requested_locale(package("en-US"), "en-US").is_ok());
    let error = match validate_requested_locale(package("fr-FR"), "en-US") {
        Ok(_) => panic!("accepted a package in the wrong owner locale"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("owner-selected en-US locale"));
}
