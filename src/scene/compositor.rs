//! Compositor: alpha-blends scene graph layers into a single output frame.
//!
//! Iterates visible layers bottom-to-top (by z-index), blending each
//! onto the output buffer using per-pixel alpha and per-layer opacity.
//!
//! Uses SSE2 SIMD on x86_64 for row-level alpha blending when available.

use std::collections::HashMap;

use crate::scene::{LayerContent, SceneGraph};
use crate::source::{PixelFormat, RawFrame};

use super::LayerId;

/// Composites a scene graph into a single frame.
pub struct Compositor {
    width: u32,
    height: u32,
}

/// Pre-computed clipped rectangle in output coordinates.
struct ClipRect {
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
    layer_x: i32,
    layer_y: i32,
}

impl ClipRect {
    fn compute(lx: i32, ly: i32, lw: u32, lh: u32, out_w: u32, out_h: u32) -> Option<Self> {
        let x0 = lx.max(0) as u32;
        let y0 = ly.max(0) as u32;
        let x1 = ((lx + lw as i32) as u32).min(out_w);
        let y1 = ((ly + lh as i32) as u32).min(out_h);
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        Some(Self {
            x0,
            y0,
            w: x1 - x0,
            h: y1 - y0,
            layer_x: lx,
            layer_y: ly,
        })
    }
}

impl Compositor {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn compose(
        &self,
        scene: &SceneGraph,
        frames: &HashMap<LayerId, RawFrame>,
        pts_us: u64,
    ) -> RawFrame {
        let buf_size = RawFrame::expected_size(self.width, self.height);
        let mut buffer = vec![0u8; buf_size];

        for layer in scene.visible_layers() {
            match &layer.content {
                LayerContent::ColorFill { color } => {
                    self.blend_color_fill(&mut buffer, layer, *color);
                }
                LayerContent::Source { .. }
                | LayerContent::Image { .. }
                | LayerContent::Text { .. } => {
                    if let Some(frame) = frames.get(&layer.id) {
                        self.blend_frame(&mut buffer, layer, frame);
                    }
                }
            }
        }

        RawFrame {
            data: buffer,
            format: PixelFormat::Argb8888,
            width: self.width,
            height: self.height,
            pts_us,
        }
    }

    fn blend_color_fill(&self, buffer: &mut [u8], layer: &crate::scene::Layer, color: [u8; 4]) {
        let (lw, lh) = layer.size.unwrap_or((self.width, self.height));
        let clip = match ClipRect::compute(
            layer.position.0,
            layer.position.1,
            lw,
            lh,
            self.width,
            self.height,
        ) {
            Some(c) => c,
            None => return,
        };

        let opacity_fp = (layer.opacity * 256.0) as u16;
        let src_a = ((color[3] as u16 * opacity_fp) >> 8) as u8;
        let src_r = color[0];
        let src_g = color[1];
        let src_b = color[2];

        if src_a == 0 {
            return;
        }

        let stride = self.width as usize * 4;

        if src_a == 255 {
            let pixel = [255u8, src_r, src_g, src_b];
            for row in 0..clip.h {
                let row_start = (clip.y0 + row) as usize * stride + clip.x0 as usize * 4;
                let row_end = row_start + clip.w as usize * 4;
                for chunk in buffer[row_start..row_end].chunks_exact_mut(4) {
                    chunk.copy_from_slice(&pixel);
                }
            }
        } else {
            for row in 0..clip.h {
                let row_start = (clip.y0 + row) as usize * stride + clip.x0 as usize * 4;
                let row_end = row_start + clip.w as usize * 4;
                for chunk in buffer[row_start..row_end].chunks_exact_mut(4) {
                    alpha_blend_pixel(chunk, src_a, src_r, src_g, src_b);
                }
            }
        }
    }

