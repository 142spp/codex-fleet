//! Export the actual rendered cells, including final colors and modifiers.
use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier},
};
use unicode_width::UnicodeWidthStr;

const PALETTE: [&str; 16] = [
    "#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
    "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
];
fn color(value: Color, default: &str) -> String {
    let index = match value {
        Color::Reset => return default.into(),
        Color::Rgb(r, g, b) => return format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => i,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
    };
    match index {
        0..=15 => PALETTE[index as usize].into(),
        16..=231 => {
            let n = index - 16;
            let component = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            format!(
                "#{:02x}{:02x}{:02x}",
                component(n / 36),
                component((n % 36) / 6),
                component(n % 6)
            )
        }
        _ => {
            let gray = 8 + 10 * (index - 232);
            format!("#{gray:02x}{gray:02x}{gray:02x}")
        }
    }
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn svg(buffer: &Buffer) -> String {
    let cell_width = 9;
    let cell_height = 20;
    let padding = 20;
    let width = u32::from(buffer.area.width) * cell_width + 2 * padding;
    let height = u32::from(buffer.area.height) * cell_height + 2 * padding;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" role=\"img\" aria-label=\"Codex Fleet rendered demo\">\n<rect width=\"100%\" height=\"100%\" rx=\"10\" fill=\"#19191c\"/>\n<g font-family=\"DejaVu Sans Mono, monospace\" font-size=\"14\" xml:space=\"preserve\">\n"
    );
    for y in 0..buffer.area.height {
        let mut x = 0;
        while x < buffer.area.width {
            let cell = &buffer[(buffer.area.x + x, buffer.area.y + y)];
            let style = (cell.fg, cell.bg, cell.modifier);
            let start = x;
            let mut cells = Vec::new();
            while x < buffer.area.width {
                let cell = &buffer[(buffer.area.x + x, buffer.area.y + y)];
                if (cell.fg, cell.bg, cell.modifier) != style {
                    break;
                }
                let width = UnicodeWidthStr::width(cell.symbol()).max(1) as u16;
                if !cell.symbol().trim().is_empty() {
                    cells.push((x, width, cell.symbol()));
                }
                x += width;
            }
            let (fg, bg, modifiers) = style;
            let mut fg = color(fg, "#cecbd8");
            let mut bg = color(bg, "#19191c");
            if modifiers.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let left = padding + u32::from(start) * cell_width;
            let top = padding + u32::from(y) * cell_height;
            let length = u32::from(x - start) * cell_width;
            if bg != "#19191c" {
                svg.push_str(&format!("<rect x=\"{left}\" y=\"{top}\" width=\"{length}\" height=\"{cell_height}\" fill=\"{bg}\"/>\n"));
            }
            if !cells.is_empty() {
                let weight = if modifiers.contains(Modifier::BOLD) {
                    "bold"
                } else {
                    "normal"
                };
                let italic = if modifiers.contains(Modifier::ITALIC) {
                    "italic"
                } else {
                    "normal"
                };
                let decoration = if modifiers.contains(Modifier::UNDERLINED) {
                    "underline"
                } else {
                    "none"
                };
                let opacity = if modifiers.contains(Modifier::DIM) {
                    "0.65"
                } else {
                    "1"
                };
                for (column, width, symbol) in cells {
                    let left = padding + u32::from(column) * cell_width;
                    let length = u32::from(width) * cell_width;
                    svg.push_str(&format!("<text x=\"{left}\" y=\"{}\" textLength=\"{length}\" lengthAdjust=\"spacingAndGlyphs\" fill=\"{fg}\" font-weight=\"{weight}\" font-style=\"{italic}\" text-decoration=\"{decoration}\" opacity=\"{opacity}\">{}</text>\n",top+15,escape(symbol)));
                }
            }
        }
    }
    svg.push_str("</g>\n</svg>\n");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{layout::Rect, style::Style};
    #[test]
    fn exports_final_styles_and_grid_positions() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        buffer.set_string(
            0,
            0,
            "<&",
            Style::default()
                .fg(Color::Red)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        );
        let output = svg(&buffer);
        assert!(output.contains("width=\"18\" height=\"20\" fill=\"#2472c8\""));
        assert!(output.contains("fill=\"#cd3131\" font-weight=\"bold\""));
        assert!(output.contains(">&lt;</text>"));
        assert!(output.contains(">&amp;</text>"));
    }
    #[test]
    fn wide_characters_occupy_two_cells() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer.set_string(0, 0, "한x", ratatui::style::Style::default());
        let output = svg(&buffer);
        assert!(output.contains("textLength=\"18\""));
        assert!(output.contains(">한</text>"));
        assert!(output.contains("<text x=\"38\""));
    }
}
