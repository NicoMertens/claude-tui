use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use std::time::{Duration, SystemTime};

use crate::{
    app::{App, ListHits},
    dirpick::DirPicker,
    keys,
    list::Entry,
    menu::Menu,
    term_view::TermView,
    theme, update,
    windows::Focus,
};

const SIDEBAR_WIDTH: u16 = 42;
/// Kennzeichnet die Verzeichnis-Überschriften in der Sessionliste.
const FOLDER_ICON: &str = "📁";

/// Teilt den Bildschirm in Sessionliste und Fensterfläche; bei ausgeblendeter Liste bekommen die
/// Fenster die volle Breite (die Liste ist dann leer).
fn split(area: Rect, list_hidden: bool) -> [Rect; 2] {
    if list_hidden {
        return [Rect::new(area.x, area.y, 0, area.height), area];
    }
    Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(10)]).areas(area)
}

/// Fläche, auf die die Fenster verteilt werden: rechts neben der Sessionliste bzw. der ganze
/// Bildschirm, wenn sie ausgeblendet ist.
pub fn windows_area(screen: Rect, list_hidden: bool) -> Rect {
    split(screen, list_hidden)[1]
}

pub fn draw(f: &mut Frame, app: &App) {
    let [sidebar, _] = split(f.area(), app.list_hidden);
    f.render_widget(
        Block::default().style(Style::default().bg(theme::BG)),
        f.area(),
    );
    if app.list_hidden {
        // Keine sichtbare Liste: alte Klickflächen dürfen nicht mehr greifen.
        app.list_hits.replace(ListHits::default());
    } else {
        draw_sidebar(f, app, sidebar);
    }
    draw_windows(f, app);
    draw_overlay(f, app);
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
    // Das Menü erscheint als Overlay über den Fenstern; im unteren Rahmen der Liste steht nur, wie es
    // sich öffnet (und ob ein Update wartet).
    let mut hint = vec![
        Span::raw(" "),
        Span::styled(keys::PREFIX_HINT, Style::default().fg(theme::ACCENT)),
        Span::styled(" Menu", Style::default().fg(theme::MUTED)),
    ];
    if !app.marked.is_empty() {
        hint.push(Span::styled(
            format!(" · {} marked", app.marked.len()),
            Style::default().fg(theme::MENU),
        ));
    }
    if let Some(update) = &app.update {
        hint.push(Span::styled(
            format!(" · v{} available (u)", update.version),
            Style::default().fg(theme::MENU),
        ));
    }
    hint.push(Span::raw(" "));

    let focused = app.windows.focus() == Focus::List;
    let title = vec![
        Span::styled(
            " ✻ claude-tui ",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("v{} ", update::CURRENT),
            Style::default().fg(theme::MUTED),
        ),
    ];
    let block = rounded(frame_color(focused))
        .title(Line::from(title))
        .title_bottom(Line::from(hint));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let width = inner.width as usize;
    let mut bottom_lines = Vec::new();
    if let Some(notice) = &app.notice {
        bottom_lines.push(Line::from(Span::styled(
            head(notice, width),
            Style::default().fg(theme::WAITING),
        )));
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
    let mut block = rounded(theme::MENU);
    if !title.is_empty() {
        block = block.title(Line::from(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme::MENU)
                .add_modifier(Modifier::BOLD),
        )));
    }
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

const DIR_PICKER_HINT: &str = " Tab complete · Enter · Esc";
const MENU_FOOTER: &str = " Esc close";

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
        lines.push(section_title("recent"));
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
        None => muted(DIR_PICKER_HINT),
    });
    lines
}

fn draw_session_list(f: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;
    let mut items: Vec<ListItem> = Vec::new();
    // Claude-ID je Listenzeile (Überschriften: `None`) — für Mausklicks in die Liste.
    let mut rows: Vec<Option<String>> = Vec::new();
    let mut selected_item = None;
    for group in &app.groups {
        let header = tail(&group.cwd.display().to_string(), width.saturating_sub(4));
        // Das Ordner-Emoji ist zwei Spalten breit.
        let line = Line::from(vec![
            Span::raw(format!("{FOLDER_ICON} ")),
            Span::styled(header, Style::default().fg(theme::MUTED)),
        ]);
        items.push(ListItem::new(line));
        rows.push(None);
        for entry in &group.entries {
            let selected = app.cursor.as_deref() == Some(entry.claude_id.as_str());
            if selected {
                selected_item = Some(items.len());
            }
            items.push(entry_item(app, entry, selected, width));
            rows.push(Some(entry.claude_id.clone()));
        }
    }
    if app.loading() && items.is_empty() {
        items.push(ListItem::new(muted("loading sessions …")));
    }
    let mut state = ListState::default().with_selected(selected_item);
    f.render_stateful_widget(List::new(items), area, &mut state);

    // Sichtbare Zeilen merken (jede Listenzeile ist eine Bildschirmzeile).
    let hits = rows
        .into_iter()
        .skip(state.offset())
        .take(area.height as usize)
        .enumerate()
        .filter_map(|(i, id)| id.map(|id| (area.y + i as u16, id)))
        .collect();
    app.list_hits.replace(ListHits { area, rows: hits });
}