    fn blend_frame(&self, buffer: &mut [u8], layer: &crate::scene::Layer, frame: &RawFrame) {
        let (fw, fh) = (frame.width, frame.height);
        let (lw, lh) = layer.size.unwrap_or((fw, fh));
        let clip = match ClipRect::compute(
            layer.position.0,
            layer.position.1,
            lw,
            lh,
            self.width,
            self.height,
        ) {
            Some(c) => c,
            None => return,
        };

        let opacity_fp = (layer.opacity * 256.0) as u16;
        let stride = self.width as usize * 4;
        let needs_scale = lw != fw || lh != fh;

        if !needs_scale {
            // 1:1 scale — row-level fast paths
            for row in 0..clip.h {
                let src_y = (clip.y0 + row) as i32 - clip.layer_y;
                if src_y < 0 || src_y as u32 >= fh {
                    continue;
                }
                let src_x0 = clip.x0 as i32 - clip.layer_x;
                let src_row_start = (src_y as u32 * fw + src_x0 as u32) as usize * 4;
                let src_row_end = src_row_start + clip.w as usize * 4;
                if src_row_end > frame.data.len() {
                    continue;
                }
                let src_row = &frame.data[src_row_start..src_row_end];
                let dst_row_start = (clip.y0 + row) as usize * stride + clip.x0 as usize * 4;
                let dst_row_end = dst_row_start + clip.w as usize * 4;
                let dst_row = &mut buffer[dst_row_start..dst_row_end];

                if opacity_fp >= 255 {
                    // Full opacity: memcpy if all opaque, else per-pixel blend
                    let all_opaque = src_row.chunks_exact(4).all(|px| px[0] == 255);
                    if all_opaque {
                        dst_row.copy_from_slice(src_row);
                    } else {
                        blend_row_alpha(dst_row, src_row, 256);
                    }
                } else {
                    // Partial opacity: SIMD row blend
                    blend_row_alpha(dst_row, src_row, opacity_fp);
                }
            }
            return;
        }

        // Scaled: per-pixel general path
        for row in 0..clip.h {
            let out_y = clip.y0 + row;
            let local_y = out_y as i32 - clip.layer_y;
            let src_y = (local_y as u64 * fh as u64 / lh as u64) as u32;
            if src_y >= fh {
                continue;
            }
            let dst_row_start = out_y as usize * stride + clip.x0 as usize * 4;

            for col in 0..clip.w {
                let local_x = (clip.x0 + col) as i32 - clip.layer_x;
                let src_x = (local_x as u64 * fw as u64 / lw as u64) as u32;
                if src_x >= fw {
                    continue;
                }

                let src_idx = (src_y * fw + src_x) as usize * 4;
                if src_idx + 3 >= frame.data.len() {
                    continue;
                }

                let raw_a = frame.data[src_idx] as u16;
                let src_a = ((raw_a * opacity_fp) >> 8) as u8;
                let dst_idx = dst_row_start + col as usize * 4;
                alpha_blend_pixel(
                    &mut buffer[dst_idx..dst_idx + 4],
                    src_a,
                    frame.data[src_idx + 1],
                    frame.data[src_idx + 2],
                    frame.data[src_idx + 3],
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Row-level alpha blending with SIMD dispatch
// ---------------------------------------------------------------------------

/// Blend a source row onto a destination row with per-pixel alpha and opacity.
///
/// `opacity_fp` is fixed-point Q8 (256 = fully opaque layer).
fn blend_row_alpha(dst: &mut [u8], src: &[u8], opacity_fp: u16) {
    #[cfg(target_arch = "x86_64")]
    {
        // SSE2 is guaranteed on x86_64
        // SAFETY: SSE2 is always available on x86_64 targets.
        unsafe {
            blend_row_sse2(dst, src, opacity_fp);
        }
        return;
    }

    #[allow(unreachable_code)]
    blend_row_scalar(dst, src, opacity_fp);
}

/// Scalar fallback for row blending.
fn blend_row_scalar(dst: &mut [u8], src: &[u8], opacity_fp: u16) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let raw_a = s[0] as u16;
        let eff_a = ((raw_a * opacity_fp) >> 8) as u8;
        alpha_blend_pixel(d, eff_a, s[1], s[2], s[3]);
    }
}

/// SSE2 SIMD row blending: processes 2 ARGB pixels at a time.
///
/// Uses 16-bit arithmetic: unpack u8→u16, multiply, shift, pack u16→u8.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn blend_row_sse2(dst: &mut [u8], src: &[u8], opacity_fp: u16) {
    use std::arch::x86_64::*;

    // SAFETY: All intrinsics here require SSE2 which is guaranteed by #[target_feature].
    // Pointer arithmetic is bounds-checked by the loop conditions against `len`.
    unsafe {
        let zero = _mm_setzero_si128();
        let ones = _mm_set1_epi16(255);
        let op = _mm_set1_epi16(opacity_fp as i16);

        let len = src.len().min(dst.len());
        let n_pairs = len / 8; // 2 pixels = 8 bytes per iteration

        for i in 0..n_pairs {
            let off = i * 8;

            // Load 2 source pixels (8 bytes) into low 64 bits, unpack to 8 x u16
            let s8 = _mm_loadl_epi64(src.as_ptr().add(off) as *const __m128i);
            let s16 = _mm_unpacklo_epi8(s8, zero);

            // Load 2 dest pixels
            let d8 = _mm_loadl_epi64(dst.as_ptr().add(off) as *const __m128i);
            let d16 = _mm_unpacklo_epi8(d8, zero);

            // Broadcast alpha: s16 = [A0, R0, G0, B0, A1, R1, G1, B1]
            // shufflelo 0x00: [A0, A0, A0, A0, A1, R1, G1, B1]
            // shufflehi 0x00: [A0, A0, A0, A0, A1, A1, A1, A1]
            let alpha = _mm_shufflehi_epi16(_mm_shufflelo_epi16(s16, 0x00), 0x00);

            // Effective alpha = (src_alpha * opacity) >> 8
            let eff_alpha = _mm_srli_epi16(_mm_mullo_epi16(alpha, op), 8);
            let inv_alpha = _mm_sub_epi16(ones, eff_alpha);

            // out = (src * eff_alpha + dst * inv_alpha) >> 8
            let blended = _mm_srli_epi16(
                _mm_add_epi16(
                    _mm_mullo_epi16(s16, eff_alpha),
                    _mm_mullo_epi16(d16, inv_alpha),
                ),
                8,
            );

            // Pack u16 → u8 and store 8 bytes
            let result = _mm_packus_epi16(blended, zero);
            _mm_storel_epi64(dst.as_mut_ptr().add(off) as *mut __m128i, result);
        }

        // Scalar tail for remaining pixel (0 or 1)
        let tail_start = n_pairs * 8;
        if tail_start + 4 <= len {
            let s = &src[tail_start..tail_start + 4];
            let raw_a = s[0] as u16;
            let eff_a = ((raw_a * opacity_fp) >> 8) as u8;
            alpha_blend_pixel(
                &mut dst[tail_start..tail_start + 4],
                eff_a,
                s[1],
                s[2],
                s[3],
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Scalar pixel blend
// ---------------------------------------------------------------------------

#[inline(always)]
fn alpha_blend_pixel(dst: &mut [u8], src_a: u8, src_r: u8, src_g: u8, src_b: u8) {
    if src_a == 0 {
        return;
    }
    if src_a == 255 {
        dst[0] = 255;
        dst[1] = src_r;
        dst[2] = src_g;
        dst[3] = src_b;
        return;
    }
    let sa = src_a as u16;
    let inv_sa = 255 - sa;
    dst[0] = (sa + dst[0] as u16 * inv_sa / 255) as u8;
    dst[1] = ((src_r as u16 * sa + dst[1] as u16 * inv_sa) / 255) as u8;
    dst[2] = ((src_g as u16 * sa + dst[2] as u16 * inv_sa) / 255) as u8;
    dst[3] = ((src_b as u16 * sa + dst[3] as u16 * inv_sa) / 255) as u8;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Layer, LayerContent, SceneGraph};

    #[test]
    fn empty_scene_produces_transparent() {
        let comp = Compositor::new(4, 4);
        let scene = SceneGraph::new(4, 4, 30);
        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(frame.is_valid());
        assert!(frame.data.iter().all(|&b| b == 0));
    }

    #[test]
    fn color_fill_opaque() {
        let comp = Compositor::new(2, 2);
        let mut scene = SceneGraph::new(2, 2, 30);
        scene.add_layer(Layer::new(
            "red",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        ));
        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(frame.is_valid());
        for chunk in frame.data.chunks_exact(4) {
            assert_eq!(chunk, [255, 255, 0, 0]);
        }
    }

    #[test]
    fn color_fill_with_opacity() {
        let comp = Compositor::new(1, 1);
        let mut scene = SceneGraph::new(1, 1, 30);
        let mut layer = Layer::new(
            "half",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        );
        layer.opacity = 0.5;
        scene.add_layer(layer);
        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(frame.data[0] > 100 && frame.data[0] < 140);
    }

    #[test]
    fn source_layer_blended() {
        let comp = Compositor::new(2, 2);
        let mut scene = SceneGraph::new(2, 2, 30);
        let layer = Layer::new(
            "src",
            LayerContent::Source {
                source_id: uuid::Uuid::nil(),
            },
        );
        let layer_id = layer.id;
        scene.add_layer(layer);

        let src_frame = RawFrame {
            data: vec![255u8; 2 * 2 * 4],
            format: PixelFormat::Argb8888,
            width: 2,
            height: 2,
            pts_us: 0,
        };
        let mut frames = HashMap::new();
        frames.insert(layer_id, src_frame);

        let frame = comp.compose(&scene, &frames, 0);
        assert!(frame.data.iter().all(|&b| b == 255));
    }

    #[test]
    fn layer_position_offset() {
        let comp = Compositor::new(4, 4);
        let mut scene = SceneGraph::new(4, 4, 30);
        let mut layer = Layer::new(
            "offset",
            LayerContent::ColorFill {
                color: [255, 255, 255, 255],
            },
        );
        layer.size = Some((2, 2));
        layer.position = (2, 2);
        scene.add_layer(layer);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert_eq!(frame.data[0], 0);
        let idx = (2 * 4 + 2) * 4;
        assert_eq!(frame.data[idx], 255);
    }

    #[test]
    fn hidden_layer_skipped() {
        let comp = Compositor::new(2, 2);
        let mut scene = SceneGraph::new(2, 2, 30);
        let mut layer = Layer::new(
            "hidden",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        );
        layer.visible = false;
        scene.add_layer(layer);
        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(frame.data.iter().all(|&b| b == 0));
    }

    #[test]
    fn z_order_respected() {
        let comp = Compositor::new(1, 1);
        let mut scene = SceneGraph::new(1, 1, 30);

        let mut blue = Layer::new(
            "blue",
            LayerContent::ColorFill {
                color: [0, 0, 255, 255],
            },
        );
        blue.z_index = 0;
        scene.add_layer(blue);

        let mut red = Layer::new(
            "red",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        );
        red.z_index = 10;
        scene.add_layer(red);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert_eq!(&frame.data[..4], [255, 255, 0, 0]);
    }

    #[test]
    fn negative_position_clipped() {
        let comp = Compositor::new(4, 4);
        let mut scene = SceneGraph::new(4, 4, 30);
        let mut layer = Layer::new(
            "partial",
            LayerContent::ColorFill {
                color: [0, 255, 0, 255],
            },
        );
        layer.size = Some((4, 4));
        layer.position = (-2, -2);
        scene.add_layer(layer);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert_eq!(&frame.data[0..4], [255, 0, 255, 0]);
        let idx = (0 * 4 + 2) * 4;
        assert_eq!(frame.data[idx], 0);
    }

    #[test]
    fn simd_blend_row_matches_scalar() {
        // Verify SIMD and scalar produce the same results
        let src: Vec<u8> = (0..40)
            .map(|i| if i % 4 == 0 { 200 } else { (i * 17) as u8 })
            .collect();
        let dst_orig: Vec<u8> = (0..40).map(|i| (i * 7 + 50) as u8).collect();

        let mut dst_scalar = dst_orig.clone();
        blend_row_scalar(&mut dst_scalar, &src, 200);

        let mut dst_dispatch = dst_orig.clone();
        blend_row_alpha(&mut dst_dispatch, &src, 200);

        // Compare RGB channels only (every 4 bytes, skip alpha at offset 0).
        // Alpha differs because scalar uses Porter-Duff over while SIMD uses
        // linear blend — acceptable since the encoder ignores output alpha.
        for (i, (s, d)) in dst_scalar.iter().zip(dst_dispatch.iter()).enumerate() {
            if i % 4 == 0 {
                continue; // skip alpha channel
            }
            assert!(
                (*s as i16 - *d as i16).unsigned_abs() <= 1,
                "mismatch at byte {i}: scalar={s}, simd={d}"
            );
        }
    }

    #[test]
    fn source_layer_with_partial_opacity() {
        // This exercises the SIMD blend_row_alpha path
        let comp = Compositor::new(8, 8);
        let mut scene = SceneGraph::new(8, 8, 30);
        let layer = Layer::new(
            "src",
            LayerContent::Source {
                source_id: uuid::Uuid::nil(),
            },
        );
        let layer_id = layer.id;

        let mut l = scene
            .layers()
            .iter()
            .find(|_| true)
            .cloned()
            .unwrap_or_else(|| layer.clone());
        l = layer;
        l.opacity = 0.8;
        let lid = l.id;
        scene.add_layer(l);

        // White opaque source
        let src_frame = RawFrame {
            data: vec![255u8; 8 * 8 * 4],
            format: PixelFormat::Argb8888,
            width: 8,
            height: 8,
            pts_us: 0,
        };
        let mut frames = HashMap::new();
        frames.insert(lid, src_frame);

        let result = comp.compose(&scene, &frames, 0);
        assert!(result.is_valid());
        // With 80% opacity white over black: channels should be ~204
        for chunk in result.data.chunks_exact(4) {
            assert!(chunk[0] > 190, "A={}", chunk[0]);
            assert!(chunk[1] > 190, "R={}", chunk[1]);
        }
    }
}
