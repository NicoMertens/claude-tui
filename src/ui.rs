use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
    Frame,
};
use tui_term::widget::PseudoTerminal;

use crate::{app::App, keys, theme};

const SIDEBAR_WIDTH: u16 = 28;

struct Areas {
    sidebar: Rect,
    pane: Rect,
    footer: Rect,
}

fn areas(area: Rect) -> Areas {
    let [body, footer] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    let [sidebar, pane] =
        Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(10)]).areas(body);
    Areas {
        sidebar,
        pane,
        footer,
    }
}

/// Zeilen und Spalten, die eine Session im Hauptfenster zur Verfügung hat.
pub fn pane_size(width: u16, height: u16) -> (u16, u16) {
    let inner = areas(Rect::new(0, 0, width, height))
        .pane
        .inner(Margin::new(1, 1));
    (inner.height.max(1), inner.width.max(1))
}

pub fn draw(f: &mut Frame, app: &App) {
    let a = areas(f.area());
    f.render_widget(
        Block::default().style(Style::default().bg(theme::BG)),
        f.area(),
    );
    draw_sidebar(f, app, a.sidebar);
    draw_pane(f, app, a.pane);
    draw_footer(f, app, a.footer);
}

fn rounded(color: ratatui::style::Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let title = Span::styled(
        " ✻ claude-tui ",
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    let block = rounded(theme::ACCENT_DIM).title(Line::from(title));

    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let selected = i == app.active;
            let (label, color) = theme::status(session.status);
            let name_style = if selected {
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::MUTED)
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        if selected { "▌" } else { " " },
                        Style::default().fg(theme::ACCENT),
                    ),
                    Span::styled(format!("{} ", i + 1), Style::default().fg(theme::MUTED)),
                    Span::styled(session.name.clone(), name_style),
                ]),
                Line::from(vec![
                    Span::raw("   "),
                    Span::styled("● ", Style::default().fg(color)),
                    Span::styled(label, Style::default().fg(color)),
                ]),
            ])
        })
        .collect();

    f.render_widget(List::new(items).block(block), area);
}

fn draw_pane(f: &mut Frame, app: &App, area: Rect) {
    let Some(session) = app.sessions.get(app.active) else {
        let hint = format!("Keine Session – {} c startet eine neue", keys::PREFIX_HINT);
        f.render_widget(Paragraph::new(hint).block(rounded(theme::ACCENT_DIM)), area);
        return;
    };
    let title = Span::styled(
        format!(" {} ", session.name),
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    let block = rounded(theme::ACCENT).title(Line::from(title));
    let parser = session.screen();
    f.render_widget(PseudoTerminal::new(parser.screen()).block(block), area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let commands = "c neu · x schließen · n/p wechseln · 1–9 wählen · q beenden";
    let line = if app.prefix_armed {
        Line::from(vec![
            Span::styled(" BEFEHL ", Style::default().fg(theme::BG).bg(theme::ACCENT)),
            Span::styled(
                format!(" {commands} · {} senden", keys::PREFIX_HINT),
                Style::default().fg(theme::TEXT),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                format!(" {} ", keys::PREFIX_HINT),
                Style::default().fg(theme::ACCENT),
            ),
            Span::styled(
                format!("dann: {commands}"),
                Style::default().fg(theme::MUTED),
            ),
        ])
    };
    f.render_widget(Paragraph::new(line), area);
}
