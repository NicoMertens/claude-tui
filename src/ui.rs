use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use tui_term::widget::{Cursor, PseudoTerminal};

use std::time::{Duration, SystemTime};

use crate::{app::App, dirpick::DirPicker, keys, list::Entry, menu::Menu, theme, windows::Focus};

const SIDEBAR_WIDTH: u16 = 42;
/// Kennzeichnet die Verzeichnis-Überschriften in der Sessionliste.
const FOLDER_ICON: &str = "📁";

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

/// Mindesthöhe der Sessionliste, wenn darunter das Menü offen ist.
const MIN_LIST_HEIGHT: u16 = 5;

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    // Menü bzw. Verzeichnisauswahl: eigene Box unter der Sessionliste.
    let panel_width = area.width.saturating_sub(2) as usize;
    let panel = match (&app.dir_picker, &app.menu) {
        (Some(picker), _) => Some((
            "Verzeichnis".to_string(),
            dir_picker_lines(picker, panel_width),
        )),
        (None, Some(menu)) => Some((menu.title.clone(), menu_lines(menu, panel_width))),
        (None, None) => None,
    };
    let panel_height = panel.as_ref().map_or(0, |(_, lines)| {
        (lines.len() as u16 + 2).min(area.height.saturating_sub(MIN_LIST_HEIGHT))
    });
    let [list_box, panel_box] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(panel_height)]).areas(area);
    if let Some((title, lines)) = panel {
        draw_panel(f, &title, lines, panel_box);
    }

    let focused = app.windows.focus() == Focus::List;
    let title = Span::styled(
        " ✻ claude-tui ",
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    let block = rounded(frame_color(focused)).title(Line::from(title));
    let inner = block.inner(list_box);
    f.render_widget(block, list_box);

    let width = inner.width as usize;
    let mut bottom_lines = if panel_height == 0 {
        vec![Line::from(vec![
            Span::styled(keys::PREFIX_HINT, Style::default().fg(theme::ACCENT)),
            Span::styled(" Menü", Style::default().fg(theme::MUTED)),
        ])]
    } else {
        Vec::new()
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

/// Box mit blauem Rand für Menü, Fensterauswahl, Löschabfrage und Verzeichnisauswahl. Passt der
/// Inhalt nicht ganz hinein, bleiben die unteren Zeilen sichtbar.
fn draw_panel(f: &mut Frame, title: &str, mut lines: Vec<Line<'static>>, area: Rect) {
    let block = rounded(theme::MENU).title(Line::from(Span::styled(
        format!(" {title} "),
        Style::default()
            .fg(theme::MENU)
            .add_modifier(Modifier::BOLD),
    )));
    let inner = block.inner(area);
    let skip = lines.len().saturating_sub(inner.height as usize);
    lines.drain(..skip);
    f.render_widget(Paragraph::new(lines).block(block), area);
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

/// Zeile über die ganze Breite, ausgewählt in der Menüfarbe hinterlegt.
fn selectable(text: String, selected: bool, width: usize) -> Line<'static> {
    let style = if selected {
        Style::default().fg(theme::BG).bg(theme::MENU)
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
    let mut lines = vec![Line::from(vec![
        Span::styled("> ", Style::default().fg(theme::ACCENT)),
        Span::styled(
            tail(&picker.input, width.saturating_sub(3)),
            Style::default().fg(theme::TEXT),
        ),
        Span::styled("█", Style::default().fg(theme::ACCENT)),
    ])];
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
        let header = tail(&group.cwd.display().to_string(), width.saturating_sub(4));
        // Das Ordner-Emoji ist zwei Spalten breit.
        let line = Line::from(vec![
            Span::raw(format!("{FOLDER_ICON} ")),
            Span::styled(header, Style::default().fg(theme::MUTED)),
        ]);
        items.push(ListItem::new(line));
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

/// Eine Zeile: Zustandssymbol, Titel, Fensternummer, Zeit seit der letzten Aktivität.
fn entry_item(app: &App, entry: &Entry, selected: bool, width: usize) -> ListItem<'static> {
    let session = entry.running.and_then(|id| app.session(id));
    let (symbol, color) = theme::status_symbol(session.map(|s| s.status), clock());
    let slot = entry.running.and_then(|id| app.windows.window_of(id));
    let window = slot.map_or_else(|| " ".repeat(4), |s| format!(" [{}]", s + 1));
    let since = match session {
        Some(s) => Some(s.last_output().elapsed()),
        None => entry
            .modified
            .map(|m| SystemTime::now().duration_since(m).unwrap_or_default()),
    };
    let time = since.map(short_age).unwrap_or_default();
    // Symbol + Leerzeichen, Fensterspalte, Leerzeichen + Zeit (max. 5 Zeichen).
    let title_width = width.saturating_sub(2 + 4 + 6);
    let title = head(&entry.title, title_width);
    let padding = " ".repeat(title_width.saturating_sub(title.chars().count()));

    let name_style = if session.is_some() || selected {
        Style::default().fg(theme::TEXT)
    } else {
        Style::default().fg(theme::MUTED)
    };
    let line = Line::from(vec![
        Span::styled(format!("{symbol} "), Style::default().fg(color)),
        Span::styled(format!("{title}{padding}"), name_style),
        Span::styled(window, Style::default().fg(theme::ACCENT)),
        Span::styled(format!(" {time:>5}"), Style::default().fg(theme::MUTED)),
    ]);
    let item = ListItem::new(line);
    if selected {
        item.style(
            Style::default()
                .bg(theme::SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        item
    }
}

/// Uhr für den Spinner; alle Spinner laufen damit im Gleichtakt.
fn clock() -> Duration {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
}

/// Kürzt hinten mit „…“ (für Titel).
fn head(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let keep: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{keep}…")
}

/// „jetzt“, „5m“, „3h“, „2d“.
fn short_age(since: Duration) -> String {
    let secs = since.as_secs();
    match secs {
        0..=59 => "jetzt".to_string(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// Menü unten in der Sessionliste, nach unten ausgerichtet.
fn menu_lines(menu: &Menu, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
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
        let (label, label_color) = theme::status_label(session.status);
        let (symbol, _) = theme::status_symbol(Some(session.status), clock());
        let separator = Span::styled("· ", Style::default().fg(color));
        let status = Span::styled(
            format!("{symbol} {label} "),
            Style::default().fg(label_color),
        );
        let block = rounded(color).title(Line::from(vec![title, separator, status]));
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
