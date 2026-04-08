use std::io::Cursor;

use tray_icon::Icon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    /// No VMs running (grey)
    Idle,
    /// VMs running (purple)
    Active,
    /// Gateway unreachable (red)
    Error,
}

// @2x Retina variants for macOS menu bar (44x44)
static IDLE_PNG: &[u8] = include_bytes!("../icons/tray-idle@2x.png");
static ACTIVE_PNG: &[u8] = include_bytes!("../icons/tray-active@2x.png");
static ERROR_PNG: &[u8] = include_bytes!("../icons/tray-error@2x.png");

pub fn load_icon(state: TrayState) -> Icon {
    let png_data = match state {
        TrayState::Idle => IDLE_PNG,
        TrayState::Active => ACTIVE_PNG,
        TrayState::Error => ERROR_PNG,
    };

    let decoder = png::Decoder::new(Cursor::new(png_data));
    let mut reader = decoder.read_info().expect("invalid PNG");
    let mut buf = vec![0u8; reader.output_buffer_size().expect("no frame info")];
    let info = reader.next_frame(&mut buf).expect("failed to decode PNG");
    buf.truncate(info.buffer_size());

    // png crate decodes to RGB or RGBA; we need RGBA
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(buf.len() / 3 * 4);
            for chunk in buf.chunks_exact(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        other => panic!("unexpected PNG color type: {other:?}"),
    };

    Icon::from_rgba(rgba, info.width, info.height).expect("failed to create icon")
}
