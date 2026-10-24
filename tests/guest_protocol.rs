#[allow(dead_code)]
#[path = "../experiments/cow-memory/guest-init.rs"]
mod guest_init;

#[test]
fn identity_and_network_setup_require_entropy_preparation() {
    let error = guest_init::identity_preparation_allowed(false).unwrap_err();
    assert!(error.contains("reseeded"));
    assert!(guest_init::identity_preparation_allowed(true).is_ok());
}

#[test]
fn extracts_dhcp_client_identifier_only_from_client_discover_packets() {
    let identifier = [0x00, 0x91, 0x82, 0x73, 0x64];
    let frame = dhcp_frame(68, 67, Some(&identifier));
    assert_eq!(
        guest_init::dhcpv4_client_identifier(&frame),
        Some(identifier.to_vec())
    );

    let reply = dhcp_frame(67, 68, Some(&identifier));
    assert_eq!(guest_init::dhcpv4_client_identifier(&reply), None);
    let missing = dhcp_frame(68, 67, None);
    assert_eq!(guest_init::dhcpv4_client_identifier(&missing), None);
}

#[test]
fn derives_a_stable_dhcp_client_identifier_from_each_machine_id() {
    let identifier = "00000000000000000000000000000001";
    let client_id = guest_init::machine_id_client_identifier(identifier).unwrap();
    assert_eq!(client_id.len(), 17);
    assert_eq!(client_id[0], 0);
    assert_eq!(client_id[16], 1);
    assert!(guest_init::machine_id_client_identifier("not-a-machine-id").is_err());
}

#[test]
fn counts_vmgenid_triggered_kernel_rng_reseed_messages() {
    let log = "[    1.0] crng reseeded due to virtual machine fork\n[    2.0] unrelated message\n[    3.0] crng reseeded due to virtual machine fork\n";
    assert_eq!(guest_init::vmgenid_reseed_event_count(log), 2);
    assert_eq!(guest_init::vmgenid_reseed_event_count("no fork event\n"), 0);
}

#[test]
fn sums_guest_local_timer_interrupt_counts() {
    let interrupts = "           CPU0       CPU1\nLOC:        123        124   Local timer interrupts\nRES:          3          4   Rescheduling interrupts\n";
    assert_eq!(
        guest_init::local_timer_interrupt_count(interrupts),
        Some(247)
    );
    assert_eq!(
        guest_init::local_timer_interrupt_count("no LOC row\n"),
        None
    );
}

#[test]
fn parses_guest_rtc_epoch_seconds() {
    assert_eq!(
        guest_init::rtc_epoch_seconds("1792822284\n"),
        Some(1_792_822_284)
    );
    assert_eq!(guest_init::rtc_epoch_seconds("not available\n"), None);
}

fn dhcp_frame(source_port: u16, destination_port: u16, client_id: Option<&[u8]>) -> Vec<u8> {
    let mut dhcp = vec![0u8; 240];
    dhcp[0] = 1;
    dhcp[236..240].copy_from_slice(&[99, 130, 83, 99]);
    dhcp.extend_from_slice(&[53, 1, 1]);
    if let Some(client_id) = client_id {
        dhcp.extend_from_slice(&[61, client_id.len() as u8]);
        dhcp.extend_from_slice(client_id);
    }
    dhcp.push(255);

    let ip_length = 20 + 8 + dhcp.len();
    let mut frame = vec![0u8; 14 + ip_length];
    frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    frame[14] = 0x45;
    frame[16..18].copy_from_slice(&(ip_length as u16).to_be_bytes());
    frame[23] = 17;
    frame[34..36].copy_from_slice(&source_port.to_be_bytes());
    frame[36..38].copy_from_slice(&destination_port.to_be_bytes());
    frame[38..40].copy_from_slice(&((8 + dhcp.len()) as u16).to_be_bytes());
    frame[42..].copy_from_slice(&dhcp);
    frame
}
