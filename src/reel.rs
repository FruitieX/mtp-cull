//! Reel selection is separate from review decisions and import choices.
use std::collections::BTreeSet;

#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Position {
    #[default]
    Bottom,
    Left,
    Right,
}
impl Position {
    pub const ALL: [Self; 3] = [Self::Bottom, Self::Left, Self::Right];
    pub fn label(self) -> &'static str {
        match self {
            Self::Bottom => "Bottom",
            Self::Left => "Left",
            Self::Right => "Right",
        }
    }
    pub fn is_side(self) -> bool {
        self != Self::Bottom
    }
    pub fn next(self) -> Self {
        match self {
            Self::Bottom => Self::Left,
            Self::Left => Self::Right,
            Self::Right => Self::Bottom,
        }
    }
}

#[derive(Default)]
pub struct Selection {
    pub indices: BTreeSet<usize>,
    anchor: Option<usize>,
}
impl Selection {
    pub fn single(&mut self, index: usize) {
        self.indices.clear();
        self.indices.insert(index);
        self.anchor = Some(index);
    }
    pub fn click(&mut self, index: usize, visible: &[usize], toggle: bool, range: bool) {
        if range {
            let start = self
                .anchor
                .and_then(|a| visible.iter().position(|i| *i == a));
            let end = visible.iter().position(|i| *i == index);
            if let (Some(start), Some(end)) = (start, end) {
                if !toggle {
                    self.indices.clear();
                }
                self.indices
                    .extend(&visible[start.min(end)..=start.max(end)]);
                return;
            }
        }
        if toggle {
            if !self.indices.remove(&index) {
                self.indices.insert(index);
            }
            self.anchor = Some(index);
        } else {
            self.single(index);
        }
    }
    pub fn retain_visible(&mut self, visible: &[usize]) {
        self.indices.retain(|i| visible.contains(i));
        if self.anchor.is_some_and(|i| !visible.contains(&i)) {
            self.anchor = None;
        }
    }
}

/// Fixed-size cells keep rendering proportional to the visible rows, not session size.
pub struct Layout {
    pub columns: usize,
    pub cell: eframe::egui::Vec2,
    pub count: usize,
}
impl Layout {
    pub fn rect(&self, index: usize) -> eframe::egui::Rect {
        eframe::egui::Rect::from_min_size(
            eframe::egui::pos2(
                (index % self.columns) as f32 * self.cell.x,
                (index / self.columns) as f32 * self.cell.y,
            ),
            self.cell - eframe::egui::vec2(8.0, 8.0),
        )
    }
    pub fn range(&self, viewport: eframe::egui::Rect, vertical: bool) -> std::ops::Range<usize> {
        if vertical {
            let first = (viewport.top() / self.cell.y).floor().max(0.0) as usize * self.columns;
            let end = ((viewport.bottom() / self.cell.y).ceil() as usize + 1) * self.columns;
            first.min(self.count)..end.min(self.count)
        } else {
            let first = (viewport.left() / self.cell.x).floor().max(0.0) as usize;
            let end = (viewport.right() / self.cell.x).ceil() as usize + 1;
            first.min(self.count)..end.min(self.count)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_toggles_ranges_and_prunes_hidden_images() {
        let mut selection = Selection::default();
        let visible = [0, 2, 4, 6, 8];
        selection.single(2);
        selection.click(6, &visible, true, false);
        assert_eq!(selection.indices, BTreeSet::from([2, 6]));
        selection.click(6, &visible, true, false);
        assert_eq!(selection.indices, BTreeSet::from([2]));
        selection.single(2);
        selection.click(8, &visible, false, true);
        assert_eq!(selection.indices, BTreeSet::from([2, 4, 6, 8]));
        selection.retain_visible(&[0, 4]);
        assert_eq!(selection.indices, BTreeSet::from([4]));
        selection.single(0);
        assert_eq!(selection.indices, BTreeSet::from([0]));
    }
    #[test]
    fn virtualized_grid_and_row_cover_scrolled_targets() {
        use eframe::egui::{Rect, pos2, vec2};
        let grid = Layout {
            columns: 4,
            cell: vec2(120.0, 100.0),
            count: 19,
        };
        assert_eq!(grid.rect(18).min, pos2(240.0, 400.0));
        assert_eq!(
            grid.range(
                Rect::from_min_size(pos2(0.0, 200.0), vec2(480.0, 100.0)),
                true
            ),
            8..16
        );
        let row = Layout {
            columns: 19,
            ..grid
        };
        assert_eq!(
            row.range(
                Rect::from_min_size(pos2(600.0, 0.0), vec2(240.0, 100.0)),
                false
            ),
            5..8
        );
    }
    #[test]
    fn side_strip_virtualizes_one_column_in_photo_order() {
        use eframe::egui::{Rect, pos2, vec2};
        let strip = Layout {
            columns: 1,
            cell: vec2(240.0, 200.0),
            count: 500,
        };
        assert_eq!(strip.rect(80).min, pos2(0.0, 16000.0));
        assert_eq!(
            strip.range(
                Rect::from_min_size(pos2(0.0, 16000.0), vec2(240.0, 600.0)),
                true
            ),
            80..84
        );
        assert_eq!(Position::Bottom.next(), Position::Left);
        assert_eq!(Position::Left.next(), Position::Right);
        assert_eq!(Position::Right.next(), Position::Bottom);
    }
}
