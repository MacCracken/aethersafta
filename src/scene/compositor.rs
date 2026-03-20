//! Compositor: alpha-blends scene graph layers into a single output frame.
//!
//! Iterates visible layers bottom-to-top (by z-index), blending each
//! onto the output buffer using per-pixel alpha and per-layer opacity.

use std::collections::HashMap;

use crate::scene::{LayerContent, SceneGraph};
use crate::source::RawFrame;

use super::LayerId;

/// Composites a scene graph into a single frame.
pub struct Compositor {
    width: u32,
    height: u32,
}

/// Pre-computed clipped rectangle in output coordinates.
struct ClipRect {
    /// First visible column in output.
    x0: u32,
    /// First visible row in output.
    y0: u32,
    /// Number of visible columns.
    w: u32,
    /// Number of visible rows.
    h: u32,
    /// Layer x offset (may be negative — x0 accounts for clipping).
    layer_x: i32,
    /// Layer y offset.
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
    /// Create a compositor for the given output resolution.
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Composite all visible layers into a single ARGB8888 frame.
    ///
    /// `frames` maps layer IDs to their current frame data. Layers with
    /// `LayerContent::ColorFill` don't need an entry. Layers whose ID
    /// is missing from the map are skipped.
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

        // color is RGBA: [R, G, B, A]
        // Pre-compute opacity as fixed-point: opacity_fp = round(opacity * 256)
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
            // Fast path: fully opaque — write ARGB directly per row
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

        // Fast path: 1:1 scale, full opacity → direct row copy
        if !needs_scale && opacity_fp >= 255 {
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

                // Check if all source pixels are fully opaque (A=255) in ARGB
                // For fully opaque rows, memcpy is fastest
                let all_opaque = src_row.chunks_exact(4).all(|px| px[0] == 255);
                if all_opaque {
                    buffer[dst_row_start..dst_row_start + src_row.len()].copy_from_slice(src_row);
                } else {
                    let dst_row = &mut buffer[dst_row_start..dst_row_start + src_row.len()];
                    for (dst, src) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(4)) {
                        alpha_blend_pixel(dst, src[0], src[1], src[2], src[3]);
                    }
                }
            }
            return;
        }

        // General path: scaling and/or partial opacity
        for row in 0..clip.h {
            let out_y = clip.y0 + row;
            let local_y = out_y as i32 - clip.layer_y;

            let src_y = if needs_scale {
                (local_y as u64 * fh as u64 / lh as u64) as u32
            } else {
                local_y as u32
            };
            if src_y >= fh {
                continue;
            }

            let dst_row_start = out_y as usize * stride + clip.x0 as usize * 4;

            for col in 0..clip.w {
                let out_x = clip.x0 + col;
                let local_x = out_x as i32 - clip.layer_x;

                let src_x = if needs_scale {
                    (local_x as u64 * fw as u64 / lw as u64) as u32
                } else {
                    local_x as u32
                };
                if src_x >= fw {
                    continue;
                }

                let src_idx = (src_y * fw + src_x) as usize * 4;
                if src_idx + 3 >= frame.data.len() {
                    continue;
                }

                // ARGB: [A, R, G, B] — apply opacity via fixed-point
                let raw_a = frame.data[src_idx] as u16;
                let src_a = ((raw_a * opacity_fp) >> 8) as u8;
                let src_r = frame.data[src_idx + 1];
                let src_g = frame.data[src_idx + 2];
                let src_b = frame.data[src_idx + 3];

                let dst_idx = dst_row_start + col as usize * 4;
                alpha_blend_pixel(
                    &mut buffer[dst_idx..dst_idx + 4],
                    src_a,
                    src_r,
                    src_g,
                    src_b,
                );
            }
        }
    }
}

/// Alpha-blend a single ARGB pixel onto a 4-byte destination slice.
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
                color: [255, 0, 0, 255], // RGBA: red, fully opaque
            },
        ));
        let frame = comp.compose(&scene, &HashMap::new(), 0);
        assert!(frame.is_valid());
        // Every pixel should be ARGB [255, 255, 0, 0]
        for chunk in frame.data.chunks_exact(4) {
            assert_eq!(chunk, [255, 255, 0, 0]); // A=255, R=255, G=0, B=0
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
        // Alpha should be ~127
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

        // White opaque frame: ARGB [255, 255, 255, 255]
        let src_frame = RawFrame {
            data: vec![255u8; 2 * 2 * 4],
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
        // Top-left 2x2 should be transparent, bottom-right 2x2 should be white
        // pixel (0,0) = transparent
        assert_eq!(frame.data[0], 0);
        // pixel (2,2) = offset into buffer: (2*4+2)*4 = 40
        let idx = (2 * 4 + 2) * 4;
        assert_eq!(frame.data[idx], 255); // A
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

        // Bottom layer: blue
        let mut blue = Layer::new(
            "blue",
            LayerContent::ColorFill {
                color: [0, 0, 255, 255],
            },
        );
        blue.z_index = 0;
        scene.add_layer(blue);

        // Top layer: red (fully opaque, should overwrite)
        let mut red = Layer::new(
            "red",
            LayerContent::ColorFill {
                color: [255, 0, 0, 255],
            },
        );
        red.z_index = 10;
        scene.add_layer(red);

        let frame = comp.compose(&scene, &HashMap::new(), 0);
        // Should be red: ARGB [255, 255, 0, 0]
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
        // pixel (0,0) should be green (clipped portion is visible)
        assert_eq!(&frame.data[0..4], [255, 0, 255, 0]);
        // pixel (2,0) should be transparent (outside layer bounds)
        let idx = (0 * 4 + 2) * 4;
        assert_eq!(frame.data[idx], 0);
    }
}
