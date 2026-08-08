//! 컨트롤 갤러리 — 커스텀 컨트롤을 한 화면에서 눈으로 확인하는 **임시 데모**(사용자 요청 08-08).
//!
//! 새 컨트롤([`crate::controls`])을 각각 하나씩 배치해 포커스 링·도움말 "?"·창 활성 색·상호작용을
//! 실물로 확인한다. **제품 화면이 아니라 검수용** — 정식 UI 통합은 별도.

use crate::controls::{
    Checkbox, Combo, ComboItem, Control, ExtendedCombo, GridColumn, LabelSide, RadioGroup,
    RadioOption, TextBox, TreeGrid, TreeModel, TreeNode, TreeView,
};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::InputEvent;
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

const PAD: i32 = 16;
const LABEL_H: i32 = 22;
const GAP: i32 = 16;
const CTRL_H: i32 = 30;

/// 컨트롤 하나를 배치하고(라벨 공간 + 본체) 다음 y를 돌려준다.
#[allow(clippy::too_many_arguments)]
fn place(
    ctrl: &mut dyn Widget,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    label_h: i32,
    gap: i32,
    inv: &mut Invalidations,
) -> i32 {
    let cy = y + label_h;
    ctrl.set_bounds(Rect::new(x, cy, w, h), inv);
    cy + h + gap
}

/// 컨트롤 갤러리 위젯(검수용).
#[derive(Debug)]
pub struct GalleryWidget {
    bounds: Rect,
    scale: f32,
    scroll: i32,
    cb_right: Checkbox,
    cb_left: Checkbox,
    cb_only: Checkbox,
    radio: RadioGroup,
    textbox: TextBox,
    combo: Combo,
    ext: ExtendedCombo,
    tree: TreeView,
    grid: TreeGrid,
}

impl Default for GalleryWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl GalleryWidget {
    /// 데모 데이터로 채운 갤러리를 만든다.
    #[must_use]
    pub fn new() -> Self {
        let mut cb_right = Checkbox::new("Show warning before emptying the Trash", true);
        cb_right.set_help("A confirmation dialog appears before the Trash is emptied.");
        cb_right.set_show_help(true);

        let combo_items = vec![
            ComboItem::new("home", "Home"),
            ComboItem::new("recents", "Recents"),
            ComboItem::new("desktop", "Desktop"),
        ];
        let ext_items = vec![
            ComboItem::new("pathfinder", "Path Finder.app").with_icon("◎"),
            ComboItem::new("textedit", "TextEdit.app").with_icon("▤"),
            ComboItem::new("xcode", "Xcode.app").with_icon("◆"),
        ];

        let tree_model = TreeModel::new(vec![
            TreeNode::branch(
                "Path Finder",
                vec![
                    TreeNode::leaf("About Path Finder"),
                    TreeNode::branch("Trash", vec![TreeNode::leaf("Empty Trash")]),
                ],
            ),
            TreeNode::leaf("Show Desktop"),
        ]);
        let grid_model = TreeModel::new(vec![TreeNode::branch(
            "Path Finder",
            vec![
                TreeNode::leaf("About Path Finder").with_cells(vec![String::new()]),
                TreeNode::leaf("Settings…").with_cells(vec!["⌘,".into()]),
                TreeNode::leaf("Empty Trash").with_cells(vec!["⇧⌘⌫".into()]),
            ],
        )]);

        Self {
            bounds: Rect::default(),
            scale: 1.0,
            scroll: 0,
            cb_right,
            cb_left: Checkbox::new("Enable bug reporter", false).with_label_side(LabelSide::Left),
            cb_only: Checkbox::new("", true).with_label_side(LabelSide::None),
            radio: RadioGroup::new(
                vec![
                    RadioOption::new("icon", "Icon view"),
                    RadioOption::new("list", "List view"),
                    RadioOption::new("column", "Column view"),
                ],
                1,
            ),
            textbox: TextBox::new("Run command"),
            combo: Combo::new(combo_items, 0),
            ext: ExtendedCombo::new(ext_items, 0, "Choose…"),
            tree: TreeView::new(tree_model),
            grid: TreeGrid::new(
                grid_model,
                vec![GridColumn::new("Menu", 220), GridColumn::new("Command", 90)],
            ),
        }
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 배율 지정 — 전 컨트롤에 전파.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        let s = scale.max(0.5);
        self.scale = s;
        self.cb_right.set_scale(s);
        self.cb_left.set_scale(s);
        self.cb_only.set_scale(s);
        self.radio.set_scale(s);
        self.textbox.set_scale(s);
        self.combo.set_scale(s);
        self.ext.set_scale(s);
        self.tree.set_scale(s);
        self.grid.set_scale(s);
        self.relayout(inv);
    }

