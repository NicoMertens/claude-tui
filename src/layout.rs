//! Geometrie der Fenster: Anordnung der festen Fensterplätze 1–4 und Nachbarsuche für Ctrl+Pfeil.

use ratatui::layout::{Constraint, Layout, Margin, Rect};

use crate::windows::MAX_WINDOWS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Fensterplätze je Spalte: links 1 und 3, rechts 2 und 4.
const COLUMNS: [[usize; 2]; 2] = [[0, 2], [1, 3]];

/// Rechtecke der offenen Fenster als (Fensterindex, Rechteck) in Nummernreihenfolge.
///
/// Jedes Fenster hat einen festen Platz im 2 × 2-Raster (1 oben links, 2 oben rechts, 3 unten
/// links, 4 unten rechts). Fehlende Fenster lassen keine Lücke: Ist eine Spalte leer, nimmt die
/// andere die ganze Breite; ein Fenster ohne Partner in seiner Spalte die ganze Höhe.
pub fn windows(area: Rect, occupied: [bool; MAX_WINDOWS]) -> Vec<(usize, Rect)> {
    let columns: Vec<Vec<usize>> = COLUMNS
        .iter()
        .map(|slots| slots.iter().copied().filter(|&i| occupied[i]).collect())
        .filter(|slots: &Vec<usize>| !slots.is_empty())
        .collect();
    let column_areas = match columns.len() {
        2 => halves_horizontal(area).to_vec(),
        _ => vec![area],
    };
    let mut rects: Vec<(usize, Rect)> = columns
        .iter()
        .zip(column_areas)
        .flat_map(|(slots, column)| {
            let parts = match slots.len() {
                2 => halves_vertical(column).to_vec(),
                _ => vec![column],
            };
            slots.iter().copied().zip(parts).collect::<Vec<_>>()
        })
        .collect();
    rects.sort_by_key(|&(slot, _)| slot);
    rects
}

/// Sichtbare Fenster als (Fensterindex, Rechteck). Gezoomt bekommt nur `zoomed` die ganze
/// Fläche, die übrigen Fenster fehlen.
pub fn visible(
    area: Rect,
    occupied: [bool; MAX_WINDOWS],
    zoomed: Option<usize>,
) -> Vec<(usize, Rect)> {
    match zoomed {
        Some(slot) if occupied.get(slot) == Some(&true) => vec![(slot, area)],
        _ => windows(area, occupied),
    }
}

/// Höchstens so viele Notizzeilen stehen unten im Fenster.
pub const NOTE_MAX_ROWS: usize = 5;

/// Innenfläche eines Fensters: der Teil für das Terminal der Session und darunter `note_lines`
/// Notizzeilen — höchstens [`NOTE_MAX_ROWS`] und höchstens ein Drittel der Höhe, damit Claude
/// genug Platz behält. Bleibt keine Zeile übrig, entfällt die Notiz.
pub fn window_content(rect: Rect, note_lines: usize) -> (Rect, Option<Rect>) {
    let inner = rect.inner(Margin::new(1, 1));
    let rows = note_lines
        .min(NOTE_MAX_ROWS)
        .min(usize::from(inner.height) / 3);
    let rows = u16::try_from(rows).unwrap_or(0);
    if rows == 0 {
        return (inner, None);
    }
    let term = Rect {
        height: inner.height - rows,
        ..inner
    };
    let note = Rect {
        y: term.bottom(),
        height: rows,
        ..inner
    };
    (term, Some(note))
}

fn halves_horizontal(area: Rect) -> [Rect; 2] {
    Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(area)
}

fn halves_vertical(area: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(area)
}

