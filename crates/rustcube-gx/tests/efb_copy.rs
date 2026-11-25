//! Render to texture: GXCopyTex (BP 0x49, 0x4A, 0x4B, 0x52) copies the red
//! EFB, and a later quad samples the copy's address: red, not the RAM there
//! (black) and not a missing texture (white).
//! Skips without a wgpu adapter.
use rustcube_gx::api::{tev_mode, Gx, Ram, TexObj};
use rustcube_gx::enums::*;
use rustcube_gx::gpu::GxRenderer;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits:
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .ok()
}

fn bp(gx: &mut Gx, reg: u32, v: u32, ram: &[u8]) {
    let w = (reg << 24) | (v & 0x00FF_FFFF);
    let mut dl = vec![command::LOAD_BP_REG as u8];
    dl.extend_from_slice(&w.to_be_bytes());
    gx.run_display_list(&dl, &Ram(ram));
}

fn quad(gx: &mut Gx, ram: &[u8]) {
    let textured = true;
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    if textured {
        gx.set_vtx_desc(attr::TEX0, attr_type::DIRECT);
        gx.set_vtx_attr_fmt(0, attr::TEX0, comp_cnt::TEX_ST, comp_type::F32, 0);
    } else {
        gx.set_vtx_desc(attr::CLR0, attr_type::DIRECT);
        gx.set_vtx_attr_fmt(0, attr::CLR0, comp_cnt::CLR_RGBA, comp_type::RGBA8, 0);
    }
    gx.begin(command::DRAW_QUADS, 0, 4);
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        gx.position3f32(x, y, -1.5);
        if textured {
            gx.tex_coord2f32(0.5, 0.5);
        } else {
            gx.color4u8(255, 0, 0, 255);
        }
    }
    gx.end(&Ram(ram));
}

#[test]
fn efb_copy_becomes_the_texture_at_its_address() {
    let Some((device, queue)) = device() else {
        eprintln!("no wgpu adapter; skipping");
        return;
    };
    let ram = vec![0u8; 0x4000]; // the copy's RAM bytes stay zero (black)
    let mut gx = Gx::new();
    gx.set_copy_clear([255, 0, 0, 255], 0x00FF_FFFF);
    gx.set_cull_mode(cull_mode::NONE);
    gx.set_projection(
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0, -2.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        1,
    );
    // Pass 1: the EFB cleared to red, its 16x16 corner copied to 0x2000.
    bp(&mut gx, 0x49, 0, &ram); // source top-left 0,0
    bp(&mut gx, 0x4A, 15 | (15 << 10), &ram); // 16x16
    bp(&mut gx, 0x4B, 0x2000 >> 5, &ram);
    bp(&mut gx, 0x52, 0x800, &ram); // to texture, clear after
                                    // Pass 2: full-screen quad sampling the texture at 0x2000.
    gx.set_viewport(0.0, 0.0, 640.0, 480.0, 0.0, 1.0);
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
            0x2000,
            16,
            16,
            tex_format::RGBA8,
            wrap_mode::CLAMP,
            wrap_mode::CLAMP,
            false,
        ),
        0,
    );
    quad(&mut gx, &ram);
    let frame = gx.take_frame();
    assert_eq!(frame.tex_copies.len(), 1);
    assert_eq!(frame.tex_copies[0].src, [0, 0, 16, 16]);

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut r = GxRenderer::new(device.clone(), queue.clone(), format);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    // The second time comes from what the first prepared (as pictures
    // between game frames do), the copy into the texture made before.
    for n in 1..=2 {
        r.render_frame(&frame, &target.create_view(&Default::default()), [64, 64]);
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
        let at = |x: usize, y: usize| px[y * 256 + x * 4..y * 256 + x * 4 + 3].to_vec();
        assert_eq!(
            at(32, 32),
            vec![255, 0, 0],
            "the screen quad shows the EFB copy (drawn {n} times)"
        );
    }
}
