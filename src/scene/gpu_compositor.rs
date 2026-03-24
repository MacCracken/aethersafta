//! GPU-accelerated compositor via soorat.
//!
//! Replaces the CPU alpha-blending path with wgpu sprite rendering.
//! Each visible layer becomes a textured quad drawn by soorat's
//! [`SpritePipeline`], with GPU-native scaling, positioning, and opacity.
//!
//! Requires the `gpu` feature.

use std::collections::HashMap;

use soorat::render_target::RenderTarget;
use soorat::{Color, GpuContext, Sprite, SpriteBatch, SpritePipeline, Texture, UvRect};

use crate::scene::{LayerContent, LayerId, SceneGraph};
use crate::source::{PixelFormat, RawFrame};

/// GPU-accelerated compositor backed by soorat's wgpu rendering pipeline.
///
/// Uploads layer frames as GPU textures and composites them in a single
/// render pass with hardware-accelerated alpha blending and scaling.
pub struct GpuCompositor {
    gpu: GpuContext,
    pipeline: SpritePipeline,
    render_target: RenderTarget,
    width: u32,
    height: u32,
    /// Cached layer textures — reused across frames when source unchanged.
    texture_cache: HashMap<LayerId, CachedTexture>,
}

struct CachedTexture {
    /// Kept alive so the bind group's texture reference remains valid.
    #[allow(dead_code)]
    texture: Texture,
    bind_group: wgpu::BindGroup,
    /// Hash of the frame data for cache invalidation.
    data_hash: u64,
    width: u32,
    height: u32,
}

/// Describes a layer to be rendered in the GPU pass.
struct LayerWork {
    layer_id: LayerId,
    sprite: Sprite,
    /// RGBA pixel data for upload (converted from ARGB).
    rgba: Vec<u8>,
    src_width: u32,
    src_height: u32,
    data_hash: u64,
}

impl GpuCompositor {
    /// Create a new GPU compositor with the given output dimensions.
    ///
    /// Initializes a headless wgpu context (no window needed).
    ///
    /// # Errors
    ///
    /// Returns an error if GPU initialization fails.
    pub fn new(width: u32, height: u32) -> Result<Self, soorat::RenderError> {
        let gpu = pollster::block_on(GpuContext::new())?;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;

        let pipeline = SpritePipeline::new(&gpu.device, format)?;
        pipeline.update_projection(&gpu.queue, width as f32, height as f32);

        let render_target = RenderTarget::new(&gpu.device, width, height, format);

        Ok(Self {
            gpu,
            pipeline,
            render_target,
            width,
            height,
            texture_cache: HashMap::new(),
        })
    }

    /// Create a GPU compositor using an existing [`GpuContext`].
    pub fn with_context(
        gpu: GpuContext,
        width: u32,
        height: u32,
    ) -> Result<Self, soorat::RenderError> {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let pipeline = SpritePipeline::new(&gpu.device, format)?;
        pipeline.update_projection(&gpu.queue, width as f32, height as f32);
        let render_target = RenderTarget::new(&gpu.device, width, height, format);

        Ok(Self {
            gpu,
            pipeline,
            render_target,
            width,
            height,
            texture_cache: HashMap::new(),
        })
    }

    /// Resize the output. Recreates the render target.
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.pipeline
            .update_projection(&self.gpu.queue, width as f32, height as f32);
        self.render_target = RenderTarget::new(
            &self.gpu.device,
            width,
            height,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
    }

    /// Compose scene layers into a single ARGB8888 [`RawFrame`] on the GPU.
    ///
    /// Each visible layer with a corresponding frame in `frames` is uploaded
    /// as a GPU texture and rendered as a sprite with position, size, and opacity.
    /// The result is read back from the GPU as ARGB8888 pixels.
    pub fn compose(
        &mut self,
        scene: &SceneGraph,
        frames: &HashMap<LayerId, RawFrame>,
        pts_us: u64,
    ) -> RawFrame {
        // Phase 1: collect layer work items (no &mut self needed)
        let work = self.collect_layer_work(scene, frames);

        if work.is_empty() {
            let buf_size = RawFrame::expected_size(self.width, self.height);
            return RawFrame {
                data: vec![0u8; buf_size].into(),
                format: PixelFormat::Argb8888,
                width: self.width,
                height: self.height,
                pts_us,
            };
        }

        // Phase 2: prune stale cache entries
        let active_ids: std::collections::HashSet<LayerId> =
            scene.layers().iter().map(|l| l.id).collect();
        self.texture_cache.retain(|id, _| active_ids.contains(id));

        // Phase 3: upload textures and render
        let layout = self.pipeline.texture_bind_group_layout();

        for item in &work {
            let needs_update = match self.texture_cache.get(&item.layer_id) {
                Some(cached) => {
                    cached.data_hash != item.data_hash
                        || cached.width != item.src_width
                        || cached.height != item.src_height
                }
                None => true,
            };

            if needs_update {
                let texture = Texture::from_rgba(
                    &self.gpu.device,
                    &self.gpu.queue,
                    &item.rgba,
                    item.src_width,
                    item.src_height,
                    "layer_texture",
                )
                .expect("texture upload failed");
                let bind_group = texture.bind_group(&self.gpu.device, layout);
                self.texture_cache.insert(
                    item.layer_id,
                    CachedTexture {
                        texture,
                        bind_group,
                        data_hash: item.data_hash,
                        width: item.src_width,
                        height: item.src_height,
                    },
                );
            }
        }

        // Phase 4: draw each layer
        let clear = Some(Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        });