    /// 레이아웃 — 세로 스택. 각 항목: 라벨 + 컨트롤.
    fn relayout(&mut self, inv: &mut Invalidations) {
        let x = self.bounds.x + self.s(PAD);
        let w = (self.bounds.w - self.s(PAD) * 2).clamp(self.s(120), self.s(360));
        let label_h = self.s(LABEL_H);
        let gap = self.s(GAP);
        let ctrl_h = self.s(CTRL_H);
        let (radio_h, tree_h, grid_h) = (self.s(26 * 3), self.s(24 * 5), self.s(26 + 24 * 4));
        let mut y = self.bounds.y + self.s(PAD) - self.scroll;
        y = place(&mut self.cb_right, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.cb_left, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.cb_only, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.radio, x, y, w, radio_h, label_h, gap, inv);
        y = place(&mut self.textbox, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.combo, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.ext, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.tree, x, y, w, tree_h, label_h, gap, inv);
        let _ = place(&mut self.grid, x, y, w, grid_h, label_h, gap, inv);
        inv.push(self.bounds);
    }

    fn labels() -> [&'static str; 9] {
        [
            "Checkbox — 라벨 오른쪽 (+ 도움말 ?)",
            "Checkbox — 라벨 왼쪽",
            "Checkbox — 체크만",
            "RadioGroup (옵션 박스)",
            "TextBox — placeholder",
            "Combo — 일반 (∨)",
            "ExtendedCombo — 확장 (⇕ · Choose…)",
            "TreeView — 계층",
            "TreeGrid — 그리드 + 트리",
        ]
    }
}

impl Widget for GalleryWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        self.relayout(inv);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // 클릭 시 포커스를 해당 컨트롤로 옮긴다(포커스 링 데모).
        if let InputEvent::MouseDown { x, y, .. } = *ev {
            let p = Point { x, y };
            self.cb_right
                .set_focused(self.cb_right.bounds().contains(p));
            self.cb_left.set_focused(self.cb_left.bounds().contains(p));
            self.cb_only.set_focused(self.cb_only.bounds().contains(p));
            self.radio.set_focused(self.radio.bounds().contains(p));
            self.textbox.set_focused(self.textbox.bounds().contains(p));
            self.combo.set_focused(self.combo.bounds().contains(p));
            self.ext.set_focused(self.ext.bounds().contains(p));
            self.tree.set_focused(self.tree.bounds().contains(p));
            self.grid.set_focused(self.grid.bounds().contains(p));
        }
        // 이벤트를 전 컨트롤에 전달(각자 bounds/포커스로 자기 것만 처리).
        self.cb_right.on_event(ev, inv);
        self.cb_left.on_event(ev, inv);
        self.cb_only.on_event(ev, inv);
        self.radio.on_event(ev, inv);
        self.textbox.on_event(ev, inv);
        self.combo.on_event(ev, inv);
        self.ext.on_event(ev, inv);
        self.tree.on_event(ev, inv);
        self.grid.on_event(ev, inv);
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        ctx.fill_rect(self.bounds, theme.panel_bg);
        let labels = Self::labels();
        let widgets: [&dyn Widget; 9] = [
            &self.cb_right,
            &self.cb_left,
            &self.cb_only,
            &self.radio,
            &self.textbox,
            &self.combo,
            &self.ext,
            &self.tree,
            &self.grid,
        ];
        // 섹션 라벨(각 컨트롤 위).
        ctx.select_font(FontSlot::Status, false);
        for (label, w) in labels.iter().zip(widgets.iter()) {
            let b = w.bounds();
            let lr = Rect::new(
                self.bounds.x + self.s(PAD),
                b.y - self.s(LABEL_H),
                self.bounds.w,
                self.s(LABEL_H),
            );
            ctx.text(lr.x, lr.y + self.s(2), lr, label, theme.text_dim);
        }
        // 컨트롤(콤보 드롭다운이 아래를 덮도록 순서대로 — 마지막에 그려도 무방).
        for w in widgets {
            w.paint(ctx, theme);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gallery() -> (GalleryWidget, Invalidations) {
        let mut g = GalleryWidget::new();
        let mut inv = Invalidations::default();
        g.set_bounds(Rect::new(0, 0, 420, 900), &mut inv);
        (g, inv)
    }
    fn click(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }

    #[test]
    fn lays_out_all_controls_within_columns() {
        let (g, _) = gallery();
        // 첫 체크박스는 상단, 그리드는 하단.
        assert!(g.cb_right.bounds().y < g.grid.bounds().y);
        assert!(g.cb_right.bounds().w > 0);
    }

    #[test]
    fn click_moves_focus_to_hit_control() {
        let (mut g, mut inv) = gallery();
        let tb = g.textbox.bounds();
        g.on_event(&click(tb.x + 4, tb.y + 4), &mut inv);
        assert!(g.textbox.is_focused(), "텍스트박스로 포커스 이동");
        assert!(!g.combo.is_focused());
    }

    #[test]
    fn checkbox_toggles_through_gallery() {
        let (mut g, mut inv) = gallery();
        let cb = g.cb_left.bounds();
        let before = g.cb_left.is_checked();
        g.on_event(&click(cb.x + 4, cb.y + 4), &mut inv);
        assert_ne!(g.cb_left.is_checked(), before, "갤러리 경유 토글");
    }
}
