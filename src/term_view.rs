//! Zeichnet einen `vt100`-Bildschirm als ratatui-Widget.
//!
//! Eigenbau statt `tui-term`: dessen ratatui-0.29-Version hängt an `vt100` 0.15, und das verwirft
//! das Attribut *dim* (SGR 2). Claude Code zeichnet damit z. B. Prompt-Vorschläge grau.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};

use crate::selection::{self, Cell};

/// Hintergrund markierter Zellen — das Blau der Menübox.
const SELECTION_BG: Color = crate::theme::MENU;

pub struct TermView<'a> {
    screen: &'a vt100::Screen,
    show_cursor: bool,
    selection: Option<(Cell, Cell)>,
}

impl<'a> TermView<'a> {
    pub fn new(screen: &'a vt100::Screen) -> Self {
        Self {
            screen,
            show_cursor: true,
            selection: None,
        }
    }

    pub fn show_cursor(mut self, show: bool) -> Self {
        self.show_cursor = show;
        self
    }

    /// Markierter Bereich (Leserichtung, Enden eingeschlossen); wird invers gezeichnet.
    pub fn selection(mut self, range: Option<(Cell, Cell)>) -> Self {
        self.selection = range;
        self
    }
}

impl Widget for TermView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Nach einem Verkleinern kann die Fläche kurz größer sein als der Puffer (Layout noch auf
        // alter Größe) — nie außerhalb schreiben, sonst bricht ratatui ab.
        let area = area.intersection(*buf.area());
        let (rows, cols) = self.screen.size();
        let rows = rows.get().min(area.height);
        let cols = cols.get().min(area.width);
        for row in 0..rows {
            for col in 0..cols {
                let Some(cell) = self.screen.cell(row, col) else {
                    continue;
                };
                // Die rechte Hälfte eines breiten Zeichens zeichnet ratatui selbst mit.
                if cell.is_wide_continuation() {
                    continue;
                }
                let target = &mut buf[(area.x + col, area.y + row)];
                if cell.has_contents() {
                    target.set_symbol(cell.contents());
                } else {
                    target.set_symbol(" ");
                }
                let mut style = style_of(cell);
                if self
                    .selection
                    .is_some_and(|range| selection::contains(range, Cell { row, col }))
                {
                    style = style.bg(SELECTION_BG).fg(Color::Black);
                }
                target.set_style(style);
            }
        }
        let (cursor_row, cursor_col) = self.screen.cursor_position();
        let cursor_visible = self.show_cursor
            && !self.screen.hide_cursor()
            && self.screen.scrollback() == 0
            && cursor_row < rows
            && cursor_col < cols;
        if cursor_visible {
            let target = &mut buf[(area.x + cursor_col, area.y + cursor_row)];
            target.set_style(Style::default().add_modifier(Modifier::REVERSED));
        }
    }
}

fn style_of(cell: &vt100::Cell) -> Style {
    let mut style = Style::default()
        .fg(color(cell.fgcolor()))
        .bg(color(cell.bgcolor()));
    let attributes = [
        (cell.bold(), Modifier::BOLD),
        (cell.dim(), Modifier::DIM),
        (cell.italic(), Modifier::ITALIC),
        (cell.underline(), Modifier::UNDERLINED),
        (cell.inverse(), Modifier::REVERSED),
    ];
    for (active, modifier) in attributes {
        if active {
            style = style.add_modifier(modifier);
        }
    }
    style
}

fn color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU16;

    fn render(bytes: &[u8]) -> Buffer {
        let size = |n| NonZeroU16::new(n).expect("> 0");
        let mut parser = vt100::Parser::new(size(2), size(10), 0);
        parser.process(bytes);
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        TermView::new(parser.screen())
            .show_cursor(false)
            .render(area, &mut buf);
        buf
    }

    #[test]
    fn dim_text_keeps_dim_modifier() {
        let buf = render(b"\x1b[2mgrau\x1b[0m weiss");
        assert_eq!(buf[(0, 0)].symbol(), "g");
        assert!(buf[(0, 0)].modifier.contains(Modifier::DIM));
        assert!(!buf[(5, 0)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn colors_are_mapped() {
        let buf = render(b"\x1b[31mA\x1b[38;2;1;2;3mB");
        assert_eq!(buf[(0, 0)].fg, Color::Indexed(1));
        assert_eq!(buf[(1, 0)].fg, Color::Rgb(1, 2, 3));
    }

    #[test]
    fn area_larger_than_buffer_does_not_panic() {
        // Wie nach einem Verkleinern: Layout noch 30×80, Puffer schon 27 Zeilen.
        let size = |n| NonZeroU16::new(n).expect("> 0");
        let mut parser = vt100::Parser::new(size(30), size(80), 0);
        parser.process(b"\x1b[30;1Hletzte Zeile");
        let mut buf = Buffer::empty(Rect::new(0, 0, 102, 27));
        TermView::new(parser.screen()).render(Rect::new(20, 2, 80, 30), &mut buf);
        assert_eq!(buf[(20, 2)].symbol(), " ");
    }
}