        for (i, item) in work.iter().enumerate() {
            let cached = self.texture_cache.get(&item.layer_id).unwrap();
            let layer_batch = SpriteBatch {
                sprites: vec![item.sprite.clone()],
            };
            self.pipeline.draw(
                &self.gpu.device,
                &self.gpu.queue,
                &self.render_target.view,
                &layer_batch,
                &cached.bind_group,
                if i == 0 { clear } else { None },
            );
        }

        // Phase 5: read back pixels as RGBA, convert to ARGB
        let rgba = self
            .render_target
            .read_pixels(&self.gpu.device, &self.gpu.queue)
            .unwrap_or_else(|_| vec![0u8; (self.width * self.height * 4) as usize]);

        let argb = rgba_to_argb(&rgba);

        RawFrame {
            data: argb.into(),
            format: PixelFormat::Argb8888,
            width: self.width,
            height: self.height,
            pts_us,
        }
    }

    /// Collect work items from the scene without mutating self.
    fn collect_layer_work(
        &self,
        scene: &SceneGraph,
        frames: &HashMap<LayerId, RawFrame>,
    ) -> Vec<LayerWork> {
        let mut work = Vec::new();

        for layer in scene.layers().iter().filter(|l| l.visible) {
            match &layer.content {
                LayerContent::ColorFill { color } => {
                    let rgba = vec![color[0], color[1], color[2], color[3]];
                    let hash = simple_hash(&rgba);
                    let (lw, lh) = layer.size.unwrap_or((self.width, self.height));

                    work.push(LayerWork {
                        layer_id: layer.id,
                        sprite: Sprite {
                            x: layer.position.0 as f32,
                            y: layer.position.1 as f32,
                            width: lw as f32,
                            height: lh as f32,
                            rotation: 0.0,
                            color: Color {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: layer.opacity,
                            },
                            texture_id: 0,
                            z_order: layer.z_index,
                            uv: UvRect::FULL,
                        },
                        rgba,
                        src_width: 1,
                        src_height: 1,
                        data_hash: hash,
                    });
                }
                LayerContent::Source { .. }
                | LayerContent::Image { .. }
                | LayerContent::Text { .. } => {
                    let frame = match frames.get(&layer.id) {
                        Some(f) if f.format == PixelFormat::Argb8888 && f.is_valid() => f,
                        _ => continue,
                    };

                    let rgba = argb_to_rgba(&frame.data);
                    let hash = simple_hash(&frame.data);
                    let (lw, lh) = layer.size.unwrap_or((frame.width, frame.height));

                    work.push(LayerWork {
                        layer_id: layer.id,
                        sprite: Sprite {
                            x: layer.position.0 as f32,
                            y: layer.position.1 as f32,
                            width: lw as f32,
                            height: lh as f32,
                            rotation: 0.0,
                            color: Color {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: layer.opacity,
                            },
                            texture_id: 0,
                            z_order: layer.z_index,
                            uv: UvRect::FULL,
                        },
                        rgba,
                        src_width: frame.width,
                        src_height: frame.height,
                        data_hash: hash,
                    });
                }
            }
        }

        work
    }

    /// Access the underlying GPU context.
    #[must_use]
    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    /// Output width.
    #[must_use]
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Output height.
    #[must_use]
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }
}

/// Convert ARGB8888 pixel data to RGBA8888.
#[inline]
fn argb_to_rgba(argb: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(argb.len());
    for px in argb.chunks_exact(4) {
        rgba.extend_from_slice(&[px[1], px[2], px[3], px[0]]);
    }
    rgba
}

/// Convert RGBA8888 pixel data to ARGB8888.
#[inline]
fn rgba_to_argb(rgba: &[u8]) -> Vec<u8> {
    let mut argb = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        argb.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
    }
    argb
}

/// Simple FNV-1a hash for cache invalidation.
#[inline]
fn simple_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_rgba_roundtrip() {
        let argb = vec![255, 128, 64, 32, 0, 10, 20, 30];
        let rgba = argb_to_rgba(&argb);
        assert_eq!(rgba, vec![128, 64, 32, 255, 10, 20, 30, 0]);
        let back = rgba_to_argb(&rgba);
        assert_eq!(back, argb);
    }

    #[test]
    fn simple_hash_deterministic() {
        let data = b"hello world";
        let h1 = simple_hash(data);
        let h2 = simple_hash(data);
        assert_eq!(h1, h2);
        assert_ne!(h1, simple_hash(b"different"));
    }

    #[test]
    fn argb_to_rgba_empty() {
        assert!(argb_to_rgba(&[]).is_empty());
    }

    #[test]
    fn rgba_to_argb_empty() {
        assert!(rgba_to_argb(&[]).is_empty());
    }
}
