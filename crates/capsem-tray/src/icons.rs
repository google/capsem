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
        px[2] = 0;                          // B
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

    #[test]
    fn tint_to_orange_preserves_alpha_and_remaps_channels() {
        // Two opaque grey pixels (RGBA), one transparent, one semi-transparent.
        let mut rgba = vec![
            128, 128, 128, 255,
            64, 64, 64, 128,
            0, 0, 0, 0,        // fully transparent -- must be untouched
            200, 200, 200, 200,
        ];
        tint_to_orange(&mut rgba);
        // First pixel: lum=128, R=128, G=(128*136/255)=68, B=0, alpha=255
        assert_eq!(&rgba[0..4], &[128, 68, 0, 255]);
        // Second pixel: lum=64, R=64, G=(64*136/255)=34, B=0, alpha=128
        assert_eq!(&rgba[4..8], &[64, 34, 0, 128]);
        // Transparent pixel untouched
        assert_eq!(&rgba[8..12], &[0, 0, 0, 0]);
        // Semi-transparent: lum=200, R=200, G=(200*136/255)=106, B=0
        assert_eq!(&rgba[12..16], &[200, 106, 0, 200]);
    }

    #[test]
    fn debug_build_tray_icon_is_orange() {
        // Debug builds (cfg!(debug_assertions)) tint grey -> orange so the
        // dev tray is visually distinct from an installed release build.
        // `cargo test` always runs under debug, so the tint must apply.
        let icon_bytes = {
            let (_, _, mut rgba) = decode_png(IDLE_PNG);
            tint_to_orange(&mut rgba);
            rgba
        };
        let mut saw_orange = false;
        for chunk in icon_bytes.chunks_exact(4) {
            if chunk[3] > 128 {
                // Orange: R >> B and R >= G
                if chunk[0] > 100 && chunk[0] >= chunk[1] && chunk[0] > chunk[2] && chunk[2] == 0 {
                    saw_orange = true;
                    break;
                }
            }
        }
        assert!(saw_orange, "expected at least one orange opaque pixel after tinting");
    }
}
