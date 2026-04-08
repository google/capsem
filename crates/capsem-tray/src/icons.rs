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
    fn active_png_decodes_to_44x44() {
        let (w, h, rgba) = decode_png(ACTIVE_PNG);
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
        for state in [TrayState::Idle, TrayState::Active, TrayState::Error] {
            let _icon = load_icon(state);
        }
    }

    #[test]
    fn idle_png_is_black_template() {
        let (_, _, rgba) = decode_png(IDLE_PNG);
        // Template icon: black pixels with alpha (macOS adapts to light/dark)
        for chunk in rgba.chunks_exact(4) {
            if chunk[3] > 0 {
                assert!(
                    chunk[0] < 10 && chunk[1] < 10 && chunk[2] < 10,
                    "template icon should be black, got r={} g={} b={}",
                    chunk[0], chunk[1], chunk[2]
                );
                return;
            }
        }
        panic!("no non-transparent pixels found");
    }

    #[test]
    fn active_png_has_purple_pixels() {
        let (_, _, rgba) = decode_png(ACTIVE_PNG);
        // Purple: high red, low green, high blue
        for chunk in rgba.chunks_exact(4) {
            if chunk[3] > 200 {
                assert!(
                    chunk[0] > chunk[1] && chunk[2] > chunk[1],
                    "expected purple (R>G, B>G), got r={} g={} b={}",
                    chunk[0], chunk[1], chunk[2]
                );
                return;
            }
        }
        panic!("no opaque pixels found");
    }

    #[test]
    fn error_png_has_red_pixels() {
        let (_, _, rgba) = decode_png(ERROR_PNG);
        // Red: high red, lower green and blue
        for chunk in rgba.chunks_exact(4) {
            if chunk[3] > 200 {
                assert!(
                    chunk[0] > chunk[1] && chunk[0] > chunk[2],
                    "expected red-dominant, got r={} g={} b={}",
                    chunk[0], chunk[1], chunk[2]
                );
                return;
            }
        }
        panic!("no opaque pixels found");
    }
}
