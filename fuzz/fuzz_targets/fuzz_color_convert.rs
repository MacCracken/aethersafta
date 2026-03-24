#![no_main]

use libfuzzer_sys::fuzz_target;

use aethersafta::encode::{argb_to_nv12, argb_to_yuv420p, nv12_to_argb};

/// Fuzz color conversion with arbitrary ARGB data.
///
/// Exercises: BT.709 YUV/NV12 conversion, odd dimensions, roundtrip stability.
/// Must never panic, and output dimensions must be correct.
fuzz_target!(|data: &[u8]| {
    if data.len() < 6 {
        return;
    }

    let width = ((data[0] as u32) % 64) + 1;
    let height = ((data[1] as u32) % 64) + 1;
    let argb_size = (width as usize) * (height as usize) * 4;

    // Need at least enough data to fill the ARGB buffer
    if data.len() < 2 + argb_size {
        return;
    }

    let argb = &data[2..2 + argb_size];

    // ARGB → YUV420p must not panic, output size must be correct
    let yuv = argb_to_yuv420p(argb, width, height);
    let cw = (width as usize + 1) / 2;
    let ch = (height as usize + 1) / 2;
    let expected_yuv = (width as usize) * (height as usize) + 2 * cw * ch;
    assert_eq!(yuv.len(), expected_yuv);

    // ARGB → NV12 must not panic, output size must be correct
    let nv12 = argb_to_nv12(argb, width, height);
    let expected_nv12 = (width as usize) * (height as usize) + cw * ch * 2;
    assert_eq!(nv12.len(), expected_nv12);

    // NV12 → ARGB roundtrip must not panic, output size must be correct
    let back = nv12_to_argb(&nv12, width, height);
    assert_eq!(back.len(), argb_size);

    // All alpha values in the output must be 255
    for chunk in back.chunks_exact(4) {
        assert_eq!(chunk[0], 255);
    }
});
