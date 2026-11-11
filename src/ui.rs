use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use tui_term::widget::{Cursor, PseudoTerminal};

use std::time::SystemTime;

use crate::{app::App, dirpick::DirPicker, keys, list::Entry, menu::Menu, theme, windows::Focus};

const SIDEBAR_WIDTH: u16 = 34;

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

    let width = inner.width as usize;
    let mut bottom_lines = match (&app.dir_picker, &app.menu) {
        (Some(picker), _) => dir_picker_lines(picker, width),
        (None, Some(menu)) => menu_lines(menu, width),
        (None, None) => vec![Line::from(vec![
            Span::styled(keys::PREFIX_HINT, Style::default().fg(theme::ACCENT)),
            Span::styled(" Menü", Style::default().fg(theme::MUTED)),
        ])],
    };
    if let Some(notice) = &app.notice {
        let line = Line::from(Span::styled(
            head(notice, width),
            Style::default().fg(theme::WAITING),
        ));
        bottom_lines.insert(0, line);
    }
    // Passt der untere Teil nicht ganz hinein, bleiben seine unteren Zeilen sichtbar.
    let skip = bottom_lines.len().saturating_sub(inner.height as usize);
    bottom_lines.drain(..skip);
    let [list_area, bottom] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(bottom_lines.len() as u16),
    ])
    .areas(inner);

    draw_session_list(f, app, list_area);
    f.render_widget(Paragraph::new(bottom_lines), bottom);
}

fn section_title(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("── {title} "),
        Style::default().fg(theme::MUTED),
    ))
}

fn muted(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(theme::MUTED),
    ))
}

/// Zeile über die ganze Breite, ausgewählt in Akzentfarbe hinterlegt.
fn selectable(text: String, selected: bool, width: usize) -> Line<'static> {
    let style = if selected {
        Style::default().fg(theme::BG).bg(theme::ACCENT)
    } else {
        Style::default().fg(theme::TEXT)
    };
    Line::from(Span::styled(format!("{text:<width$}"), style))
}

/// Kürzt vorne mit „…“, damit bei Pfaden das Ende (der Ordnername) sichtbar bleibt.
fn tail(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    let rest: String = text.chars().skip(count - keep).collect();
    format!("…{rest}")
}

fn dir_picker_lines(picker: &DirPicker, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![section_title("Verzeichnis")];
    lines.push(Line::from(vec![
        Span::styled("> ", Style::default().fg(theme::ACCENT)),
        Span::styled(
            tail(&picker.input, width.saturating_sub(3)),
            Style::default().fg(theme::TEXT),
        ),
        Span::styled("█", Style::default().fg(theme::ACCENT)),
    ]));
    if !picker.recent.is_empty() {
        lines.push(section_title("zuletzt"));
        for (i, dir) in picker.recent.iter().enumerate() {
            let text = format!(" {}", tail(&dir.display().to_string(), width - 1));
            lines.push(selectable(text, picker.selected == Some(i), width));
        }
    }
    lines.push(match picker.error {
        Some(error) => Line::from(Span::styled(
            format!(" {error}"),
            Style::default().fg(theme::WAITING),
        )),
        None => muted(" Tab ergänzen · Enter · Esc"),
    });
    lines
}

fn draw_session_list(f: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_item = None;
    for group in &app.groups {
        let header = tail(&group.cwd.display().to_string(), width.saturating_sub(3));
        items.push(ListItem::new(muted(&format!("── {header}"))));
        for entry in &group.entries {
            let selected = app.cursor.as_deref() == Some(entry.claude_id.as_str());
            if selected {
                selected_item = Some(items.len());
            }
            items.push(entry_item(app, entry, selected, width));
        }
    }
    let mut state = ListState::default().with_selected(selected_item);
    f.render_stateful_widget(List::new(items), area, &mut state);
}

/// Zwei Zeilen: Titel mit Fensternummer, darunter Status (laufend) oder Alter (gespeichert).
fn entry_item(app: &App, entry: &Entry, selected: bool, width: usize) -> ListItem<'static> {
    let slot = entry.running.and_then(|id| app.windows.window_of(id));
    let suffix = slot.map(|s| format!(" [{}]", s + 1)).unwrap_or_default();
    let title_width = width.saturating_sub(2 + suffix.chars().count());
    let name_style = match (selected, entry.running.is_some()) {
        (true, _) => Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD),
        (false, true) => Style::default().fg(theme::TEXT),
        (false, false) => Style::default().fg(theme::MUTED),
    };
    let first = Line::from(vec![
        Span::styled(
            if selected { "▌ " } else { "  " },
            Style::default().fg(theme::ACCENT),
        ),
        Span::styled(head(&entry.title, title_width), name_style),
        Span::styled(suffix, Style::default().fg(theme::ACCENT)),
    ]);
    let status = entry
        .running
        .and_then(|id| app.session(id))
        .map(|s| s.status);
    let second = match status {
        Some(status) => {
            let (label, color) = theme::status(status);
            Line::from(vec![
                Span::raw("    "),
                Span::styled("● ", Style::default().fg(color)),
                Span::styled(label, Style::default().fg(color)),
            ])
        }
        None => muted(&format!(
            "    {}",
            entry.modified.map(age).unwrap_or_default()
        )),
    };
    ListItem::new(vec![first, second])
}

/// Kürzt hinten mit „…“ (für Titel).
fn head(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let keep: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{keep}…")
}

/// „vor 5 Min.“, „vor 3 Std.“, „vor 2 Tagen“.
fn age(modified: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(modified)
        .map_or(0, |d| d.as_secs());
    match secs {
        0..=59 => "gerade eben".to_string(),
        60..=3599 => format!("vor {} Min.", secs / 60),
        3600..=86_399 => format!("vor {} Std.", secs / 3600),
        86_400..=172_799 => "vor 1 Tag".to_string(),
        _ => format!("vor {} Tagen", secs / 86_400),
    }
}

/// Menü unten in der Sessionliste, nach unten ausgerichtet.
fn menu_lines(menu: &Menu, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![section_title(&menu.title)];
    for (i, item) in menu.items.iter().enumerate() {
        let text = format!(" {:<3}{}", item.hotkey.label(), item.label);
        lines.push(selectable(text, i == menu.selected, width));
    }
    lines.push(muted(" Esc schließen"));
    lines
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
            format!(" {} · {} ", slot + 1, app.title_of(id)),
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
