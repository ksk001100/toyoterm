use super::*;

pub(super) struct GpuImage {
    pipeline: RenderPipeline,
    bindings: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

impl GpuImage {
    pub(super) fn new(
        device: &Device,
        queue: &Queue,
        format: wgpu::TextureFormat,
        image: &toyoterm_terminal::TerminalImage,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("terminal image"),
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
        queue.write_texture(
            texture.as_image_copy(),
            &image.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 4),
                rows_per_image: Some(image.height),
            },
            texture.size(),
        );
        let view = texture.create_view(&TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terminal image sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terminal image parameters"),
            size: 32,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("terminal image shader"),
            source: ShaderSource::Wgsl(include_str!("graphics.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("terminal image pipeline"),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terminal image bindings"),
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
        Self {
            pipeline,
            bindings,
            uniform,
        }
    }

    pub(super) fn update(
        &self,
        queue: &Queue,
        image: &toyoterm_terminal::TerminalImage,
        pane: PaneRect,
        layout: TextLayout,
        surface: &SurfaceConfiguration,
    ) {
        let left =
            pane.x as f32 + layout.horizontal_padding + f32::from(image.column) * layout.cell_width;
        let top = pane.y as f32 + layout.vertical_padding + image.row as f32 * layout.line_height;
        let width = image.display_width as f32;
        let height = image.display_height as f32;
        let data = [
            left / surface.width as f32 * 2.0 - 1.0,
            1.0 - top / surface.height as f32 * 2.0,
            width / surface.width as f32 * 2.0,
            -height / surface.height as f32 * 2.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&data));
    }
    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bindings, &[]);
        pass.draw(0..6, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a working GPU or software adapter"]
    fn gpu_terminal_image_blends_and_clips_to_pane() {
        pollster::block_on(async {
            let instance = Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance.request_adapter(&Default::default()).await.unwrap();
            let (device, queue) = adapter
                .request_device(&DeviceDescriptor::default())
                .await
                .unwrap();
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let image = toyoterm_terminal::TerminalImage {
                id: 1,
                width: 1,
                height: 1,
                rgba: Arc::from([255, 0, 0, 128]),
                column: 0,
                row: -1,
                columns: 4,
                rows: 4,
                display_width: 4,
                display_height: 4,
            };
            let gpu = GpuImage::new(&device, &queue, format, &image);
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("terminal image test"),
                size: wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let surface = SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: 4,
                height: 4,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 1,
                alpha_mode: CompositeAlphaMode::Opaque,
                color_space: Default::default(),
                view_formats: vec![],
            };
            let pane = PaneRect::new(1, 1, 2, 2);
            let layout = TextLayout {
                font_size: 1.0,
                line_height: 1.0,
                cell_width: 1.0,
                horizontal_padding: 0.0,
                vertical_padding: 0.0,
            };
            gpu.update(&queue, &image, pane, layout, &surface);
            let view = target.create_view(&Default::default());
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("terminal image readback"),
                size: 1024,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let attachments = [Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color {
                            r: 0.0,
                            g: 0.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    color_attachments: &attachments,
                    ..Default::default()
                });
                pass.set_scissor_rect(1, 1, 2, 2);
                gpu.draw(&mut pass);
            }
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(4),
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
                for y in 0..4 {
                    for x in 0..4 {
                        let expected = if (1..3).contains(&x) && (1..3).contains(&y) {
                            [128u8, 0, 127, 255]
                        } else {
                            [0, 0, 255, 255]
                        };
                        for (actual, expected) in bytes[y * 256 + x * 4..y * 256 + x * 4 + 4]
                            .iter()
                            .zip(expected)
                        {
                            assert!(
                                actual.abs_diff(expected) <= 1,
                                "pixel {x},{y}: {actual} != {expected}"
                            );
                        }
                    }
                }
            }
            readback.unmap();
        });
    }
}
