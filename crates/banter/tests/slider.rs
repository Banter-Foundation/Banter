use banter::{slider_fraction, slider_track_width, SETTINGS_SLIDER_VALUE_W};

#[test]
fn track_leaves_room_for_the_value_text() {
    assert_eq!(slider_track_width(400.0), 400.0 - SETTINGS_SLIDER_VALUE_W);
    assert!(SETTINGS_SLIDER_VALUE_W >= 56.0, "room for \"2000 ms\" and \"-100 dB\"");
}

#[test]
fn track_never_goes_below_one_pixel() {
    assert_eq!(slider_track_width(10.0), 1.0);
    assert_eq!(slider_track_width(0.0), 1.0);
}

#[test]
fn fraction_maps_the_range_and_clamps() {
    assert_eq!(slider_fraction(0, 0, 200), 0.0);
    assert_eq!(slider_fraction(100, 0, 200), 0.5);
    assert_eq!(slider_fraction(200, 0, 200), 1.0);
    assert_eq!(slider_fraction(-50, -100, 0), 0.5);
    assert_eq!(slider_fraction(300, 0, 200), 1.0);
    assert_eq!(slider_fraction(-5, 0, 200), 0.0);
}

#[test]
fn fraction_of_an_empty_range_is_zero() {
    assert_eq!(slider_fraction(5, 5, 5), 0.0);
}
