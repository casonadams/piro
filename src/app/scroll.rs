#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Input,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionType {
    #[default]
    Char,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct DocPos {
    pub line: usize,
    pub col: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct ScrollBounds {
    pub total: usize,
    pub view: usize,
}

#[derive(Debug, Default, Clone)]
pub struct ScrollState {
    pub scroll_from_bottom: usize,
    pub auto_scroll: bool,
}

impl ScrollState {
    pub fn scroll_up(&mut self, delta: usize, bounds: ScrollBounds) {
        if bounds.total > bounds.view {
            self.scroll_from_bottom = (self.scroll_from_bottom + delta).min(bounds.total - bounds.view);
            self.auto_scroll = false;
        }
    }

    pub fn scroll_down(&mut self, delta: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(delta);
        if self.scroll_from_bottom == 0 {
            self.auto_scroll = true;
        }
    }

    pub fn visible_range(&self, total: usize, view: usize) -> (usize, usize) {
        if total <= view {
            return (0, total);
        }
        let max = total - view;
        let offset = if self.auto_scroll {
            0
        } else {
            self.scroll_from_bottom.min(max)
        };
        let start = max - offset;
        (start, (start + view).min(total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_state_visible_range() {
        let mut scroll = ScrollState {
            scroll_from_bottom: 0,
            auto_scroll: true,
        };
        assert_eq!(scroll.visible_range(100, 20), (80, 100));

        scroll.scroll_up(10, ScrollBounds { total: 100, view: 20 });
        assert_eq!(scroll.visible_range(100, 20), (70, 90));

        scroll.scroll_down(10);
        assert_eq!(scroll.visible_range(100, 20), (80, 100));
        assert!(scroll.auto_scroll);
    }
}
