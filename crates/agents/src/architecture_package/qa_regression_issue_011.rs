use super::comparison_id_safe;

#[test]
fn dotted_comparison_ids_are_stable_safe_keys() {
    assert!(comparison_id_safe("gate.dashboard.pass.mobile"));
    assert!(comparison_id_safe("P0-release_gate.desktop-1440"));
}

#[test]
fn comparison_ids_still_reject_path_and_control_characters() {
    for unsafe_id in ["../gate", "gate/pass", "gate pass", "gate\npass", ""] {
        assert!(!comparison_id_safe(unsafe_id), "accepted unsafe id: {unsafe_id:?}");
    }
}
