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
mod tests {
    use super::*;

    #[test]
    fn idle_png_decodes_to_44x44() {
        let (w, h, rgba) = decode_png(IDLE_PNG);
        assert_eq!(w, 44);
        assert_eq!(h, 44);
        assert!(!rgba.is_empty());
    }

    #[test]
    fn error_png_decodes_to_44x44() {
        let (w, h, rgba) = decode_png(ERROR_PNG);
        assert_eq!(w, 44);
        assert_eq!(h, 44);
        assert!(!rgba.is_empty());
    }

    #[test]
    fn all_states_produce_valid_icons() {
        // Verifies the full load_icon path doesn't panic
        for state in [TrayState::Idle, TrayState::Error] {
            let _icon = load_icon(state);
        }
    }

    #[test]
    fn idle_png_is_grey() {
        let (_, _, rgba) = decode_png(IDLE_PNG);
        // Grey icon: R ~= G ~= B for non-transparent pixels
        for chunk in rgba.chunks_exact(4) {
            if chunk[3] > 128 {
                let max = chunk[0].max(chunk[1]).max(chunk[2]);
                let min = chunk[0].min(chunk[1]).min(chunk[2]);
                assert!(
                    max - min < 30,
                    "idle icon should be grey (equal RGB), got r={} g={} b={}",
                    chunk[0], chunk[1], chunk[2]
                );
                return;
            }
        }
        panic!("no opaque pixels found");
    }

    #[test]
    fn error_png_is_grey() {
        let (_, _, rgba) = decode_png(ERROR_PNG);
        // Grey icon (same as idle for now)
        for chunk in rgba.chunks_exact(4) {
            if chunk[3] > 128 {
                let max = chunk[0].max(chunk[1]).max(chunk[2]);
                let min = chunk[0].min(chunk[1]).min(chunk[2]);
                assert!(
                    max - min < 30,
                    "error icon should be grey, got r={} g={} b={}",
                    chunk[0], chunk[1], chunk[2]
                );
                return;
            }
        }
        panic!("no opaque pixels found");
    }
}
