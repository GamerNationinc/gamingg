//! Flat pictures on the screen: the engine's first 2D surface.
//!
//! Deliberately generic — the overlay takes any RGBA image and a pixel
//! rectangle and draws it over the frame, no depth test, alpha blended. The
//! minimap is its first user; a crosshair, the terminal screen and the pocket
//! arcade are the queue behind it, which is why nothing in here knows what a
//! map is.
//!
//! # Off means byte-identical
//!
//! An overlay is drawn only when one has been set this session. Headless
//! captures and the culling tests never set one, so every pixel-equality
//! guarantee the renderer makes stands exactly as before this module existed.

use bytemuck::{Pod, Zeroable};

use crate::DEPTH_FORMAT;

/// Where an overlay sits on screen, in pixels from the top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct RectUniform {
    rect: [f32; 4],
    screen: [f32; 2],
    _pad: [f32; 2],
}

/// How many independent overlay pictures a frame can carry. Slot 0 is the
/// minimap and slot 1 the HUD by convention in the app; the pass itself does
/// not care.
///
/// Raising this is free and safe: an unset slot is a `None` the draw loop
/// skips entirely, so the "nothing set means byte-identical frames" guarantee
/// the culling tests rest on is per-slot and cannot be disturbed by the array
/// getting longer.
pub const OVERLAY_SLOTS: usize = 19;

/// One picture on screen: its texture, its rectangle uniform, and the bind
/// group tying them together.
struct Picture {
    _texture: wgpu::Texture,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// The overlay pipeline plus whatever pictures are currently set.
pub struct OverlayPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    slots: [Option<Picture>; OVERLAY_SLOTS],
}

impl OverlayPass {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("overlay.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_overlay"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            // The pass carries a depth attachment, so the pipeline must too —
            // but the overlay neither tests nor writes it: it is on top of
            // everything by construction.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_overlay"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("overlay sampler"),
            // Nearest: the minimap is one pixel per column and pixel text
            // should stay crisp when scaled, exactly like the block textures.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        OverlayPass {
            pipeline,
            layout,
            sampler,
            slots: Default::default(),
        }
    }

    /// Put `pixels` (tightly packed RGBA, `width * height * 4` bytes) on
    /// screen at `rect` in slot `slot`, on a target `screen` pixels big.
    /// Reuses the slot's texture while the size is unchanged.
    ///
    /// Eight arguments and every one earns its seat: this is the single
    /// plumbing point between CPU images and the screen.
    #[allow(clippy::too_many_arguments)]
    pub fn set_picture(
        &mut self,
        slot: usize,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: (u32, u32),
        pixels: &[u8],
        rect: OverlayRect,
        screen: (u32, u32),
    ) {
        let (width, height) = size;
        assert_eq!(
            pixels.len(),
            (width * height * 4) as usize,
            "overlay pixel buffer does not match its declared size"
        );

        let recreate = self.slots[slot]
            .as_ref()
            .is_none_or(|picture| picture.width != width || picture.height != height);
        if recreate {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("overlay picture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("overlay rect"),
                size: std::mem::size_of::<RectUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("overlay bind group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.slots[slot] = Some(Picture {
                _texture: texture,
                uniform,
                bind_group,
                width,
                height,
            });
        }

        let picture = self.slots[slot].as_ref().expect("just ensured");
        queue.write_buffer(
            &picture.uniform,
            0,
            bytemuck::bytes_of(&RectUniform {
                rect: [rect.x, rect.y, rect.width, rect.height],
                screen: [screen.0 as f32, screen.1 as f32],
                _pad: [0.0; 2],
            }),
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &picture._texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Stop drawing the picture in `slot`.
    pub fn clear(&mut self, slot: usize) {
        self.slots[slot] = None;
    }

    pub fn is_set(&self, slot: usize) -> bool {
        self.slots[slot].is_some()
    }

    /// Record the draws, lowest slot first. Call last in the pass.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let mut bound = false;
        for picture in self.slots.iter().flatten() {
            if !bound {
                pass.set_pipeline(&self.pipeline);
                bound = true;
            }
            pass.set_bind_group(0, &picture.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}
