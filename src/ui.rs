use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use tui_term::widget::{Cursor, PseudoTerminal};

use crate::{app::App, keys, menu::Menu, theme, windows::Focus};

const SIDEBAR_WIDTH: u16 = 28;

fn split(area: Rect) -> [Rect; 2] {
    Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(10)]).areas(area)
}

/// Fläche rechts neben der Sessionliste, auf die die Fenster verteilt werden.
pub fn windows_area(width: u16, height: u16) -> Rect {
    split(Rect::new(0, 0, width, height))[1]
}

pub fn draw(f: &mut Frame, app: &App) {
    let [sidebar, _] = split(f.area());
    f.render_widget(
        Block::default().style(Style::default().bg(theme::BG)),
        f.area(),
    );
    draw_sidebar(f, app, sidebar);
    draw_windows(f, app);
}

fn rounded(color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
}

fn frame_color(focused: bool) -> Color {
    if focused {
        theme::FOCUS
    } else {
        theme::UNFOCUS
    }
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.windows.focus() == Focus::List;
    let title = Span::styled(
        " ✻ claude-tui ",
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    let block = rounded(frame_color(focused)).title(Line::from(title));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let bottom_height = match &app.menu {
        Some(menu) => menu.items.len() as u16 + 2,
        None => 1,
    };
    let [list_area, bottom] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(bottom_height)]).areas(inner);

    draw_session_list(f, app, list_area);
    match &app.menu {
        Some(menu) => draw_menu(f, menu, bottom),
        None => {
            let hint = Line::from(vec![
                Span::styled(keys::PREFIX_HINT, Style::default().fg(theme::ACCENT)),
                Span::styled(" Menü", Style::default().fg(theme::MUTED)),
            ]);
            f.render_widget(Paragraph::new(hint), bottom);
        }
    }
}

fn draw_session_list(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let selected = i == app.cursor;
            let (label, color) = theme::status(session.status);
            let name_style = if selected {
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::MUTED)
            };
            let mut first = vec![
                Span::styled(
                    if selected { "▌" } else { " " },
                    Style::default().fg(theme::ACCENT),
                ),
                Span::styled(session.name.clone(), name_style),
            ];
            if let Some(slot) = app.windows.window_of(session.id) {
                first.push(Span::styled(
                    format!(" [{}]", slot + 1),
                    Style::default().fg(theme::ACCENT),
                ));
            }
            ListItem::new(vec![
                Line::from(first),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled("● ", Style::default().fg(color)),
                    Span::styled(label, Style::default().fg(color)),
                ]),
            ])
        })
        .collect();

    let mut state = ListState::default().with_selected(Some(app.cursor));
    f.render_stateful_widget(List::new(items), area, &mut state);
}

/// Menü unten in der Sessionliste, nach unten ausgerichtet.
fn draw_menu(f: &mut Frame, menu: &Menu, area: Rect) {
    let mut lines = vec![Line::from(Span::styled(
        format!("── {} ", menu.title),
        Style::default().fg(theme::MUTED),
    ))];
    for (i, item) in menu.items.iter().enumerate() {
        let text = format!(" {:<3}{}", item.hotkey.label(), item.label);
        let style = if i == menu.selected {
            Style::default().fg(theme::BG).bg(theme::ACCENT)
        } else {
            Style::default().fg(theme::TEXT)
        };
        lines.push(Line::from(Span::styled(
            format!("{text:<width$}", width = area.width as usize),
            style,
        )));
    }
    lines.push(Line::from(Span::styled(
        " Esc schließen",
        Style::default().fg(theme::MUTED),
    )));
    // Passt das Menü nicht ganz hinein, bleiben die unteren Zeilen sichtbar.
    let skip = lines.len().saturating_sub(area.height as usize);
    f.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()),
        area,
    );
}

fn draw_windows(f: &mut Frame, app: &App) {
    let rects = app.window_rects();
    if rects.is_empty() {
        let [_, area] = split(f.area());
        let hint = format!(
            " Kein Fenster offen – {} n startet eine Session",
            keys::PREFIX_HINT
        );
        let text = Paragraph::new(Span::styled(hint, Style::default().fg(theme::MUTED)));
        f.render_widget(text.block(rounded(theme::UNFOCUS)), area);
        return;
    }
    for (slot, (&id, rect)) in app.windows.slots().iter().zip(rects).enumerate() {
        let Some(session) = app.session(id) else {
            continue;
        };
        let focused = app.windows.focus() == Focus::Window(slot);
        let color = frame_color(focused);
        let title = Span::styled(
            format!(" {} · {} ", slot + 1, session.name),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        );
        let block = rounded(color).title(Line::from(title));
        let mut cursor = Cursor::default();
        if !focused {
            cursor.hide();
        }
        let parser = session.screen();
        f.render_widget(
            PseudoTerminal::new(parser.screen())
                .block(block)
                .cursor(cursor),
            rect,
        );
    }
}
