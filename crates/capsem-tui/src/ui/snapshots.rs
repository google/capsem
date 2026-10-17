use super::*;

pub(super) fn buffer_to_svg(buffer: &Buffer) -> String {
    const CHAR_WIDTH: usize = 11;
    const LINE_HEIGHT: usize = 22;
    const FONT_SIZE: usize = 16;
    const PAD: usize = 16;

    let width = buffer.area.width as usize;
    let height = buffer.area.height as usize;
    let svg_width = width * CHAR_WIDTH + PAD * 2;
    let content_height = height * LINE_HEIGHT + PAD * 2;
    let svg_height = svg_width.max(content_height);
    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{svg_width}\" height=\"{svg_height}\" viewBox=\"0 0 {svg_width} {svg_height}\">\n"
    ));
    svg.push_str(&format!(
        "<rect width=\"100%\" height=\"100%\" fill=\"{}\"/>\n",
        color_hex(PREVIEW_BG)
    ));
    svg.push_str(
        "<style>text{font-family:Menlo,Monaco,Consolas,monospace;dominant-baseline:text-before-edge;}</style>\n",
    );

    for y in 0..height {
        for x in 0..width {
            let cell = &buffer.content()[y * width + x];
            let bg = if cell.bg == Color::Reset { PREVIEW_BG } else { cell.bg };
            let rect_x = PAD + x * CHAR_WIDTH;
            let rect_y = PAD + y * LINE_HEIGHT;
            svg.push_str(&format!(
                "<rect x=\"{rect_x}\" y=\"{rect_y}\" width=\"{CHAR_WIDTH}\" height=\"{LINE_HEIGHT}\" fill=\"{}\"/>\n",
                color_hex(bg)
            ));

            let symbol = cell.symbol();
            if symbol == " " {
                continue;
            }
            let fg = if cell.fg == Color::Reset { TEXT } else { cell.fg };
            let weight = if cell.modifier.contains(Modifier::BOLD) {
                "700"
            } else {
                "400"
            };
            svg.push_str(&format!(
                "<text x=\"{rect_x}\" y=\"{rect_y}\" font-size=\"{FONT_SIZE}\" font-weight=\"{weight}\" fill=\"{}\">{}</text>\n",
                color_hex(fg),
                escape_xml(symbol)
            ));
        }
    }
    svg.push_str("</svg>\n");
    svg
}

fn color_hex(color: Color) -> String {
    match color {
        Color::Reset => color_hex(TEXT),
        Color::Black => "#000000".to_string(),
        Color::Red => "#f38ba8".to_string(),
        Color::Green => "#a6e3a1".to_string(),
        Color::Yellow => "#f9e2af".to_string(),
        Color::Blue => "#89b4fa".to_string(),
        Color::Magenta => "#cba6f7".to_string(),
        Color::Cyan => "#89dceb".to_string(),
        Color::Gray => "#bac2de".to_string(),
        Color::DarkGray => "#585b70".to_string(),
        Color::LightRed => "#f38ba8".to_string(),
        Color::LightGreen => "#a6e3a1".to_string(),
        Color::LightYellow => "#f9e2af".to_string(),
        Color::LightBlue => "#89b4fa".to_string(),
        Color::LightMagenta => "#cba6f7".to_string(),
        Color::LightCyan => "#89dceb".to_string(),
        Color::White => "#ffffff".to_string(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(index) => {
            let gray = index.max(16);
            format!("#{gray:02x}{gray:02x}{gray:02x}")
        }
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
