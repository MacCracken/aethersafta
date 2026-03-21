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

        let opacity = (layer.opacity * 255.0).min(255.0) as u8;
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
                ranga::transform::resize(&rgba_buf, lw, lh, ranga::transform::ScaleFilter::Bilinear)
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

            if opacity_fp >= 255 {
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
fn blend_row_alpha(dst: &mut [u8], src: &[u8], opacity_fp: u16) {
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
        let comp = Compositor::new(8, 8);
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
}
