//! Draw a recorded GX frame with wgpu and read the pixels back. Skips when
//! the machine has no adapter (headless CI without a software Vulkan).
use rustcube_gx::api::{tev_mode, Gx, Ram, TexCopy, TexObj};
use rustcube_gx::enums::*;
use rustcube_gx::gpu::GxRenderer;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }))
    .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits:
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .ok()
}

#[test]
fn red_textured_quad_over_blue_clear() {
    let Some((device, queue)) = device() else {
        eprintln!("no wgpu adapter; skipping");
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut r = GxRenderer::new(device.clone(), queue.clone(), format);

    let mut ram = vec![0u8; 0x2000];
    for i in 0..16 {
        ram[0x1000 + 2 * i..0x1002 + 2 * i].copy_from_slice(&0xF800u16.to_be_bytes());
    }
    let mut gx = Gx::new();
    gx.set_copy_clear([0, 0, 255, 255], 0x00FF_FFFF);
    gx.set_cull_mode(cull_mode::NONE);
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_desc(attr::TEX0, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    gx.set_vtx_attr_fmt(0, attr::TEX0, comp_cnt::TEX_ST, comp_type::F32, 0);
    gx.set_num_chans(0);
    gx.set_tev_op(0, tev_mode::REPLACE);
    gx.set_tev_order(
        0,
        tex_coord_id::TEXCOORD0,
        tex_map_id::TEXMAP0,
        color_channel_id::COLOR_NULL,
    );
    gx.load_tex_obj(
        &TexObj::new(
            0x8000_1000,
            4,
            4,
            tex_format::RGB565,
            wrap_mode::REPEAT,
            wrap_mode::REPEAT,
            false,
        ),
        0,
    );
    // Orthographic projection over -1..1 with z from -1 (near) to -2 (far).
    gx.set_projection(
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0, -2.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        1,
    );
    gx.set_viewport(0.0, 0.0, 640.0, 480.0, 0.0, 1.0);
    gx.begin(command::DRAW_QUADS, 0, 4);
    for (x, y) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
        gx.position3f32(x, y, -1.5);
        gx.tex_coord2f32(0.5, 0.5);
    }
    gx.end(&Ram(&ram));
    let frame = gx.take_frame();

    let size = [64u32, 64];
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    // The same draws again after an EFB copy at draw 0: that copy's empty
    // clear-only pass starts where the picture's pass does, and once stood in
    // for it (the picture came out as the clear colour alone).
    let mut early = frame.clone();
    early.tex_copies.push(TexCopy {
        draw: 0,
        src: [0, 0, 8, 8],
        id: 900,
        clear: true,
    });
    early.serial += 1;
    for (n, f) in [&frame, &early].into_iter().enumerate() {
        r.render_frame(f, &target.create_view(&Default::default()), size);

        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 * 64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let px = buf.slice(..).get_mapped_range().unwrap().to_vec();
        let at = |x: usize, y: usize| &px[y * 256 + x * 4..y * 256 + x * 4 + 4];
        assert_eq!(
            at(32, 32),
            &[255, 0, 0, 255],
            "center is the red texture (frame {n})"
        );
        assert_eq!(at(2, 2), &[0, 0, 255, 255], "corner is the clear color");
    }
}