/// Nächstes Fenster in `direction` vom Fenster `current` aus; `rects` wie aus [`windows`].
///
/// In Frage kommen Fenster, die vollständig in Pfeilrichtung liegen und sich quer dazu mit dem
/// aktuellen überlappen. Davon gewinnt das nächstgelegene, bei Gleichstand die kleinere Nummer.
pub fn neighbor(rects: &[(usize, Rect)], current: usize, direction: Direction) -> Option<usize> {
    let cur = rects.iter().find(|&&(i, _)| i == current)?.1;
    rects
        .iter()
        .filter(|&&(i, _)| i != current)
        .filter_map(|&(i, r)| {
            let (distance, overlap) = match direction {
                Direction::Right if r.x >= cur.right() => (r.x - cur.right(), overlap_y(cur, r)),
                Direction::Left if r.right() <= cur.x => (cur.x - r.right(), overlap_y(cur, r)),
                Direction::Down if r.y >= cur.bottom() => (r.y - cur.bottom(), overlap_x(cur, r)),
                Direction::Up if r.bottom() <= cur.y => (cur.y - r.bottom(), overlap_x(cur, r)),
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

    fn open(slots: &[usize]) -> [bool; MAX_WINDOWS] {
        let mut occupied = [false; MAX_WINDOWS];
        for &i in slots {
            occupied[i] = true;
        }
        occupied
    }

    /// Rechteck je Fensterplatz; `None` für nicht offene Fenster.
    fn layout(slots: &[usize]) -> [Option<Rect>; MAX_WINDOWS] {
        let mut out = [None; MAX_WINDOWS];
        for (i, r) in windows(AREA, open(slots)) {
            out[i] = Some(r);
        }
        out
    }

    #[test]
    fn every_combination_covers_area_without_overlap() {
        for bits in 1u8..16 {
            let occupied: [bool; MAX_WINDOWS] = std::array::from_fn(|i| bits & (1 << i) != 0);
            let rects = windows(AREA, occupied);
            let slots: Vec<usize> = rects.iter().map(|&(i, _)| i).collect();
            let expected: Vec<usize> = (0..MAX_WINDOWS).filter(|&i| occupied[i]).collect();
            assert_eq!(slots, expected, "{occupied:?}");
            let total: u32 = rects.iter().map(|(_, r)| r.area()).sum();
            assert_eq!(total, AREA.area(), "{occupied:?}");
            for (a, (_, ra)) in rects.iter().enumerate() {
                for (_, rb) in &rects[a + 1..] {
                    assert!(ra.intersection(*rb).is_empty(), "{occupied:?}");
                }
            }
        }
        assert!(windows(AREA, [false; MAX_WINDOWS]).is_empty());
    }

    #[test]
    fn single_window_gets_whole_area_whatever_its_number() {
        for slot in 0..MAX_WINDOWS {
            assert_eq!(windows(AREA, open(&[slot])), [(slot, AREA)]);
        }
    }

    #[test]
    fn one_and_two_side_by_side() {
        let [Some(a), Some(b), None, None] = layout(&[0, 1]) else {
            panic!("1 und 2");
        };
        assert_eq!(a.height, AREA.height);
        assert_eq!(a.right(), b.x);
    }

    #[test]
    fn three_goes_below_one_and_two_spans_the_height() {
        let [Some(a), Some(b), Some(c), None] = layout(&[0, 1, 2]) else {
            panic!("1, 2 und 3");
        };
        assert_eq!(b.height, AREA.height);
        assert_eq!((a.x, a.bottom()), (c.x, c.y));
        assert!(a.right() <= b.x);
    }

    #[test]
    fn four_without_three_goes_bottom_right() {
        let [Some(a), Some(b), None, Some(d)] = layout(&[0, 1, 3]) else {
            panic!("1, 2 und 4");
        };
        assert_eq!(a.height, AREA.height);
        assert_eq!((b.x, b.bottom()), (d.x, d.y));
    }

    #[test]
    fn one_and_three_stacked_full_width() {
        let [Some(a), None, Some(c), None] = layout(&[0, 2]) else {
            panic!("1 und 3");
        };
        assert_eq!(a.width, AREA.width);
        assert_eq!(a.bottom(), c.y);
    }

    #[test]
    fn four_windows_form_a_grid() {
        let [Some(a), Some(b), Some(c), Some(d)] = layout(&[0, 1, 2, 3]) else {
            panic!("alle vier");
        };
        assert_eq!((a.y, a.right()), (b.y, b.x));
        assert_eq!((c.y, c.right()), (d.y, d.x));
        assert_eq!(a.bottom(), c.y);
    }

    #[test]
    fn zoomed_window_alone_gets_whole_area() {
        let occupied = open(&[0, 1, 2]);
        assert_eq!(visible(AREA, occupied, Some(1)), [(1, AREA)]);
        assert_eq!(visible(AREA, occupied, None), windows(AREA, occupied));
        assert_eq!(visible(AREA, occupied, Some(3)).len(), 3, "nicht offen");
    }

    #[test]
    fn note_takes_the_last_inner_rows() {
        let rect = Rect::new(10, 5, 30, 12);
        let inner = Rect::new(11, 6, 28, 10);
        assert_eq!(window_content(rect, 0), (inner, None));
        let (term, note) = window_content(rect, 2);
        assert_eq!(term, Rect::new(11, 6, 28, 8));
        assert_eq!(note, Some(Rect::new(11, 14, 28, 2)));
        let height = |rect, lines| window_content(rect, lines).1.map(|r| r.height);
        assert_eq!(height(rect, 9), Some(3), "höchstens ein Drittel");
        let tall = Rect::new(0, 0, 30, 50);
        assert_eq!(height(tall, 9), Some(NOTE_MAX_ROWS as u16));
        assert_eq!(height(Rect::new(0, 0, 30, 4), 1), None, "zu niedrig");
    }

    #[test]
    fn neighbors_with_two_windows() {
        let r = windows(AREA, open(&[0, 1]));
        assert_eq!(neighbor(&r, 0, Right), Some(1));
        assert_eq!(neighbor(&r, 1, Left), Some(0));
        assert_eq!(neighbor(&r, 0, Left), None);
        assert_eq!(neighbor(&r, 0, Down), None);
    }

    #[test]
    fn neighbors_with_one_and_three() {
        let r = windows(AREA, open(&[0, 2]));
        assert_eq!(neighbor(&r, 0, Down), Some(2));
        assert_eq!(neighbor(&r, 2, Up), Some(0));
        // Beide liegen ganz links: Ctrl+← führt aus jedem in die Sessionliste.
        assert_eq!(neighbor(&r, 2, Left), None);
    }

    #[test]
    fn neighbors_with_three_windows() {
        let r = windows(AREA, open(&[0, 1, 2]));
        assert_eq!(
            neighbor(&r, 1, Left),
            Some(0),
            "Gleichstand: kleinere Nummer"
        );
        assert_eq!(neighbor(&r, 0, Right), Some(1));
        assert_eq!(neighbor(&r, 2, Right), Some(1));
        assert_eq!(neighbor(&r, 0, Down), Some(2));
        assert_eq!(neighbor(&r, 2, Up), Some(0));
        assert_eq!(neighbor(&r, 1, Down), None);
    }

    #[test]
    fn neighbors_with_four_windows() {
        let r = windows(AREA, open(&[0, 1, 2, 3]));
        assert_eq!(neighbor(&r, 0, Right), Some(1));
        assert_eq!(neighbor(&r, 0, Down), Some(2));
        assert_eq!(neighbor(&r, 3, Left), Some(2));
        assert_eq!(neighbor(&r, 3, Up), Some(1));
        assert_eq!(neighbor(&r, 2, Left), None);
    }
}
