//! Real GPU mipmaps: egui-wgpu's managed image textures have only one level.
use eframe::{egui, egui_wgpu::RenderState, wgpu};
use std::sync::{Arc, Mutex};
type Retired = Arc<Mutex<Vec<(egui::TextureId, wgpu::Texture)>>>;

pub struct Mipmapper {
    state: RenderState,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    retired: Retired,
}
pub struct Texture {
    pub id: egui::TextureId,
    texture: wgpu::Texture,
    retired: Retired,
}
impl Drop for Texture {
    fn drop(&mut self) {
        // This frame's meshes may still reference the image. Match egui's
        // managed-handle semantics by releasing it on the following frame.
        self.retired
            .lock()
            .unwrap()
            .push((self.id, self.texture.clone()));
    }
}
impl Mipmapper {
    pub fn new(state: RenderState) -> Self {
        let shader = state
            .device
            .create_shader_module(wgpu::include_wgsl!("mipmaps.wgsl"));
        let pipeline = state
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Photo mipmap pyramid"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let sampler = state.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Photo mipmap reduction"),
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            state,
            pipeline,
            sampler,
            retired: Arc::new(Mutex::new(Vec::new())),
        }
    }
    pub fn collect(&self) {
        let retired = std::mem::take(&mut *self.retired.lock().unwrap());
        if !retired.is_empty() {
            let mut renderer = self.state.renderer.write();
            for (id, texture) in retired {
                renderer.free_texture(&id);
                texture.destroy();
            }
        }
    }
    pub fn upload(&self, image: &egui::ColorImage) -> Texture {
        let size = wgpu::Extent3d {
            width: image.size[0] as u32,
            height: image.size[1] as u32,
            depth_or_array_layers: 1,
        };
        let levels = size.max_mips(wgpu::TextureDimension::D2);
        let texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Photo with mipmaps"),
            size,
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        // Borrow the decoded pixel bytes; do not make another image-sized CPU copy.
        self.state.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.width * 4),
                rows_per_image: Some(size.height),
            },
            size,
        );
        let views = (0..levels)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect::<Vec<_>>();
        let mut encoder =
            self.state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Photo mipmap generation"),
                });
        for level in 1..levels {
            let bindings = self
                .state
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Photo mipmap source"),
                    layout: &self.pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                &views[(level - 1) as usize],
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Photo mipmap level"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &views[level as usize],
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.draw(0..3, 0..1);
        }
        self.state.queue.submit([encoder.finish()]);
        let id = self
            .state
            .renderer
            .write()
            .register_native_texture_with_sampler_options(
                &self.state.device,
                &texture.create_view(&Default::default()),
                wgpu::SamplerDescriptor {
                    label: Some("Photo trilinear sampling"),
                    min_filter: wgpu::FilterMode::Linear,
                    mag_filter: wgpu::FilterMode::Linear,
                    mipmap_filter: wgpu::MipmapFilterMode::Linear,
                    ..Default::default()
                },
            );
        Texture {
            id,
            texture,
            retired: self.retired.clone(),
        }
    }
    #[cfg(feature = "ui-smoke")]
    pub fn verify_low_pass(&self) -> color_eyre::eyre::Result<()> {
        use color_eyre::eyre::{bail, eyre};
        let mut image = egui::ColorImage::filled([8, 8], egui::Color32::BLACK);
        for y in 0..8 {
            for x in 0..8 {
                if (x + y) % 2 == 1 {
                    image.pixels[y * 8 + x] = egui::Color32::WHITE;
                }
            }
        }
        let texture = self.upload(&image);
        let buffer = self.state.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mipmap verification readback"),
            size: 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .state
            .device
            .create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: 3,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.state.queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        let timeout = std::time::Duration::from_secs(10);
        self.state
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(timeout),
            })
            .map_err(|e| eyre!("GPU readback poll: {e}"))?;
        receiver
            .recv_timeout(timeout)?
            .map_err(|e| eyre!("GPU readback map: {e}"))?;
        let bytes = buffer.slice(..).get_mapped_range()?;
        if !bytes[..3].iter().all(|v| (126..=129).contains(v)) || bytes[3] != 255 {
            bail!(
                "Mipmap reduction must average a pixel checkerboard to gray: {:?}",
                &bytes[..4]
            );
        }
        drop(bytes);
        buffer.unmap();
        drop(texture);
        self.collect();
        Ok(())
    }
}
