use std::io::Cursor;

use tray_icon::Icon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    /// Normal state (grey template -- OS adapts to light/dark)
    Idle,
    /// Gateway unreachable (grey, same as idle)
    Error,
}

// @2x Retina variants for macOS menu bar (44x44)
static IDLE_PNG: &[u8] = include_bytes!("../icons/tray-idle@2x.png");
static ERROR_PNG: &[u8] = include_bytes!("../icons/tray-error@2x.png");

pub fn load_icon(state: TrayState) -> Icon {
    let png_data = match state {
        TrayState::Idle => IDLE_PNG,
        TrayState::Error => ERROR_PNG,
    };

    let decoder = png::Decoder::new(Cursor::new(png_data));
    let mut reader = decoder.read_info().expect("invalid PNG");
    let mut buf = vec![0u8; reader.output_buffer_size().expect("no frame info")];
    let info = reader.next_frame(&mut buf).expect("failed to decode PNG");
    buf.truncate(info.buffer_size());

    // png crate decodes to RGB or RGBA; we need RGBA
    let mut rgba = match info.color_type {
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

    if cfg!(debug_assertions) {
        tint_to_orange(&mut rgba);
    }

    Icon::from_rgba(rgba, info.width, info.height).expect("failed to create icon")
}

/// Recolor opaque pixels in-place to a bright orange so dev builds are
/// visually distinct from the installed (grey) tray. The icon is grey
/// (R == G == B) on disk; we remap the luminance to an orange ramp so
/// anti-aliased edges stay smooth instead of banding.
fn tint_to_orange(rgba: &mut [u8]) {
    // Saturated orange target: #FF8800. The luminance of the source pixel
    // scales each channel, preserving anti-aliasing.
    for px in rgba.chunks_exact_mut(4) {
        let alpha = px[3];
        if alpha == 0 {
            continue;
        }
        // Source is grey, so any channel equals the luminance.
        let lum = px[0] as u16;
        px[0] = ((lum * 255) / 255) as u8; // R
        px[1] = ((lum * 136) / 255) as u8; // G
        px[2] = 0; // B
    }
}

/// Decode a PNG to (width, height, rgba_bytes) without creating an Icon.
/// Used by tests to verify embedded PNGs.
#[cfg(test)]
fn decode_png(data: &[u8]) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(Cursor::new(data));
    let mut reader = decoder.read_info().expect("invalid PNG");
    let mut buf = vec![0u8; reader.output_buffer_size().expect("no frame info")];
    let info = reader.next_frame(&mut buf).expect("failed to decode PNG");
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

#[cfg(test)]
mod tests;
