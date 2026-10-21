//! The animated obsidian + nether portal background, drawn by a wgpu shader.
//!
//! The widget redraws itself about 30 times a second without rebuilding the rest of the UI,
//! and stops animating while a game is running so the game gets the GPU. With the CPU
//! renderer (low-memory machines) shaders aren't available, and a static gradient is used.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::wgpu;
use iced::widget::shader::{self, Viewport};
use iced::{Rectangle, mouse, window};

use crate::textures::GameTextures;

const FRAME: Duration = Duration::from_millis(33);

/// The shader widget's program.
pub struct Portal {
    pub textures: Option<Arc<GameTextures>>,
    pub animate: bool,
    /// Size of one obsidian block, logical px.
    pub block: f32,
}

/// When the animation started, so time keeps flowing across redraws.
pub struct Clock(Instant);

impl Default for Clock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl<Message> shader::Program<Message> for Portal {
    type State = Clock;
    type Primitive = Primitive;

    fn update(
        &self,
        _state: &mut Clock,
        event: &iced::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        match event {
            // Schedule from the real clock: the event's own timestamp can already be stale,
            // which would make every frame "due now" and spin the UI thread.
            iced::Event::Window(window::Event::RedrawRequested(_)) if self.animate => {
                Some(shader::Action::request_redraw_at(Instant::now() + FRAME))
            }
            _ => None,
        }
    }

    fn draw(&self, state: &Clock, _cursor: mouse::Cursor, _bounds: Rectangle) -> Primitive {
        Primitive {
            time: state.0.elapsed().as_secs_f32(),
            block: self.block,
            textures: self.textures.clone(),
        }
    }
}

#[derive(Debug)]
pub struct Primitive {
    time: f32,
    block: f32,
    textures: Option<Arc<GameTextures>>,
}

pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    /// Which textures are on the GPU (by address), so they're uploaded once.
    uploaded: usize,
    portal_frames: f32,
    has_obsidian: bool,
    srgb_target: bool,
}

fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("netherNode background texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Plain (non-sRGB) format: the shader works in sRGB values throughout.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        size,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

impl Pipeline {
    fn bind(
        &self,
        device: &wgpu::Device,
        obsidian: &wgpu::TextureView,
        portal: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("netherNode background"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(obsidian),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(portal),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: Option<&Arc<GameTextures>>,
    ) {
        let id = textures.map_or(0, |t| Arc::as_ptr(t) as usize);
        if id == self.uploaded {
            return;
        }
        self.uploaded = id;
        let blank = texture(device, queue, 1, 1, &[0, 0, 0, 255]);
        let obsidian = textures.and_then(|t| t.obsidian.as_ref());
        let portal = textures.and_then(|t| t.portal.as_ref());
        let obsidian_view = obsidian.map(|i| texture(device, queue, i.width, i.height, &i.rgba));
        let portal_view = portal.map(|i| texture(device, queue, i.width, i.height, &i.rgba));
        self.has_obsidian = obsidian.is_some();
        // The portal texture is a vertical strip of square frames.
        self.portal_frames = portal.map_or(0.0, |i| (i.height / i.width.max(1)) as f32);
        self.bind_group = self.bind(
            device,
            obsidian_view.as_ref().unwrap_or(&blank),
            portal_view.as_ref().unwrap_or(&blank),
        );
    }
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("netherNode background"),
            source: wgpu::ShaderSource::Wgsl(include_str!("background.wgsl").into()),
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("netherNode background"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("netherNode background"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("netherNode background"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("netherNode background uniforms"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Nearest filtering keeps the game's pixel art crisp at any size.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let blank = texture(device, queue, 1, 1, &[0, 0, 0, 255]);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("netherNode background"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&blank),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&blank),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Self {
            pipeline,
            layout,
            uniforms,
            sampler,
            bind_group,
            uploaded: 0,
            portal_frames: 0.0,
            has_obsidian: false,
            srgb_target: format.is_srgb(),
        }
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        pipeline.upload(device, queue, self.textures.as_ref());
        let scale = viewport.scale_factor();
        let uniforms: [f32; 12] = [
            bounds.x * scale,
            bounds.y * scale,
            bounds.width * scale,
            bounds.height * scale,
            self.time,
            (self.block * scale).round(),
            pipeline.portal_frames,
            if pipeline.has_obsidian { 1.0 } else { 0.0 },
            if pipeline.srgb_target { 1.0 } else { 0.0 },
            0.0,
            0.0,
            0.0,
        ];
        let bytes: Vec<u8> = uniforms.iter().flat_map(|f| f.to_ne_bytes()).collect();
        queue.write_buffer(&pipeline.uniforms, 0, &bytes);
    }

    fn draw(&self, pipeline: &Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &pipeline.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        true
    }
}
