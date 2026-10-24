#[allow(dead_code)]
#[path = "../experiments/cow-memory/guest-init.rs"]
mod guest_init;

#[test]
fn identity_and_network_setup_require_entropy_preparation() {
    let error = guest_init::identity_preparation_allowed(false).unwrap_err();
    assert!(error.contains("reseeded"));
    assert!(guest_init::identity_preparation_allowed(true).is_ok());
}
