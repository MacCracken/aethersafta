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
///
/// Stores a reusable buffer to eliminate per-frame heap allocation.
/// Call [`reclaim_buffer()`](Self::reclaim_buffer) to return a frame's buffer
/// after use, enabling zero-allocation steady-state compositing.
pub struct Compositor {
    width: u32,
    height: u32,
    /// Reusable compositing buffer (ARGB8888, width × height × 4 bytes).
    scratch: Vec<u8>,
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
    #[inline]
    fn compute(lx: i32, ly: i32, lw: u32, lh: u32, out_w: u32, out_h: u32) -> Option<Self> {
        let x0 = lx.max(0) as u32;
        let y0 = ly.max(0) as u32;
        // Use i64 to avoid overflow when position + size exceeds i32 range
        let x1_i64 = (lx as i64 + lw as i64).clamp(0, out_w as i64);
        let y1_i64 = (ly as i64 + lh as i64).clamp(0, out_h as i64);
        let x1 = x1_i64 as u32;
        let y1 = y1_i64 as u32;
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
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let buf_size = RawFrame::expected_size(width, height);
        Self {
            width,
            height,
            scratch: vec![0u8; buf_size],
        }
    }

    #[must_use]
    pub fn compose(
        &mut self,
        scene: &SceneGraph,
        frames: &HashMap<LayerId, RawFrame>,
        pts_us: u64,
    ) -> RawFrame {
        let buf_size = RawFrame::expected_size(self.width, self.height);

        // Reuse scratch if available, otherwise allocate
        let mut buffer = if self.scratch.len() == buf_size {
            let mut buf = std::mem::take(&mut self.scratch);
            buf.fill(0);
            buf
        } else {
            vec![0u8; buf_size]
        };

        for layer in scene.layers().iter().filter(|l| l.visible) {
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

    /// Return a previously composed frame's data buffer for reuse.
    ///
    /// Call this after you're done with a [`RawFrame`] (e.g. after encoding)
    /// to avoid heap allocation on the next [`compose()`](Self::compose) call.
    ///
    /// ```rust,no_run
    /// # use aethersafta::Compositor;
    /// # let mut comp = Compositor::new(1920, 1080);
    /// # let scene = aethersafta::SceneGraph::new(1920, 1080, 30);
    /// let frame = comp.compose(&scene, &Default::default(), 0);
    /// // ... encode frame ...
    /// comp.reclaim_buffer(frame.data);
    /// ```
    pub fn reclaim_buffer(&mut self, buf: Vec<u8>) {
        self.scratch = buf;
    }

    #[inline]
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

        let opacity = (layer.opacity.clamp(0.0, 1.0) * 255.0) as u8;
        // color is [R, G, B, A] from scene; convert to ARGB for blend
        let src_argb = [color[3], color[0], color[1], color[2]];
        let eff_a = ((src_argb[0] as u16 * opacity as u16) >> 8) as u8;

        if eff_a == 0 {
            return;
        }

        let stride = self.width as usize * 4;

        if eff_a >= 254 {
            // Fast path: fully opaque fill — direct memcpy, no blend needed
            let pixel = [255u8, color[0], color[1], color[2]];
            for row in 0..clip.h {
                let row_start = (clip.y0 + row) as usize * stride + clip.x0 as usize * 4;
                let row_end = row_start + clip.w as usize * 4;
                for chunk in buffer[row_start..row_end].chunks_exact_mut(4) {
                    chunk.copy_from_slice(&pixel);
                }
            }
        } else {
            // Partial opacity: per-pixel blend via ranga
            for row in 0..clip.h {
                let row_start = (clip.y0 + row) as usize * stride + clip.x0 as usize * 4;
                let row_end = row_start + clip.w as usize * 4;
                for chunk in buffer[row_start..row_end].chunks_exact_mut(4) {
                    let result = ranga::blend::blend_pixel_argb(
                        src_argb,
                        [chunk[0], chunk[1], chunk[2], chunk[3]],
                        ranga::blend::BlendMode::Normal,
                        opacity,
                    );
                    chunk.copy_from_slice(&result);
                }
            }
        }
    }

    #[inline]
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

        let opacity_fp = (layer.opacity.clamp(0.0, 1.0) * 256.0) as u16;
        let stride = self.width as usize * 4;
        let needs_scale = lw != fw || lh != fh;

        // If scaling is needed, pre-resize via ranga (bilinear) and blend the result.
        // ranga::transform::resize requires Rgba8, so we convert ARGB→RGBA, resize, then RGBA→ARGB.
        let scaled_data;
        let (src_data, src_w, src_h) = if needs_scale {
            let argb_buf = ranga::pixel::PixelBuffer::new(
                frame.data.clone(),
                fw,
                fh,
                ranga::pixel::PixelFormat::Argb8,
            );
            let Ok(argb_buf) = argb_buf else { return };
            let Ok(rgba_buf) = ranga::convert::argb8_to_rgba8(&argb_buf) else {
                return;
            };
            let Ok(resized) =
                ranga::transform::resize(&rgba_buf, lw, lh, ranga::transform::ScaleFilter::Bicubic)
            else {
                return;
            };
            let Ok(back) = ranga::convert::rgba8_to_argb8(&resized) else {
                return;
            };
            scaled_data = back.data;
            (&scaled_data[..], lw, lh)
        } else {
            (&frame.data[..], fw, fh)
        };

        // 1:1 blend (original or pre-resized)
        for row in 0..clip.h {
            let src_y = (clip.y0 + row) as i32 - clip.layer_y;
            if src_y < 0 || src_y as u32 >= src_h {
                continue;
            }
            let src_x0 = clip.x0 as i32 - clip.layer_x;
            let src_row_start = (src_y as u32 * src_w + src_x0 as u32) as usize * 4;
            let src_row_end = src_row_start + clip.w as usize * 4;
            if src_row_end > src_data.len() {
                continue;
            }
            let src_row = &src_data[src_row_start..src_row_end];
            let dst_row_start = (clip.y0 + row) as usize * stride + clip.x0 as usize * 4;
            let dst_row_end = dst_row_start + clip.w as usize * 4;
            let dst_row = &mut buffer[dst_row_start..dst_row_end];

            if opacity_fp >= 256 {
                let all_opaque = src_row.chunks_exact(4).all(|px| px[0] == 255);
                if all_opaque {
                    dst_row.copy_from_slice(src_row);
                } else {
                    blend_row_alpha(dst_row, src_row, 256);
                }
            } else {
                blend_row_alpha(dst_row, src_row, opacity_fp);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Row-level alpha blending — delegated to ranga
// ---------------------------------------------------------------------------

/// Blend a source row onto a destination row with per-pixel alpha and opacity.
///
/// `opacity_fp` is fixed-point Q8 (256 = fully opaque layer).
/// Delegates to ranga's ARGB blend which includes SIMD acceleration.
#[inline]
fn blend_row_alpha(dst: &mut [u8], src: &[u8], opacity_fp: u16) {
    // ranga expects 0..=255; 256 means fully opaque so pass 255
    let opacity = opacity_fp.min(255) as u8;
    ranga::blend::blend_row_normal_argb(src, dst, opacity);
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
        let mut comp = Compositor::new(4, 4);
        let scene = SceneGraph::new(4, 4, 30);
        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(frame.is_valid());
        assert!(frame.data.iter().all(|&b| b == 0));
    }

    #[test]
    fn color_fill_opaque() {
        let mut comp = Compositor::new(2, 2);
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
        let mut comp = Compositor::new(1, 1);
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
        let mut comp = Compositor::new(2, 2);
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
        let mut comp = Compositor::new(4, 4);
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
        let mut comp = Compositor::new(2, 2);
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
        let mut comp = Compositor::new(1, 1);
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
        let mut comp = Compositor::new(4, 4);
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
        // pixel (2,0) should be transparent (outside layer bounds)
        let idx = 2 * 4; // column 2, row 0
        assert_eq!(frame.data[idx], 0);
    }

    #[test]
    fn ranga_blend_row_produces_result() {
        // Verify ranga's ARGB blend produces reasonable output
        let src: Vec<u8> = (0..40)
            .map(|i| if i % 4 == 0 { 200 } else { (i * 17) as u8 })
            .collect();
        let mut dst: Vec<u8> = (0..40).map(|i| (i * 7 + 50) as u8).collect();
        blend_row_alpha(&mut dst, &src, 200);
        // Should have modified dst (source has alpha=200)
        assert_ne!(&dst[1..4], &[57, 64, 71]); // original values changed
    }

    #[test]
    fn source_layer_with_partial_opacity() {
        // This exercises the SIMD blend_row_alpha path
        let mut comp = Compositor::new(8, 8);
        let mut scene = SceneGraph::new(8, 8, 30);
        let mut layer = Layer::new(
            "src",
            LayerContent::Source {
                source_id: uuid::Uuid::nil(),
            },
        );
        layer.opacity = 0.8;
        let lid = layer.id;
        scene.add_layer(layer);

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

    #[test]
    fn scaled_layer_composited() {
        let mut comp = Compositor::new(8, 8);
        let mut scene = SceneGraph::new(8, 8, 30);
        let mut layer = Layer::new(
            "scaled",
            LayerContent::Source {
                source_id: uuid::Uuid::nil(),
            },
        );
        layer.size = Some((4, 4));
        let lid = layer.id;
        scene.add_layer(layer);

        // 8x8 opaque white source frame
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
        assert_eq!(result.width, 8);
        assert_eq!(result.height, 8);
        // The 4x4 region should have non-zero pixels from the bicubic-resized source
        let idx_inside = (8 + 1) * 4; // pixel (1,1) inside 4x4 region
        assert!(
            result.data[idx_inside] > 0,
            "scaled region should have content"
        );
        // Pixel outside the 4x4 layer should be untouched (transparent black)
        let idx_outside = (5 * 8 + 5) * 4; // pixel (5,5) outside layer
        assert_eq!(
            result.data[idx_outside], 0,
            "outside layer should be transparent"
        );
    }

    #[test]
    fn multiple_layers_stacked() {
        let mut comp = Compositor::new(8, 8);
        let mut scene = SceneGraph::new(8, 8, 30);

        // Bottom: full-canvas blue (z=0)
        let mut blue = Layer::new(
            "blue",
            LayerContent::ColorFill {
                color: [0, 0, 255, 255],
            },
        );
        blue.z_index = 0;
        scene.add_layer(blue);

        // Middle: 6x6 green centered at (1,1) (z=1)
        let mut green = Layer::new(
            "green",
            LayerContent::ColorFill {
                color: [0, 255, 0, 255],
            },
        );
        green.z_index = 1;
        green.size = Some((6, 6));
        green.position = (1, 1);
        scene.add_layer(green);

        // Top: 2x2 red centered at (3,3) (z=2)
        let mut red = Layer::new(
            "red",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        );
        red.z_index = 2;
        red.size = Some((2, 2));
        red.position = (3, 3);
        scene.add_layer(red);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(frame.is_valid());

        // Corner (0,0) should be blue: [A=255, R=0, G=0, B=255]
        assert_eq!(&frame.data[0..4], [255, 0, 0, 255]);
        // Center (3,3) should be red: [A=255, R=255, G=0, B=0]
        let center = (3 * 8 + 3) * 4;
        assert_eq!(&frame.data[center..center + 4], [255, 255, 0, 0]);
        // (2,2) should be green: [A=255, R=0, G=255, B=0]
        let mid = (2 * 8 + 2) * 4;
        assert_eq!(&frame.data[mid..mid + 4], [255, 0, 255, 0]);
    }

    #[test]
    fn fully_transparent_layer_noop() {
        let mut comp = Compositor::new(4, 4);
        let mut scene = SceneGraph::new(4, 4, 30);
        let mut layer = Layer::new(
            "ghost",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        );
        layer.opacity = 0.0;
        scene.add_layer(layer);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(
            frame.data.iter().all(|&b| b == 0),
            "transparent layer should not affect output"
        );
    }

    #[test]
    fn layer_outside_bounds() {
        let mut comp = Compositor::new(4, 4);
        let mut scene = SceneGraph::new(4, 4, 30);
        let mut layer = Layer::new(
            "offscreen",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        );
        layer.size = Some((2, 2));
        layer.position = (1000, 1000);
        scene.add_layer(layer);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(
            frame.data.iter().all(|&b| b == 0),
            "off-canvas layer should produce no change"
        );
    }

    #[test]
    fn single_pixel_frame() {
        let mut comp = Compositor::new(1, 1);
        let mut scene = SceneGraph::new(1, 1, 30);
        let layer = Layer::new(
            "px",
            LayerContent::Source {
                source_id: uuid::Uuid::nil(),
            },
        );
        let lid = layer.id;
        scene.add_layer(layer);

        // Single pixel: ARGB = [255, 128, 64, 32]
        let src_frame = RawFrame {
            data: vec![255, 128, 64, 32],
            format: PixelFormat::Argb8888,
            width: 1,
            height: 1,
            pts_us: 0,
        };
        let mut frames = HashMap::new();
        frames.insert(lid, src_frame);

        let result = comp.compose(&scene, &frames, 0);
        assert!(result.is_valid());
        assert_eq!(result.width, 1);
        assert_eq!(result.height, 1);
        assert_eq!(&result.data[..], &[255, 128, 64, 32]);
    }

    #[test]
    fn large_canvas_color_fill() {
        let mut comp = Compositor::new(1920, 1080);
        let mut scene = SceneGraph::new(1920, 1080, 30);
        scene.add_layer(Layer::new(
            "fill",
            LayerContent::ColorFill {
                color: [0, 128, 255, 255],
            },
        ));

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(frame.is_valid());
        assert_eq!(frame.width, 1920);
        assert_eq!(frame.height, 1080);

        // First pixel: [A=255, R=0, G=128, B=255]
        assert_eq!(&frame.data[0..4], [255, 0, 128, 255]);
        // Last pixel
        let last = frame.data.len() - 4;
        assert_eq!(&frame.data[last..last + 4], [255, 0, 128, 255]);
    }

    // --- Additional coverage tests ---

    #[test]
    fn clip_rect_fully_negative_returns_none() {
        // Layer entirely to the left/above the canvas
        assert!(ClipRect::compute(-10, -10, 5, 5, 100, 100).is_none());
    }

    #[test]
    fn clip_rect_zero_size_returns_none() {
        assert!(ClipRect::compute(0, 0, 0, 0, 100, 100).is_none());
        assert!(ClipRect::compute(0, 0, 0, 10, 100, 100).is_none());
        assert!(ClipRect::compute(0, 0, 10, 0, 100, 100).is_none());
    }

    #[test]
    fn clip_rect_very_large_dimensions() {
        // Layer extends far beyond canvas — should clamp to canvas size
        let clip = ClipRect::compute(0, 0, u32::MAX, u32::MAX, 100, 100).unwrap();
        assert_eq!(clip.x0, 0);
        assert_eq!(clip.y0, 0);
        assert_eq!(clip.w, 100);
        assert_eq!(clip.h, 100);
    }

    #[test]
    fn clip_rect_large_negative_position_with_large_size() {
        // Position is very negative but size is huge — only the overlap shows
        let clip =
            ClipRect::compute(-1_000_000, -1_000_000, 1_000_010, 1_000_010, 100, 100).unwrap();
        assert_eq!(clip.x0, 0);
        assert_eq!(clip.y0, 0);
        assert_eq!(clip.w, 10);
        assert_eq!(clip.h, 10);
    }

    #[test]
    fn clip_rect_exactly_at_boundary() {
        // Layer starts exactly at right/bottom edge — zero visible area
        assert!(ClipRect::compute(100, 0, 10, 10, 100, 100).is_none());
        assert!(ClipRect::compute(0, 100, 10, 10, 100, 100).is_none());
        // Layer ends exactly at right/bottom edge — fully visible
        let clip = ClipRect::compute(90, 90, 10, 10, 100, 100).unwrap();
        assert_eq!(clip.w, 10);
        assert_eq!(clip.h, 10);
    }

    #[test]
    fn clip_rect_i32_overflow_protection() {
        // Position + size would overflow i32 — i64 arithmetic should handle it
        let clip = ClipRect::compute(i32::MAX - 5, 0, 100, 10, 200, 200);
        // x0 = i32::MAX - 5 which is > 200, so should be None
        assert!(clip.is_none());
    }

    #[test]
    fn color_fill_opacity_near_one() {
        // Opacity very close to 1.0 — tests the eff_a >= 254 fast path boundary
        let mut comp = Compositor::new(2, 2);
        let mut scene = SceneGraph::new(2, 2, 30);
        let mut layer = Layer::new(
            "almost_opaque",
            LayerContent::ColorFill {
                color: [0, 255, 0, 255],
            },
        );
        // 254/255 ~= 0.996 -> eff_a should be ~254, hitting the fast path
        layer.opacity = 254.0 / 255.0;
        scene.add_layer(layer);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        // Should be essentially fully opaque green in ARGB: [A, R, G, B]
        for chunk in frame.data.chunks_exact(4) {
            assert!(chunk[0] >= 253, "A should be ~255, got {}", chunk[0]);
            assert_eq!(chunk[1], 0, "R should be 0");
            assert!(chunk[2] >= 253, "G should be ~255, got {}", chunk[2]);
            assert_eq!(chunk[3], 0, "B should be 0");
        }
    }

    #[test]
    fn color_fill_very_low_opacity() {
        // Opacity just above 0 — should produce faint blend (slow path)
        let mut comp = Compositor::new(2, 2);
        let mut scene = SceneGraph::new(2, 2, 30);
        let mut layer = Layer::new(
            "faint",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        );
        layer.opacity = 0.02;
        scene.add_layer(layer);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        // Should be very dim red — non-zero but small values
        for chunk in frame.data.chunks_exact(4) {
            assert!(chunk[0] < 20, "A should be small, got {}", chunk[0]);
        }
    }

    #[test]
    fn color_fill_zero_alpha_in_color() {
        // Color itself has alpha=0 — effective alpha should be 0, early return
        let mut comp = Compositor::new(2, 2);
        let mut scene = SceneGraph::new(2, 2, 30);
        scene.add_layer(Layer::new(
            "noalpha",
            LayerContent::ColorFill {
                color: [255, 0, 0, 0],
            },
        ));

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(
            frame.data.iter().all(|&b| b == 0),
            "zero-alpha color should produce no change"
        );
    }

    #[test]
    fn color_fill_with_explicit_size() {
        // Explicitly sized color fill smaller than canvas
        let mut comp = Compositor::new(4, 4);
        let mut scene = SceneGraph::new(4, 4, 30);
        let mut layer = Layer::new(
            "sized",
            LayerContent::ColorFill {
                color: [255, 255, 255, 255],
            },
        );
        layer.size = Some((2, 2));
        layer.position = (0, 0);
        scene.add_layer(layer);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        // (0,0) should be white ARGB
        assert_eq!(&frame.data[0..4], [255, 255, 255, 255]);
        // (3,3) should be transparent
        let idx = (3 * 4 + 3) * 4;
        assert_eq!(&frame.data[idx..idx + 4], [0, 0, 0, 0]);
    }

    #[test]
    fn frame_with_partial_alpha_pixels() {
        // Source frame with mixed alpha — exercises the non-all-opaque path at full opacity
        let mut comp = Compositor::new(2, 1);
        let mut scene = SceneGraph::new(2, 1, 30);
        let layer = Layer::new(
            "mixed",
            LayerContent::Source {
                source_id: uuid::Uuid::nil(),
            },
        );
        let lid = layer.id;
        scene.add_layer(layer);

        // pixel 0: fully opaque white, pixel 1: half-alpha red
        let src_frame = RawFrame {
            data: vec![
                255, 255, 255, 255, // ARGB: opaque white
                128, 200, 0, 0, // ARGB: half-alpha red
            ],
            format: PixelFormat::Argb8888,
            width: 2,
            height: 1,
            pts_us: 0,
        };
        let mut frames = HashMap::new();
        frames.insert(lid, src_frame);

        let result = comp.compose(&scene, &frames, 0);
        assert!(result.is_valid());
        // Pixel 0 should be opaque white (all-opaque path hits copy)
        // But row has mixed alpha so blend_row_alpha is called for the whole row
        assert!(result.data[0] > 0, "should have alpha");
    }

    #[test]
    fn frame_layer_no_matching_frame_is_noop() {
        // Source layer with no frame in the map — should not crash, buffer stays zero
        let mut comp = Compositor::new(4, 4);
        let mut scene = SceneGraph::new(4, 4, 30);
        scene.add_layer(Layer::new(
            "missing",
            LayerContent::Source {
                source_id: uuid::Uuid::nil(),
            },
        ));

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(frame.data.iter().all(|&b| b == 0));
    }

    #[test]
    fn scaled_upscale_layer() {
        // Source is 2x2, layer size is 8x8 — upscale path
        let mut comp = Compositor::new(8, 8);
        let mut scene = SceneGraph::new(8, 8, 30);
        let mut layer = Layer::new(
            "upscaled",
            LayerContent::Source {
                source_id: uuid::Uuid::nil(),
            },
        );
        layer.size = Some((8, 8));
        let lid = layer.id;
        scene.add_layer(layer);

        // Tiny 2x2 opaque red source
        let src_frame = RawFrame {
            data: vec![
                255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0,
            ],
            format: PixelFormat::Argb8888,
            width: 2,
            height: 2,
            pts_us: 0,
        };
        let mut frames = HashMap::new();
        frames.insert(lid, src_frame);

        let result = comp.compose(&scene, &frames, 0);
        assert!(result.is_valid());
        // The 8x8 output should be filled with reddish pixels from bicubic upscale
        let center = (4 * 8 + 4) * 4;
        assert!(
            result.data[center] > 200,
            "center pixel should be mostly opaque after upscale"
        );
    }

    #[test]
    fn frame_layer_partial_opacity_with_blend() {
        // Frame layer at 50% opacity — exercises the opacity_fp < 256 branch
        let mut comp = Compositor::new(2, 2);
        let mut scene = SceneGraph::new(2, 2, 30);
        let mut layer = Layer::new(
            "half_src",
            LayerContent::Source {
                source_id: uuid::Uuid::nil(),
            },
        );
        layer.opacity = 0.5;
        let lid = layer.id;
        scene.add_layer(layer);

        let src_frame = RawFrame {
            data: vec![
                255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
            ],
            format: PixelFormat::Argb8888,
            width: 2,
            height: 2,
            pts_us: 0,
        };
        let mut frames = HashMap::new();
        frames.insert(lid, src_frame);

        let result = comp.compose(&scene, &frames, 0);
        assert!(result.is_valid());
        // 50% white over black should give ~128 per channel
        for chunk in result.data.chunks_exact(4) {
            assert!(
                chunk[1] > 100 && chunk[1] < 180,
                "R should be ~128, got {}",
                chunk[1]
            );
        }
    }

    #[test]
    fn frame_layer_negative_position_clips_source() {
        // Frame placed at negative coords — tests src_x0/src_y0 offset calculations
        let mut comp = Compositor::new(4, 4);
        let mut scene = SceneGraph::new(4, 4, 30);
        let mut layer = Layer::new(
            "neg_frame",
            LayerContent::Source {
                source_id: uuid::Uuid::nil(),
            },
        );
        layer.position = (-1, -1);
        let lid = layer.id;
        scene.add_layer(layer);

        // 4x4 frame, each pixel ARGB opaque with different R values by row
        let mut data = Vec::with_capacity(4 * 4 * 4);
        for row in 0..4u8 {
            for _col in 0..4u8 {
                data.extend_from_slice(&[255, row * 50, 0, 0]);
            }
        }
        let src_frame = RawFrame {
            data,
            format: PixelFormat::Argb8888,
            width: 4,
            height: 4,
            pts_us: 0,
        };
        let mut frames = HashMap::new();
        frames.insert(lid, src_frame);

        let result = comp.compose(&scene, &frames, 0);
        assert!(result.is_valid());
        // (0,0) in output corresponds to (1,1) in source
        // Source row 1 has R = 50
        assert_eq!(result.data[0], 255); // alpha
        assert_eq!(result.data[1], 50); // R from source row 1
    }

    #[test]
    fn image_and_text_layers_need_frames() {
        // Image and Text content types also go through blend_frame
        let mut comp = Compositor::new(2, 2);
        let mut scene = SceneGraph::new(2, 2, 30);

        let img_layer = Layer::new(
            "img",
            LayerContent::Image {
                path: "test.png".into(),
            },
        );
        let img_id = img_layer.id;
        scene.add_layer(img_layer);

        let txt_layer = Layer::new(
            "txt",
            LayerContent::Text {
                text: "hi".into(),
                font_size: 12.0,
                color: [255, 255, 255, 255],
            },
        );
        let txt_id = txt_layer.id;
        scene.add_layer(txt_layer);

        // Provide frames for both
        let white_frame = RawFrame {
            data: vec![255; 2 * 2 * 4],
            format: PixelFormat::Argb8888,
            width: 2,
            height: 2,
            pts_us: 0,
        };
        let mut frames = HashMap::new();
        frames.insert(img_id, white_frame.clone());
        frames.insert(txt_id, white_frame);

        let result = comp.compose(&scene, &frames, 0);
        assert!(result.is_valid());
        // Should be all-white (two opaque white layers stacked)
        assert!(result.data.iter().all(|&b| b == 255));
    }

    #[test]
    fn blend_row_alpha_full_opacity() {
        // opacity_fp = 256 gets clamped to 255 in blend_row_alpha
        let src = [255u8, 100, 200, 50, 255, 100, 200, 50];
        let mut dst = [0u8; 8];
        blend_row_alpha(&mut dst, &src, 256);
        // Source is fully opaque (alpha=255), so dst should match src
        assert_eq!(&dst, &src);
    }

    #[test]
    fn overlapping_semi_transparent() {
        let mut comp = Compositor::new(1, 1);
        let mut scene = SceneGraph::new(1, 1, 30);

        // Bottom: 50% red
        let mut red = Layer::new(
            "red",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        );
        red.opacity = 0.5;
        red.z_index = 0;
        scene.add_layer(red);

        // Top: 50% blue
        let mut blue = Layer::new(
            "blue",
            LayerContent::ColorFill {
                color: [0, 0, 255, 255],
            },
        );
        blue.opacity = 0.5;
        blue.z_index = 1;
        scene.add_layer(blue);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(frame.is_valid());

        let a = frame.data[0];
        let r = frame.data[1];
        let g = frame.data[2];
        let b = frame.data[3];

        // Should have some red and some blue, green should be near zero
        assert!(r > 30, "expected red component, got R={}", r);
        assert!(b > 30, "expected blue component, got B={}", b);
        assert!(g < 10, "green should be near zero, got G={}", g);
        // Alpha should be non-trivial from blending
        assert!(a > 80, "expected meaningful alpha, got A={}", a);
    }
}
