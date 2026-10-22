use densevm::identity::CloneIdentity;
use std::collections::HashSet;

#[test]
fn identities_are_unique_across_batches_and_macs_are_local_unicast() {
    let mut ids = HashSet::new();
    let mut macs = HashSet::new();
    for _ in 0..64 {
        let identity = CloneIdentity::generate().unwrap();
        assert_eq!(identity.machine_id.len(), 32);
        assert!(identity.machine_id.bytes().all(|b| b.is_ascii_hexdigit()));
        let first = u8::from_str_radix(&identity.mac[..2], 16).unwrap();
        assert_eq!(first & 3, 2);
        assert!(ids.insert(identity.machine_id));
        assert!(macs.insert(identity.mac));
    }
}

#[test]
fn host_entropy_payload_is_fresh_and_not_serialized_in_identity_metadata() {
    let first = CloneIdentity::generate().unwrap();
    let second = CloneIdentity::generate().unwrap();
    assert_eq!(first.entropy_hex().len(), 64);
    assert_ne!(first.entropy_hex(), second.entropy_hex());
    let serialized = serde_json::to_string(&first).unwrap();
    assert!(!serialized.contains(&first.entropy_hex()));
}