/// Eine Zeile: Zustandssymbol, Titel, Fensternummer, Zeit seit der letzten Aktivität.
fn entry_item(app: &App, entry: &Entry, selected: bool, width: usize) -> ListItem<'static> {
    let session = entry.running.and_then(|id| app.session(id));
    let (symbol, color) = if entry.external {
        theme::ELSEWHERE
    } else {
        theme::status_symbol(session.map(|s| s.status), clock())
    };
    let slot = entry.running.and_then(|id| app.windows.window_of(id));
    let window = match slot {
        Some(s) => format!(" [{}]", s + 1),
        None if entry.external => theme::ELSEWHERE_LABEL.to_string(),
        None => " ".repeat(4),
    };
    let since = match session {
        Some(s) => Some(s.last_output().elapsed()),
        None => entry
            .modified
            .map(|m| SystemTime::now().duration_since(m).unwrap_or_default()),
    };
    let time = since.map(short_age).unwrap_or_default();
    // Markierte Sessions (Leertaste) tragen ein Häkchen vor dem Titel.
    let marked = app.marked.contains(&entry.claude_id);
    let mark = if marked { "✓ " } else { "" };
    // Symbol + Leerzeichen, Häkchen, Fensterspalte, Leerzeichen + Zeit (max. 5 Zeichen).
    let title_width = width.saturating_sub(2 + mark.len().min(2) + 4 + 6);
    let title = head(&entry.title, title_width);
    let padding = " ".repeat(title_width.saturating_sub(title.chars().count()));

    // Hat die Session per `/color` eine Farbe, trägt der Titel sie; sonst hell (laufend/gewählt)
    // bzw. grau (gespeichert).
    let session_color = entry.color.as_deref().and_then(theme::session_color);
    let name_style = match session_color {
        Some(color) => Style::default().fg(color),
        None if session.is_some() || selected => Style::default().fg(theme::TEXT),
        None => Style::default().fg(theme::MUTED),
    };
    let line = Line::from(vec![
        Span::styled(format!("{symbol} "), Style::default().fg(color)),
        Span::styled(mark, Style::default().fg(theme::MENU)),
        Span::styled(format!("{title}{padding}"), name_style),
        Span::styled(
            window,
            Style::default().fg(if entry.external { color } else { theme::ACCENT }),
        ),
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
        0..=59 => "now".to_string(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

fn menu_lines(menu: &Menu, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (i, item) in menu.items.iter().enumerate() {
        let text = format!(" {:<3}{}", item.hotkey, item.label);
        lines.push(selectable(text, i == menu.selected, width));
    }
    lines.push(muted(MENU_FOOTER));
    lines
}

/// Mindestbreite des Inhalts der Verzeichnisauswahl, damit Platz zum Tippen bleibt.
const DIR_PICKER_MIN_WIDTH: usize = 40;

/// Menü, Fensterauswahl, Löschabfrage oder Verzeichnisauswahl als Overlay über dem fokussierten
/// Fenster; merkt sich die Fläche für die Maus.
fn draw_overlay(f: &mut Frame, app: &App) {
    let (title, rows, content_width) = match (&app.dir_picker, &app.menu) {
        (Some(picker), _) => (
            "Directory".to_string(),
            dir_picker_rows(picker),
            dir_picker_width(picker),
        ),
        (None, Some(menu)) => (menu.title.clone(), menu.items.len() + 1, menu_width(menu)),
        (None, None) => {
            app.overlay.set(None);
            return;
        }
    };
    let screen = f.area();
    let [sidebar, windows] = split(screen, app.list_hidden);
    // Fokus auf der Liste: mittig über der Sessionliste und auf sie begrenzt (kein Ausweichen).
    let (target, fallback) = match app.windows.focus() {
        Focus::Window(slot) => (app.window_rect(slot).unwrap_or(windows), windows),
        Focus::List => (sidebar, sidebar),
    };
    let target = target.intersection(screen);
    let fallback = fallback.intersection(screen);
    // Rahmen links/rechts; der Titel braucht zusätzlich je ein Leerzeichen davor und dahinter.
    let width = content_width.max(title.chars().count() + 2) + 2;
    let height = rows + 2;
    let area = overlay_rect(
        target,
        fallback,
        u16::try_from(width).unwrap_or(u16::MAX),
        u16::try_from(height).unwrap_or(u16::MAX),
    );
    let inner_width = area.width.saturating_sub(2) as usize;
    let lines = match (&app.dir_picker, &app.menu) {
        (Some(picker), _) => dir_picker_lines(picker, inner_width),
        (None, Some(menu)) => menu_lines(menu, inner_width),
        (None, None) => Vec::new(),
    };
    f.render_widget(Clear, area);
    draw_panel(f, &title, lines, area);
    app.overlay.set(Some(area));
}

/// Fläche des Overlays: `width` × `height`, zentriert in `target` und auf dessen Größe begrenzt.
/// Passt es nicht in `target` (zu kleines Fenster), weicht es auf `fallback` aus (die ganze
/// Fensterfläche).
fn overlay_rect(target: Rect, fallback: Rect, width: u16, height: u16) -> Rect {
    let fits = |r: Rect| r.width >= width && r.height >= height;
    let area = if fits(target) || target.area() >= fallback.area() {
        target
    } else {
        fallback
    };
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn menu_width(menu: &Menu) -> usize {
    menu.items
        .iter()
        .map(|item| 4 + item.label.chars().count())
        .chain([MENU_FOOTER.chars().count()])
        .max()
        .unwrap_or(0)
}

fn dir_picker_rows(picker: &DirPicker) -> usize {
    let recent = if picker.recent.is_empty() {
        0
    } else {
        1 + picker.recent.len()
    };
    2 + recent
}

fn dir_picker_width(picker: &DirPicker) -> usize {
    let input = picker.input.chars().count() + 3;
    let recent = picker
        .recent
        .iter()
        .map(|dir| dir.display().to_string().chars().count() + 1);
    let error = picker.error.map_or(0, |e| e.chars().count() + 1);
    recent
        .chain([input, error, DIR_PICKER_HINT.chars().count()])
        .max()
        .unwrap_or(0)
        .max(DIR_PICKER_MIN_WIDTH)
}

fn draw_windows(f: &mut Frame, app: &App) {
    let rects = app.window_rects();
    if rects.is_empty() {
        let [_, area] = split(f.area(), app.list_hidden);
        let hint = format!(" No window open – {} n starts a session", keys::PREFIX_HINT);
        let text = Paragraph::new(Span::styled(hint, Style::default().fg(theme::MUTED)));
        f.render_widget(text.block(rounded(theme::UNFOCUS)), area);
        return;
    }
    for (slot, id, rect) in rects {
        // Die Fensterrechtecke stammen aus der zuletzt gemeldeten Größe; ist das Terminal
        // inzwischen kleiner, nur den sichtbaren Teil zeichnen.
        let rect = rect.intersection(f.area());
        let Some(session) = app.session(id) else {
            continue;
        };
        if rect.is_empty() {
            continue;
        }
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
        let mut spans = vec![title, separator, status];
        if app.windows.zoomed() == Some(slot) {
            spans.push(Span::styled("· zoomed ", Style::default().fg(theme::MENU)));
        }
        let scrolled = session.scroll_offset();
        if scrolled > 0 {
            spans.push(Span::styled(
                format!("· ↑ {scrolled} lines "),
                Style::default().fg(theme::MENU),
            ));
        }
        let mut block = rounded(color).title(Line::from(spans));
        // Ohne Sessionliste fehlt deren Hinweiszeile; der Hinweis steht dann unten im Fenster
        // mit Fokus.
        if let Some(notice) = app.notice.as_deref().filter(|_| app.list_hidden && focused) {
            let width = (rect.width as usize).saturating_sub(4);
            block = block.title_bottom(Line::from(Span::styled(
                format!(" {} ", head(notice, width)),
                Style::default().fg(theme::WAITING),
            )));
        }
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        // Bis Claude etwas anzeigt, steht im Fenster, worauf es wartet — sonst wirkt es hängend.
        let waiting_text = if session.is_pending() {
            Some("queued – starts after the previous window".to_string())
        } else {
            session.starting_since().map(|since| {
                let (spinner, _) = theme::status_symbol(Some(session.status), clock());
                format!("{spinner} starting claude … {}s", since.elapsed().as_secs())
            })
        };
        if let Some(text) = waiting_text {
            let line = Line::from(Span::styled(text, Style::default().fg(theme::MUTED)));
            let middle = Rect {
                y: inner.y + inner.height / 2,
                height: inner.height.min(1),
                ..inner
            };
            f.render_widget(Paragraph::new(line).centered(), middle);
            continue;
        }
        let parser = session.screen();
        let selection = app
            .selection
            .filter(|s| s.session == id && !s.is_empty())
            .map(|s| s.range());
        f.render_widget(
            TermView::new(parser.screen())
                .show_cursor(focused)
                .selection(selection),
            inner,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS: Rect = Rect::new(42, 0, 100, 40);

    #[test]
    fn overlay_is_centered_in_target() {
        let target = Rect::new(42, 0, 50, 20);
        assert_eq!(
            overlay_rect(target, WINDOWS, 20, 10),
            Rect::new(57, 5, 20, 10)
        );
    }

    #[test]
    fn overlay_falls_back_to_windows_area_when_target_too_small() {
        let target = Rect::new(92, 20, 50, 8);
        assert_eq!(
            overlay_rect(target, WINDOWS, 30, 12),
            Rect::new(77, 14, 30, 12)
        );
    }

    #[test]
    fn overlay_is_limited_to_area() {
        assert_eq!(
            overlay_rect(WINDOWS, WINDOWS, 200, 60),
            WINDOWS,
            "Ziel ist schon die ganze Fläche"
        );
        let target = Rect::new(42, 0, 50, 20);
        assert_eq!(
            overlay_rect(target, WINDOWS, 120, 10),
            Rect::new(42, 15, 100, 10),
            "auch die Ausweichfläche ist zu schmal"
        );
    }
}
