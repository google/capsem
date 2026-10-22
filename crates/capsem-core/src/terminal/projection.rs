//! Project canonical cells as side-effect-free ANSI for thin terminal viewers.
use std::io::Write;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{LineDamageBounds, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use alacritty_terminal::Term;

use super::callbacks::Proxy;

pub(super) fn default_color(index: usize) -> Rgb {
    // Conventional xterm fallback. Application overrides are captured by the
    // engine at query time; viewers may style the unmodified ANSI palette.
    const BASE: [u32; 16] = [
        0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5, 0x7f7f7f, 0xff0000, 0x00ff00,
        0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
    ];
    let packed = match index {
        0..=15 => BASE[index],
        16..=231 => {
            let value = index - 16;
            let level = |n: usize| if n == 0 { 0 } else { 55 + n * 40 };
            (level(value / 36) << 16 | level(value / 6 % 6) << 8 | level(value % 6)) as u32
        }
        232..=255 => {
            let level = 8 + (index - 232) * 10;
            (level * 0x010101) as u32
        }
        257 => 0x000000,
        _ => 0xe5e5e5,
    };
    Rgb {
        r: (packed >> 16) as u8,
        g: (packed >> 8) as u8,
        b: packed as u8,
    }
}

fn color(out: &mut Vec<u8>, color: Color, foreground: bool, term: &Term<Proxy>) {
    let base = if foreground { 38 } else { 48 };
    let color = match color {
        Color::Indexed(i) => term.colors()[usize::from(i)].map_or(color, Color::Spec),
        Color::Named(i) => term.colors()[i].map_or(color, Color::Spec),
        _ => color,
    };
    match color {
        Color::Spec(rgb) => write!(out, "\x1b[{base};2;{};{};{}m", rgb.r, rgb.g, rgb.b).unwrap(),
        Color::Indexed(i) => write!(out, "\x1b[{base};5;{i}m").unwrap(),
        Color::Named(i) => {
            let index = i as usize;
            let code = if index < 8 {
                30 + index
            } else if index < 16 {
                90 + index - 8
            } else if matches!(i, NamedColor::Background) {
                49
            } else if (259..267).contains(&index) {
                30 + index - 259
            } else {
                39
            };
            let code = if !foreground && code != 49 && code != 39 {
                code + 10
            } else if !foreground {
                49
            } else {
                code
            };
            write!(out, "\x1b[{code}m").unwrap();
        }
    }
}

fn style(out: &mut Vec<u8>, cell: &Cell, term: &Term<Proxy>) {
    out.extend_from_slice(b"\x1b[0m");
    for (flag, code) in [
        (Flags::BOLD, "1"),
        (Flags::DIM, "2"),
        (Flags::ITALIC, "3"),
        (Flags::INVERSE, "7"),
        (Flags::HIDDEN, "8"),
        (Flags::STRIKEOUT, "9"),
    ] {
        if cell.flags.contains(flag) {
            write!(out, "\x1b[{code}m").unwrap();
        }
    }
    let underline = if cell.flags.contains(Flags::DOUBLE_UNDERLINE) {
        Some("4:2")
    } else if cell.flags.contains(Flags::UNDERCURL) {
        Some("4:3")
    } else if cell.flags.contains(Flags::DOTTED_UNDERLINE) {
        Some("4:4")
    } else if cell.flags.contains(Flags::DASHED_UNDERLINE) {
        Some("4:5")
    } else if cell.flags.contains(Flags::UNDERLINE) {
        Some("4")
    } else {
        None
    };
    if let Some(code) = underline {
        write!(out, "\x1b[{code}m").unwrap();
    }
    color(out, cell.fg, true, term);
    color(out, cell.bg, false, term);
    if let Some(Color::Spec(rgb)) = cell.underline_color() {
        write!(out, "\x1b[58;2;{};{};{}m", rgb.r, rgb.g, rgb.b).unwrap();
    } else if let Some(Color::Indexed(i)) = cell.underline_color() {
        write!(out, "\x1b[58;5;{i}m").unwrap();
    } else {
        out.extend_from_slice(b"\x1b[59m");
    }
}

pub(super) fn modes(term: &Term<Proxy>, out: &mut Vec<u8>) {
    let mode = term.mode();
    // Projection writes are absolute, independent of the workload's origin,
    // insert, scroll-region and autowrap settings. Restore input modes last.
    out.extend_from_slice(b"\x1b[?6l\x1b[4l\x1b[r\x1b[?7h");
    for (number, flag) in [
        (1, TermMode::APP_CURSOR),
        (2004, TermMode::BRACKETED_PASTE),
        (1000, TermMode::MOUSE_REPORT_CLICK),
        (1002, TermMode::MOUSE_DRAG),
        (1003, TermMode::MOUSE_MOTION),
        (1005, TermMode::UTF8_MOUSE),
        (1006, TermMode::SGR_MOUSE),
        (1004, TermMode::FOCUS_IN_OUT),
    ] {
        write!(out, "\x1b[?{number}{}", if mode.contains(flag) { 'h' } else { 'l' }).unwrap();
    }
    out.extend_from_slice(if mode.contains(TermMode::APP_KEYPAD) {
        b"\x1b="
    } else {
        b"\x1b>"
    });
}

pub(super) fn render(term: &Term<Proxy>, reset: bool, damage: &[LineDamageBounds]) -> Vec<u8> {
    let mut out = Vec::new();
    if reset {
        out.extend_from_slice(b"\x1bc");
        if term.mode().contains(TermMode::ALT_SCREEN) {
            out.extend_from_slice(b"\x1b[?1049h");
        }
    }
    modes(term, &mut out);
    let mut wrapped_from = None;
    for range in damage {
        let row = &term.grid()[Line(range.line as i32)];
        let mut left = range.left;
        let mut right = range.right.min(term.columns() - 1);
        if row[Column(left)].flags.contains(Flags::WIDE_CHAR_SPACER) {
            left = left.saturating_sub(1);
        }
        if row[Column(right)].flags.contains(Flags::WIDE_CHAR) {
            right = (right + 1).min(term.columns() - 1);
        }
        let mut end = right + 1;
        while end > left && row[Column(end - 1)] == Cell::default() {
            end -= 1;
        }
        if wrapped_from != Some(range.line) || left != 0 {
            write!(out, "\x1b[{};{}H", range.line + 1, left + 1).unwrap();
        }
        let mut previous = None;
        let mut link = None;
        for col in left..end {
            let cell = &row[Column(col)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let next_link = cell
                .hyperlink()
                .filter(|link| !link.uri().chars().any(char::is_control));
            if next_link != link {
                out.extend_from_slice(b"\x1b]8;;\x1b\\");
                if let Some(link) = &next_link {
                    out.extend_from_slice(b"\x1b]8;");
                    if !link.id().chars().any(|c| c.is_control() || matches!(c, ':' | ';')) {
                        write!(out, "id={}", link.id()).unwrap();
                    }
                    write!(out, ";{}\x1b\\", link.uri()).unwrap();
                }
                link = next_link;
            }
            let flags = cell.flags
                & !(Flags::WRAPLINE | Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
            let current = (cell.fg, cell.bg, flags, cell.underline_color());
            if previous != Some(current) {
                style(&mut out, cell, term);
                previous = Some(current);
            }
            let c = if cell.c == '\t' { ' ' } else { cell.c };
            let mut bytes = [0; 4];
            out.extend_from_slice(c.encode_utf8(&mut bytes).as_bytes());
            for c in cell.zerowidth().into_iter().flatten() {
                out.extend_from_slice(c.encode_utf8(&mut bytes).as_bytes());
            }
        }
        out.extend_from_slice(b"\x1b]8;;\x1b\\");
        if end < right + 1 {
            // Erase untouched padding instead of manufacturing copied text.
            out.extend_from_slice(b"\x1b[0m");
            if right + 1 == term.columns() {
                out.extend_from_slice(b"\x1b[K");
            } else {
                write!(out, "\x1b[{}X", right + 1 - end).unwrap();
            }
        }
        wrapped_from =
            (end == term.columns() && row[Column(right)].flags.contains(Flags::WRAPLINE)).then_some(range.line + 1);
    }
    let cursor = term.grid().cursor.point;
    write!(
        out,
        "\x1b[0m\x1b[{};{}H\x1b[?25{}",
        cursor.line.0 + 1,
        cursor.column.0 + 1,
        if term.mode().contains(TermMode::SHOW_CURSOR) {
            'h'
        } else {
            'l'
        }
    )
    .unwrap();
    let cursor = term.cursor_style();
    let shape = match cursor.shape {
        alacritty_terminal::vte::ansi::CursorShape::Beam => 5,
        alacritty_terminal::vte::ansi::CursorShape::Underline => 3,
        _ => 1,
    };
    write!(out, "\x1b[{} q", shape + usize::from(!cursor.blinking)).unwrap();
    // No title, notification, bell, clipboard or query is replayed to viewers.
    out
}
