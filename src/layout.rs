//! Geometrie der Fenster: Anordnung für 0–4 Fenster und Nachbarsuche für Alt+Pfeil.

use ratatui::layout::{Constraint, Layout, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Rechtecke der Fenster in Nummernreihenfolge (Index 0 = Fenster 1).
///
/// - 1: ganze Fläche
/// - 2: nebeneinander
/// - 3: Fenster 1 links, 2 und 3 rechts übereinander
/// - 4: 2 × 2 (1 oben links, 2 oben rechts, 3 unten links, 4 unten rechts)
pub fn windows(area: Rect, count: usize) -> Vec<Rect> {
    match count {
        0 => Vec::new(),
        1 => vec![area],
        2 => halves_horizontal(area).to_vec(),
        3 => {
            let [left, right] = halves_horizontal(area);
            let [top, bottom] = halves_vertical(right);
            vec![left, top, bottom]
        }
        _ => {
            let [top, bottom] = halves_vertical(area);
            let [a, b] = halves_horizontal(top);
            let [c, d] = halves_horizontal(bottom);
            vec![a, b, c, d]
        }
    }
}

fn halves_horizontal(area: Rect) -> [Rect; 2] {
    Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(area)
}

fn halves_vertical(area: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(area)
}

/// Nächstes Fenster in `direction` von `current` aus.
///
/// In Frage kommen Fenster, die vollständig in Pfeilrichtung liegen und sich quer dazu mit dem
/// aktuellen überlappen. Davon gewinnt das nächstgelegene, bei Gleichstand die kleinere Nummer.
pub fn neighbor(rects: &[Rect], current: usize, direction: Direction) -> Option<usize> {
    let cur = *rects.get(current)?;
    rects
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != current)
        .filter_map(|(i, r)| {
            let (distance, overlap) = match direction {
                Direction::Right if r.x >= cur.right() => (r.x - cur.right(), overlap_y(cur, *r)),
                Direction::Left if r.right() <= cur.x => (cur.x - r.right(), overlap_y(cur, *r)),
                Direction::Down if r.y >= cur.bottom() => (r.y - cur.bottom(), overlap_x(cur, *r)),
                Direction::Up if r.bottom() <= cur.y => (cur.y - r.bottom(), overlap_x(cur, *r)),
                _ => return None,
            };
            (overlap > 0).then_some((distance, i))
        })
        .min()
        .map(|(_, i)| i)
}

fn overlap_x(a: Rect, b: Rect) -> u16 {
    a.right().min(b.right()).saturating_sub(a.x.max(b.x))
}

fn overlap_y(a: Rect, b: Rect) -> u16 {
    a.bottom().min(b.bottom()).saturating_sub(a.y.max(b.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use Direction::*;

    const AREA: Rect = Rect::new(28, 0, 100, 41);

    #[test]
    fn layouts_cover_area_without_overlap() {
        for count in 1..=4 {
            let rects = windows(AREA, count);
            assert_eq!(rects.len(), count);
            let total: u32 = rects.iter().map(|r| r.area()).sum();
            assert_eq!(total, AREA.area(), "{count} Fenster");
        }
    }

    #[test]
    fn three_windows_have_one_large_left() {
        let r = windows(AREA, 3);
        assert_eq!(r[0].height, AREA.height);
        assert_eq!(r[1].x, r[2].x);
        assert!(r[1].y < r[2].y);
    }

    #[test]
    fn neighbors_with_two_windows() {
        let r = windows(AREA, 2);
        assert_eq!(neighbor(&r, 0, Right), Some(1));
        assert_eq!(neighbor(&r, 1, Left), Some(0));
        assert_eq!(neighbor(&r, 0, Left), None);
        assert_eq!(neighbor(&r, 0, Down), None);
    }

    #[test]
    fn neighbors_with_three_windows_prefer_lower_number() {
        let r = windows(AREA, 3);
        assert_eq!(neighbor(&r, 0, Right), Some(1));
        assert_eq!(neighbor(&r, 1, Left), Some(0));
        assert_eq!(neighbor(&r, 2, Left), Some(0));
        assert_eq!(neighbor(&r, 1, Down), Some(2));
        assert_eq!(neighbor(&r, 2, Up), Some(1));
        assert_eq!(neighbor(&r, 0, Up), None);
    }

    #[test]
    fn neighbors_with_four_windows() {
        let r = windows(AREA, 4);
        assert_eq!(neighbor(&r, 0, Right), Some(1));
        assert_eq!(neighbor(&r, 0, Down), Some(2));
        assert_eq!(neighbor(&r, 3, Left), Some(2));
        assert_eq!(neighbor(&r, 3, Up), Some(1));
        assert_eq!(neighbor(&r, 2, Left), None);
    }
}
