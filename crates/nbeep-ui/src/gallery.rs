//! 컨트롤 갤러리 — 커스텀 컨트롤을 한 화면에서 눈으로 확인하는 **임시 데모**(사용자 요청 08-08).
//!
//! 새 컨트롤([`crate::controls`])을 각각 하나씩 배치해 포커스 링·도움말 "?"·창 활성 색·상호작용을
//! 실물로 확인한다. **제품 화면이 아니라 검수용** — 정식 UI 통합은 별도.

use crate::controls::{
    Checkbox, Choose, ChoosePicker, Combo, ComboControl, ComboItem, Control, GridColumn, LabelSide,
    RadioGroup, RadioOption, ScrollBars, TextBox, TreeGrid, TreeModel, TreeNode, TreeView,
};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::InputEvent;
use crate::geom::{Point, Rect};
use crate::theme::{IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use std::rc::Rc;

const PAD: i32 = 16;
const LABEL_H: i32 = 22;
const GAP: i32 = 16;
/// 입력·콤보 기본 높이(논리 px) — 트리 행 수준으로 축소(구 30 → 26 · 사용자 확정).
const CTRL_H: i32 = 26;
/// 콘텐츠 자연 폭(논리 px) — 창이 좁으면 이 폭까지 좌우 스크롤로 드러난다.
const NAT_W: i32 = 360;

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
    /// 세로 스크롤 오프셋(물리 px).
    scroll: i32,
    /// 가로 스크롤 오프셋(물리 px).
    scroll_x: i32,
    /// 콘텐츠 총 높이·폭(물리 px · 스크롤 클램프 근거).
    content_h: i32,
    content_w: i32,
    /// 오버레이 스크롤바(세로+가로).
    bars: ScrollBars,
    cb_right: Checkbox,
    cb_left: Checkbox,
    cb_only: Checkbox,
    radio: RadioGroup,
    textbox: TextBox,
    combo: Combo,
    ext: Choose,
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

        // 투명 배경 이미지 아이콘(글리프가 아니라 실제 RGBA 이미지 · 색상별 스와치).
        let img = |rgb| Rc::new(IconImage::swatch(16, rgb));
        let combo_items = vec![
            ComboItem::new("home", "Home").with_image(img((0x3D, 0x8B, 0xFF))),
            ComboItem::new("recents", "Recents").with_image(img((0x2E, 0xA0, 0x43))),
            ComboItem::new("desktop", "Desktop").with_image(img((0xB5, 0x7C, 0x1E))),
        ];
        let ext_items = vec![
            ComboItem::new("pathfinder", "Path Finder.app").with_image(img((0x3D, 0x8B, 0xFF))),
            ComboItem::new("textedit", "TextEdit.app").with_image(img((0x8A, 0x91, 0x9C))),
            ComboItem::new("xcode", "Xcode.app").with_image(img((0x5B, 0x6C, 0xFF))),
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

        let mut combo = Combo::new(combo_items, 0);
        combo.set_help("New tab opens in this location by default.");
        combo.set_show_help(true);

        Self {
            bounds: Rect::default(),
            scale: 1.0,
            scroll: 0,
            scroll_x: 0,
            content_h: 0,
            content_w: 0,
            bars: ScrollBars::new(),
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
            combo,
            ext: Choose::new(ext_items, 0, "Choose…").with_choose_icon("⌕"),
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

    /// 레이아웃 — 세로 스택. 각 항목: 라벨 + 컨트롤. **자연 폭 고정**(창이 좁으면 좌우 스크롤).
    fn relayout(&mut self, inv: &mut Invalidations) {
        let pad = self.s(PAD);
        let w = self.s(NAT_W);
        let x = self.bounds.x + pad - self.scroll_x;
        let label_h = self.s(LABEL_H);
        let gap = self.s(GAP);
        let ctrl_h = self.s(CTRL_H);
        let (radio_h, tree_h, grid_h) = (self.s(24 * 3), self.s(24 * 5), self.s(26 + 24 * 4));
        let top = self.bounds.y + pad - self.scroll;
        let mut y = top;
        y = place(&mut self.cb_right, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.cb_left, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.cb_only, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.radio, x, y, w, radio_h, label_h, gap, inv);
        y = place(&mut self.textbox, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.combo, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.ext, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.tree, x, y, w, tree_h, label_h, gap, inv);
        y = place(&mut self.grid, x, y, w, grid_h, label_h, gap, inv);
        // 콘텐츠 총 크기(스크롤 클램프 근거) — 오프셋을 되돌려 계산.
        self.content_h = (y - top) + pad;
        self.content_w = w + pad * 2;
        inv.push(self.bounds);
    }

    /// Choose 컨트롤에 찾기 창 어댑터를 꽂는다(bin이 단일 파일 선택기 주입 · Adapter 패턴).
    pub fn set_choose_picker(&mut self, picker: Box<dyn ChoosePicker>) {
        self.ext.set_picker(picker);
    }

    /// 스크롤바 페이드 틱(호스트가 ~5Hz 호출) — 갤러리 자체 + 내부 트리/그리드 바 모두.
    /// 표시 상태가 바뀌면 `true`(재그리기 필요).
    pub fn tick(&mut self) -> bool {
        let mut changed = self.bars.tick();
        changed |= self.tree.tick();
        changed |= self.grid.tick();
        changed
    }

    /// 스크롤 오프셋을 콘텐츠·뷰포트 범위로 클램프한다.
    fn clamp_scroll(&mut self) {
        let max_y = (self.content_h - self.bounds.h).max(0);
        let max_x = (self.content_w - self.bounds.w).max(0);
        self.scroll = self.scroll.clamp(0, max_y);
        self.scroll_x = self.scroll_x.clamp(0, max_x);
    }

    fn labels() -> [&'static str; 9] {
        [
            "Checkbox — 라벨 오른쪽 (+ 도움말 ?)",
            "Checkbox — 라벨 왼쪽",
            "Checkbox — 체크만",
            "RadioGroup (옵션 박스)",
            "TextBox — placeholder",
            "Combo — 일반 (∨)",
            "Choose — 확장 (⇕ · Choose…)",
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
        self.relayout(inv); // content_h/w 산출
        self.clamp_scroll(); // 리사이즈로 범위 초과분 회수
        self.relayout(inv); // 클램프된 오프셋으로 재배치
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // 오버레이 스크롤바가 먼저 본다(휠·드래그·호버). 소비되면 콘텐츠로 넘기지 않는다.
        let (nx, ny, consumed) = self.bars.on_event(
            ev,
            self.bounds,
            self.content_w,
            self.content_h,
            self.scroll_x,
            self.scroll,
            self.scale,
        );
        if nx != self.scroll_x || ny != self.scroll {
            self.scroll_x = nx;
            self.scroll = ny;
            self.relayout(inv);
        }
        // 마우스 이동/스크롤은 오버레이 표시가 바뀔 수 있으니 다시 그린다.
        if consumed || matches!(ev, InputEvent::MouseMove { .. }) {
            inv.push(self.bounds);
        }
        if consumed {
            return;
        }
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
        for w in widgets {
            w.paint(ctx, theme);
        }
        // 콤보/Choose 드롭다운·찾기 오버레이는 아래 위젯(트리·그리드)에 가리면 안 되므로
        // 열려 있으면 **맨 끝에 다시 그린다**(위에 겹침).
        if self.combo.is_open() {
            self.combo.paint(ctx, theme);
        }
        if self.ext.is_open() || self.ext.is_picking() {
            self.ext.paint(ctx, theme);
        }
        // 오버레이 스크롤바(맨 위에 겹침).
        self.bars.paint(
            ctx,
            theme,
            self.bounds,
            self.content_w,
            self.content_h,
            self.scroll_x,
            self.scroll,
            self.scale,
        );
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
    fn wheel_scrolls_vertically_and_clamps() {
        // 작은 뷰포트 → 세로 스크롤 발생.
        let mut g = GalleryWidget::new();
        let mut inv = Invalidations::default();
        g.set_bounds(Rect::new(0, 0, 420, 200), &mut inv);
        let y0 = g.cb_right.bounds().y;
        // 위로 스크롤(delta 음수 = 아래로) — 콘텐츠가 위로 밀린다.
        g.on_event(&InputEvent::Wheel { delta: -600 }, &mut inv);
        assert!(g.scroll > 0, "스크롤 증가");
        assert!(g.cb_right.bounds().y < y0, "콘텐츠 위로 이동");
        // 과도 스크롤은 콘텐츠 끝으로 클램프.
        g.on_event(&InputEvent::Wheel { delta: -100_000 }, &mut inv);
        assert_eq!(g.scroll, (g.content_h - g.bounds.h).max(0));
        // 반대로 끝까지 되돌리면 0으로 클램프.
        g.on_event(&InputEvent::Wheel { delta: 100_000 }, &mut inv);
        assert_eq!(g.scroll, 0);
    }

    #[test]
    fn hwheel_scrolls_horizontally_when_narrow() {
        let mut g = GalleryWidget::new();
        let mut inv = Invalidations::default();
        // 콘텐츠 자연폭(392)보다 좁은 창 → 가로 스크롤 여지.
        g.set_bounds(Rect::new(0, 0, 200, 900), &mut inv);
        let x0 = g.cb_right.bounds().x;
        g.on_event(&InputEvent::HWheel { delta: 600 }, &mut inv);
        assert!(g.scroll_x > 0, "가로 스크롤 증가");
        assert!(g.cb_right.bounds().x < x0, "콘텐츠 왼쪽으로 이동");
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
