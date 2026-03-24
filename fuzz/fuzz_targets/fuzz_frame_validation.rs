#![no_main]

use libfuzzer_sys::fuzz_target;

use aethersafta::source::{PixelFormat, RawFrame};

/// Fuzz RawFrame validation with arbitrary data.
///
/// Exercises: expected_size_for, is_valid, edge cases with dimensions.
fuzz_target!(|data: &[u8]| {
    if data.len() < 9 {
        return;
    }

    let width = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let height = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let format = if data[8] & 1 == 0 {
        PixelFormat::Argb8888
    } else {
        PixelFormat::Nv12
    };

    // Clamp to avoid OOM
    if width > 4096 || height > 4096 || width == 0 || height == 0 {
        return;
    }

    let expected = RawFrame::expected_size_for(format, width, height);

    // Construct a frame with the remaining fuzz data
    let frame_data = &data[9..];
    let frame = RawFrame {
        data: frame_data.to_vec().into(),
        format,
        width,
        height,
        pts_us: 0,
    };

    // is_valid should match our expectation
    assert_eq!(frame.is_valid(), frame_data.len() == expected);
});
