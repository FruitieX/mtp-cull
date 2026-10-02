//! Reel selection is separate from review decisions and import choices.
use std::collections::BTreeSet;

pub const MIN_THUMBNAIL_WIDTH: f32 = 100.0;
pub const MAX_THUMBNAIL_WIDTH: f32 = 390.0;

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
    /// Choose the closest thumbnail width, then spread columns across the viewport.
    /// A complete row has only the usual inter-card gaps and no unused right margin.
    pub fn fitted_grid(width: f32, target: f32, count: usize) -> Self {
        let width = width.max(1.0);
        let target = target.clamp(MIN_THUMBNAIL_WIDTH, MAX_THUMBNAIL_WIDTH);
        let ideal = (width + 8.0) / (target + 8.0);
        let fewer = ideal.floor().max(1.0) as usize;
        let more = fewer + 1;
        let card_width = |columns: usize| (width + 8.0) / columns as f32 - 8.0;
        let columns = if (card_width(more) - target).abs() < (card_width(fewer) - target).abs() {
            more
        } else {
            fewer
        };
        let card_width = card_width(columns);
        Self {
            columns,
            cell: eframe::egui::vec2(card_width + 8.0, card_width / 1.5 + 48.0),
            count,
        }
    }
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
    fn clamp_offset(&self, offset: f32, extent: f32, vertical: bool) -> f32 {
        let length = if vertical {
            self.count.div_ceil(self.columns) as f32 * self.cell.y
        } else {
            self.count as f32 * self.cell.x
        };
        offset.clamp(0.0, (length - extent).max(0.0))
    }
    /// Keep a visible photo at the same viewport position during reflow.
    /// If the active photo is offscreen, preserve the leading visible photo instead.
    pub fn reflow_offset(
        &self,
        previous: &Self,
        viewport: eframe::egui::Rect,
        size: eframe::egui::Vec2,
        active: Option<usize>,
        vertical: bool,
    ) -> eframe::egui::Vec2 {
        use eframe::egui::Vec2;
        if self.count == 0 || previous.count == 0 {
            return Vec2::ZERO;
        }
        let axis = usize::from(vertical);
        let leading = if vertical {
            (viewport.top() / previous.cell.y).floor().max(0.0) as usize * previous.columns
        } else {
            (viewport.left() / previous.cell.x).floor().max(0.0) as usize
        };
        let active =
            active.filter(|i| *i < previous.count && previous.rect(*i).intersects(viewport));
        let index = active
            .unwrap_or(leading)
            .min(self.count.min(previous.count) - 1);
        let before = previous.rect(index);
        let anchor = if active.is_some() {
            before.center()[axis]
        } else {
            before.intersect(viewport).center()[axis]
        };
        let within_photo = ((anchor - before.min[axis]) / before.size()[axis]).clamp(0.0, 1.0);
        let within_view =
            ((anchor - viewport.min[axis]) / viewport.size()[axis].max(1.0)).clamp(0.0, 1.0);
        let after = self.rect(index);
        let offset = after.min[axis] + within_photo * after.size()[axis] - within_view * size[axis];
        let mut result = Vec2::ZERO;
        result[axis] = self.clamp_offset(offset, size[axis], vertical);
        result
    }
    /// Resolve keyboard-follow before painting, including photos larger than the view.
    pub fn follow_offset(
        &self,
        offset: eframe::egui::Vec2,
        size: eframe::egui::Vec2,
        index: usize,
        vertical: bool,
    ) -> eframe::egui::Vec2 {
        let axis = usize::from(vertical);
        let rect = self.rect(index).expand(2.0);
        let start = offset[axis];
        let next = if rect.size()[axis] > size[axis] {
            rect.center()[axis] - size[axis] * 0.5
        } else if rect.min[axis] < start {
            rect.min[axis]
        } else if rect.max[axis] > start + size[axis] {
            rect.max[axis] - size[axis]
        } else {
            start
        };
        let mut result = eframe::egui::Vec2::ZERO;
        result[axis] = self.clamp_offset(next, size[axis], vertical);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reflow_keeps_the_active_photo_anchored_in_strips_and_grids() {
        use eframe::egui::{Pos2, Rect, vec2};
        for vertical in [false, true] {
            let mut previous = Layout {
                columns: if vertical { 1 } else { 500 },
                cell: vec2(208.0, 188.0),
                count: 500,
            };
            let axis = usize::from(vertical);
            let mut size = vec2(1000.0, 600.0);
            let mut offset = vec2(0.0, 0.0);
            offset[axis] = previous.rect(200).center()[axis] - size[axis] * 0.65;
            for step in (0..60).chain((0..60).rev()) {
                let width = 200.0 + step as f32 * 4.0;
                let next = Layout {
                    columns: previous.columns,
                    cell: vec2(width + 8.0, width / 1.5 + 48.0),
                    count: 500,
                };
                let next_size = vec2(1000.0, 500.0 + step as f32);
                offset = next.reflow_offset(
                    &previous,
                    Rect::from_min_size(Pos2::ZERO + offset, size),
                    next_size,
                    Some(200),
                    vertical,
                );
                let screen = next.rect(200).center()[axis] - offset[axis];
                assert!(
                    (screen - next_size[axis] * 0.65).abs() < 1.0,
                    "vertical={vertical}, step={step}, anchor drift={}",
                    screen - next_size[axis] * 0.65
                );
                assert!(
                    next.range(
                        Rect::from_min_size(Pos2::ZERO + offset, next_size),
                        vertical
                    )
                    .contains(&200)
                );
                previous = next;
                size = next_size;
            }
        }
        let mut previous = Layout::fitted_grid(352.0, 280.0, 500);
        let mut size = vec2(352.0, 600.0);
        let mut offset = vec2(0.0, previous.rect(200).center().y - 360.0);
        for width in [432.0, 272.0, 620.0, 700.0, 1000.0, 290.0] {
            let next = Layout::fitted_grid(width, 280.0, 500);
            let next_size = vec2(width, 550.0);
            offset = next.reflow_offset(
                &previous,
                Rect::from_min_size(Pos2::ZERO + offset, size),
                next_size,
                Some(200),
                true,
            );
            assert!((next.rect(200).center().y - offset.y - next_size.y * 0.6).abs() < 0.03);
            previous = next;
            size = next_size;
        }
    }
    #[test]
    fn reflow_preserves_manual_scrolling_and_keyboard_follow_handles_clipped_photos() {
        use eframe::egui::{Pos2, Rect, pos2, vec2};
        let previous = Layout {
            columns: 500,
            cell: vec2(200.0, 100.0),
            count: 500,
        };
        let next = Layout {
            cell: vec2(400.0, 220.0),
            ..previous
        };
        let size = vec2(1000.0, 200.0);
        let view = Rect::from_min_size(pos2(5000.0, 0.0), size);
        let offset = next.reflow_offset(&previous, view, size, Some(200), false);
        let resized = Rect::from_min_size(Pos2::ZERO + offset, size);
        assert!(next.range(resized, false).contains(&25));
        assert!(
            !next.range(resized, false).contains(&200),
            "resize must not undo manual scrolling"
        );
        let grid = Layout {
            columns: 1,
            cell: vec2(300.0, 350.0),
            count: 500,
        };
        let size = vec2(300.0, 140.0);
        let offset = grid.follow_offset(vec2(0.0, 0.0), size, 200, true);
        assert!((grid.rect(200).center().y - offset.y - 70.0).abs() < 0.01);
        assert_eq!(grid.follow_offset(offset, size, 0, true).y, 101.0);
    }
    #[test]
    fn fitted_side_grids_fill_the_viewport_and_keep_the_nearest_thumbnail_size() {
        let grid = Layout::fitted_grid(352.0, 160.0, 500);
        assert_eq!(grid.columns, 2);
        assert_eq!(grid.rect(0).width(), 172.0);
        assert_eq!(grid.rect(1).right(), 352.0);
        // A narrow sidebar fits two previews closer to the target than one huge one.
        assert_eq!(Layout::fitted_grid(272.0, 160.0, 500).columns, 2);
        // Increasing the preference changes density, while preserving full width.
        assert_eq!(Layout::fitted_grid(352.0, 100.0, 500).columns, 3);
        assert_eq!(Layout::fitted_grid(352.0, 300.0, 500).columns, 1);
        assert_eq!(Layout::fitted_grid(480.0, 300.0, 500).columns, 2);
        assert_eq!(
            Layout::fitted_grid(480.0, MAX_THUMBNAIL_WIDTH, 500).columns,
            1
        );
        for width in [232.0, 272.0, 352.0, 520.0, 1000.0] {
            for target in [100.0, 160.0, 220.0, 300.0, 390.0] {
                let grid = Layout::fitted_grid(width, target, 500);
                assert!((grid.rect(grid.columns - 1).right() - width).abs() < 0.001);
                let difference = (grid.rect(0).width() - target).abs();
                for columns in 1..=12 {
                    let alternative = (width - (columns - 1) as f32 * 8.0) / columns as f32;
                    assert!(difference <= (alternative - target).abs() + 0.001);
                }
                assert!(grid.range(grid.rect(100), true).contains(&100));
            }
        }
    }
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
