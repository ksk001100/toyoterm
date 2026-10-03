use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<toyoterm_api::ImagePixels>,
}

pub(super) struct GpuBackground {
    pipeline: RenderPipeline,
    bindings: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

/// Centered cover: crop the longer axis without distorting the image.
fn cover_scale(image: [u32; 2], surface: [u32; 2]) -> [f32; 2] {
    let ratio = (surface[0] as f32 / surface[1].max(1) as f32)
        / (image[0].max(1) as f32 / image[1].max(1) as f32);
    [ratio.min(1.0), (1.0 / ratio).min(1.0)]
}

impl GpuBackground {
    pub(super) fn new(
        device: &Device,
        queue: &Queue,
        format: wgpu::TextureFormat,
        image: &BackgroundImage,
    ) -> Result<Self, RenderError> {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("background image"),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let row_bytes = image.width as usize * 4;
        let chunk_rows = (256 * 1024 / row_bytes).max(1);
        // Decode into a single mapped transfer buffer. Repeated write_texture
        // calls allocate separate staging resources on DX12; a full rgba()
        // expansion would instead allocate another image-sized CPU buffer.
        let padded_row_bytes = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let upload = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("background image upload"),
            size: (padded_row_bytes * image.height as usize) as u64,
            usage: BufferUsages::MAP_WRITE | BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        let mut mapped = upload
            .slice(..)
            .get_mapped_range_mut()
            .map_err(|error| RenderError::new("map background upload", error))?;
        image
            .pixels
            .for_each_rgba_chunk(chunk_rows * row_bytes, |offset, rgba| {
                let first_row = offset / row_bytes;
                for (index, row) in rgba.chunks_exact(row_bytes).enumerate() {
                    let start = (first_row + index) * padded_row_bytes;
                    mapped.slice(start..start + row_bytes).copy_from_slice(row);
                }
            })
            .map_err(|error| RenderError::new("expand background pixels", error))?;
        drop(mapped);
        upload.unmap();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("background image upload"),
        });
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &upload,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row_bytes as u32),
                    rows_per_image: None,
                },
            },
            texture.as_image_copy(),
            texture.size(),
        );
        queue.submit([encoder.finish()]);
        // Submission retains the transfer buffer until the GPU copy completes.
        // No CPU wait is needed before using the texture on this same queue.
        tracing::debug!(target: "toyoterm::render", stored_bytes = image.pixels.stored_len(), chunk_bytes = chunk_rows * row_bytes, "background pixels streamed for GPU upload");
        let view = texture.create_view(&TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("background image sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("background image parameters"),
            size: 32,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("background image shader"),
            source: ShaderSource::Wgsl(include_str!("background.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("background image pipeline"),
            layout: None,
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("background image bindings"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        Ok(Self {
            pipeline,
            bindings,
            uniform,
        })
    }

    pub(super) fn update(
        &self,
        queue: &Queue,
        style: &RenderStyle,
        surface: &SurfaceConfiguration,
    ) {
        let image = style
            .background_image
            .as_ref()
            .expect("background image exists");
        let scale = cover_scale([image.width, image.height], [surface.width, surface.height]);
        let premultiplied = surface.alpha_mode == CompositeAlphaMode::PreMultiplied;
        let data = [
            srgb_channel_to_linear(style.background[0]),
            srgb_channel_to_linear(style.background[1]),
            srgb_channel_to_linear(style.background[2]),
            style.opacity,
            scale[0],
            scale[1],
            style.background_image_opacity,
            if premultiplied { 1.0 } else { 0.0 },
        ];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&data));
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bindings, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crops_only_the_longer_axis() {
        assert_eq!(cover_scale([200, 100], [100, 100]), [0.5, 1.0]);
        assert_eq!(cover_scale([100, 200], [100, 100]), [1.0, 0.5]);
        assert_eq!(cover_scale([200, 100], [400, 200]), [1.0, 1.0]);
    }

    #[test]
    #[ignore = "requires a working GPU or software adapter"]
    fn gpu_background_composites_image_alpha_and_window_opacity() {
        check_background_composition(1);
        // Non-aligned rows and a short final upload exercise multiple chunks.
        check_background_composition(257);
    }

    fn check_background_composition(side: u32) {
        pollster::block_on(async {
            let instance = Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance.request_adapter(&Default::default()).await.unwrap();
            let (device, queue) = adapter
                .request_device(&DeviceDescriptor::default())
                .await
                .unwrap();
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let image = BackgroundImage {
                width: side,
                height: side,
                pixels: Arc::new(toyoterm_api::ImagePixels::new(
                    [255, 0, 0, 128].repeat((side * side) as usize),
                )),
            };
            if side > 1 {
                assert!(image.pixels.stored_len() < (side * side * 4) as usize);
            }
            let gpu = GpuBackground::new(&device, &queue, format, &image).unwrap();
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("background test target"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: side,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("background test readback"),
                size: 256 * u64::from(side),
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            for (mode, opacity, strength, expected) in [
                (
                    CompositeAlphaMode::PreMultiplied,
                    0.5,
                    1.0,
                    [64, 0, 64, 128],
                ),
                (
                    CompositeAlphaMode::PostMultiplied,
                    0.5,
                    1.0,
                    [128, 0, 127, 128],
                ),
                (
                    CompositeAlphaMode::PreMultiplied,
                    1.0,
                    0.0,
                    [0, 0, 255, 255],
                ),
                (CompositeAlphaMode::PreMultiplied, 0.0, 1.0, [0, 0, 0, 0]),
                (CompositeAlphaMode::Opaque, 1.0, 0.5, [64, 0, 191, 255]),
            ] {
                let style = RenderStyle {
                    background_image: Some(image.clone()),
                    background: [0, 0, 255],
                    opacity,
                    background_image_opacity: strength,
                    ..Default::default()
                };
                let surface = SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format,
                    width: 1,
                    height: 1,
                    present_mode: wgpu::PresentMode::Fifo,
                    desired_maximum_frame_latency: 1,
                    alpha_mode: mode,
                    color_space: Default::default(),
                    view_formats: vec![],
                };
                gpu.update(&queue, &style, &surface);
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let attachments = [Some(RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(Color::BLACK),
                            store: StoreOp::Store,
                        },
                    })];
                    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        color_attachments: &attachments,
                        ..Default::default()
                    });
                    gpu.draw(&mut pass);
                }
                encoder.copy_texture_to_buffer(
                    target.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(side),
                        },
                    },
                    target.size(),
                );
                queue.submit([encoder.finish()]);
                let (sender, receiver) = std::sync::mpsc::channel();
                readback
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        sender.send(result).unwrap()
                    });
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                receiver.recv().unwrap().unwrap();
                {
                    let bytes = readback.slice(..).get_mapped_range().unwrap();
                    for row in bytes.as_chunks::<256>().0 {
                        for (actual, expected) in row[..4].iter().zip(expected) {
                            assert!(
                                i32::from(*actual).abs_diff(expected) <= 1,
                                "{mode:?}: {:?}",
                                &row[..4]
                            );
                        }
                    }
                }
                readback.unmap();
            }
        });
    }
}
