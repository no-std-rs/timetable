use timetable_firmware::touch::{Sample, axs5106, cst816};
#[test]
fn axs5106_reference_packet_orientation_edges_release_and_invalid_coordinates() {
    let mut bytes = [0; 14];
    assert_eq!(axs5106(&bytes), Some(Sample::Released));
    bytes[1] = 1;
    assert_eq!(axs5106(&bytes), Some(Sample::Pressed { x: 171, y: 0 }));
    bytes[3] = 171;
    bytes[4] = 1;
    bytes[5] = 63;
    assert_eq!(axs5106(&bytes), Some(Sample::Pressed { x: 0, y: 319 }));
    bytes[3] = 172;
    assert_eq!(axs5106(&bytes), None);
    bytes[3] = 0;
    bytes[5] = 64;
    assert_eq!(axs5106(&bytes), None);
    bytes[1] = 0;
    assert_eq!(axs5106(&bytes), Some(Sample::Released));
}
#[test]
fn cst816_round_coordinates_remain_unchanged() {
    assert_eq!(
        cst816(&[1, 0, 239, 0, 239]),
        Some(Sample::Pressed { x: 239, y: 239 })
    );
    assert_eq!(cst816(&[1, 0, 240, 0, 0]), None);
    assert_eq!(cst816(&[0, 0, 0, 0, 0]), Some(Sample::Released));
}

#[test]
fn landscape_touch_axes_match_reference_rotation_and_reject_outside_glass() {
    use timetable_firmware::touch::axs5106_landscape;
    let mut data = [0; 14];
    assert_eq!(axs5106_landscape(&data), Some(Sample::Released));
    data[1] = 1;
    assert_eq!(
        axs5106_landscape(&data),
        Some(Sample::Pressed { x: 0, y: 0 })
    );
    data[3] = 171;
    data[4] = 1;
    data[5] = 63;
    assert_eq!(
        axs5106_landscape(&data),
        Some(Sample::Pressed { x: 319, y: 171 })
    );
    data[3] = 172;
    assert_eq!(axs5106_landscape(&data), None);
}
