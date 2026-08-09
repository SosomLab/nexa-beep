//! 트리 컨트롤 — **TreeView(단독)** 와 **TreeGrid(그리드+트리)** 의 공통 추상(사용자 요청 08-08).
//!
//! ## 추상 설계 (ControlBase → TreeControl → TreeView / TreeGrid)
//!
//! 두 컨트롤은 **같은 계층 모델**([`TreeModel`])과 **같은 트리 동작**([`TreeControl`] 기본 메서드)을
//! 공유하고, 표현만 다르다:
//!
//! - [`super::Control`] — 루트 인터페이스(포커스 링·활성·도움말).
//! - [`TreeControl`] — 트리 계층 인터페이스. 평탄화·펼침/접기·선택 이동·히트테스트를
//!   [`TreeModel`] 접근자 + 선택 상태 접근자만으로 **기본 메서드 상속**.
//! - [`TreeView`] — **단일 열** 트리(들여쓰기 + 셰브론 + 라벨).
//! - [`TreeGrid`] — **여러 열** 그리드. 첫 열이 트리(셰브론+라벨), 나머지는 셀 값. 헤더 포함.
//!
//! 확장: 열을 늘리거나(그리드) 셀 렌더를 바꿔도 트리 로직은 그대로 재사용된다(추상 레벨 연결).

use super::{
    draw_chevron_down, draw_chevron_right, image_fit_contain, BorderSpec, Control, ControlBase,
    ScrollBars,
};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::Rect;
use crate::theme::{IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use std::rc::Rc;

/// 트리 노드 — 라벨 + (그리드용) 추가 셀 값 + 자식 + 펼침 상태 + 선행 이미지(옵션).
#[derive(Clone, Debug)]
pub struct TreeNode {
    /// 트리 열 라벨.
    pub label: String,
    /// 그리드 추가 열 값(트리 열 제외 · TreeView는 무시).
    pub cells: Vec<String>,
    /// 자식 노드.
    pub children: Vec<TreeNode>,
    /// 펼침 여부.
    pub expanded: bool,
    /// 라벨 앞 이미지 아이콘(옵션 · 펼침 셰브론과 별개).
    pub image: Option<Rc<IconImage>>,
}

impl TreeNode {
    /// 잎 노드(자식 없음).
    #[must_use]
    pub fn leaf(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            cells: Vec::new(),
            children: Vec::new(),
            expanded: false,
            image: None,
        }
    }
    /// 자식 있는 노드(기본 펼침).
    #[must_use]
    pub fn branch(label: impl Into<String>, children: Vec<TreeNode>) -> Self {
        Self {
            label: label.into(),
            cells: Vec::new(),
            children,
            expanded: true,
            image: None,
        }
    }
    /// 그리드 셀 값 지정(체이닝).
    #[must_use]
    pub fn with_cells(mut self, cells: Vec<String>) -> Self {
        self.cells = cells;
        self
    }
    /// 선행 이미지 아이콘 지정(체이닝 · 셰브론과 별개).
    #[must_use]
    pub fn with_image(mut self, image: Rc<IconImage>) -> Self {
        self.image = Some(image);
        self
    }
}

/// 평탄화된 표시 행(현재 펼침 상태 기준).
#[derive(Clone, Debug)]
pub struct FlatRow {
    /// 루트부터의 자식 인덱스 경로(토글 대상).
    pub path: Vec<usize>,
    /// 깊이(들여쓰기).
    pub depth: usize,
    /// 트리 열 라벨.
    pub label: String,
    /// 그리드 셀 값.
    pub cells: Vec<String>,
    /// 자식 존재 여부(셰브론 표시 근거).
    pub has_children: bool,
    /// 펼침 여부.
    pub expanded: bool,
    /// 라벨 앞 이미지 아이콘(옵션).
    pub image: Option<Rc<IconImage>>,
}

/// 트리 계층 모델 — 노드 트리 + 펼침 상태(뷰와 무관 · TreeView/TreeGrid 공유).
#[derive(Clone, Debug, Default)]
pub struct TreeModel {
    /// 루트 노드들.
    pub roots: Vec<TreeNode>,
}

impl TreeModel {
    /// 루트 목록으로 만든다.
    #[must_use]
    pub fn new(roots: Vec<TreeNode>) -> Self {
        Self { roots }
    }

    /// 현재 펼침 상태 기준 **보이는 행**을 평탄화한다(접힌 가지는 자식 제외).
    #[must_use]
    pub fn flatten(&self) -> Vec<FlatRow> {
        let mut out = Vec::new();
        Self::walk(&self.roots, &mut Vec::new(), 0, &mut out);
        out
    }

    fn walk(nodes: &[TreeNode], path: &mut Vec<usize>, depth: usize, out: &mut Vec<FlatRow>) {
        for (i, n) in nodes.iter().enumerate() {
            path.push(i);
            out.push(FlatRow {
                path: path.clone(),
                depth,
                label: n.label.clone(),
                cells: n.cells.clone(),
                has_children: !n.children.is_empty(),
                expanded: n.expanded,
                image: n.image.clone(),
            });
            if n.expanded && !n.children.is_empty() {
                Self::walk(&n.children, path, depth + 1, out);
            }
            path.pop();
        }
    }

    fn node_at_mut(&mut self, path: &[usize]) -> Option<&mut TreeNode> {
        let (&first, rest) = path.split_first()?;
        let mut node = self.roots.get_mut(first)?;
        for &i in rest {
            node = node.children.get_mut(i)?;
        }
        Some(node)
    }

    /// 경로의 노드 펼침을 토글한다.
    pub fn toggle(&mut self, path: &[usize]) {
        if let Some(n) = self.node_at_mut(path) {
            if !n.children.is_empty() {
                n.expanded = !n.expanded;
            }
        }
    }

    /// 경로의 노드 펼침을 지정한다.
    pub fn set_expanded(&mut self, path: &[usize], on: bool) {
        if let Some(n) = self.node_at_mut(path) {
            if !n.children.is_empty() {
                n.expanded = on;
            }
        }
    }
}

// 레이아웃 상수(논리 px).
const ROW_H: i32 = 24;
const INDENT: i32 = 16;
const CHEV_W: i32 = 16;
const HEADER_H: i32 = 26;

/// 트리 계층 인터페이스 — 평탄화·펼침/접기·선택을 기본 메서드로 상속.
pub trait TreeControl: Control {
    /// 계층 모델(구현 필수).
    fn model(&self) -> &TreeModel;
    /// 계층 모델 가변(구현 필수).
    fn model_mut(&mut self) -> &mut TreeModel;
    /// 선택 행 인덱스(가시 행 기준 · 구현 필수).
    fn selected_row(&self) -> usize;
    /// 선택 행 지정(구현 필수).
    fn set_selected_row(&mut self, i: usize);
    /// 스크롤 오프셋 `(x, y)` 물리 px(구현 필수).
    fn scroll(&self) -> (i32, i32);
    /// 스크롤 오프셋 지정(구현 필수).
    fn set_scroll(&mut self, x: i32, y: i32);
    /// 오버레이 스크롤바 상태(구현 필수).
    fn bars_mut(&mut self) -> &mut ScrollBars;

    /// 트리 영역의 top(헤더 아래 등 — 그리드가 재정의). 기본 = bounds.y.
    fn tree_top(&self) -> i32 {
        self.bounds().y
    }

    /// 행이 그려지는 뷰포트(헤더 아래 · 스크롤 대상).
    fn rows_viewport(&self) -> Rect {
        let b = self.bounds();
        let top = self.tree_top();
        Rect::new(b.x, top, b.w, (b.bottom() - top).max(0))
    }

    /// 콘텐츠 총 크기 `(w, h)` — 세로=행수×행높이, 가로 기본=뷰포트 폭(그리드가 열 합으로 재정의).
    fn content_size(&self) -> (i32, i32) {
        let h = self.rows().len() as i32 * self.s(ROW_H);
        (self.rows_viewport().w, h)
    }

    /// 보이는 행.
    fn rows(&self) -> Vec<FlatRow> {
        self.model().flatten()
    }

    /// 선택을 delta만큼 이동(경계 클램프).
    fn move_selection(&mut self, delta: i32, inv: &mut Invalidations) {
        let n = self.rows().len() as i32;
        if n == 0 {
            return;
        }
        let i = (self.selected_row() as i32 + delta).clamp(0, n - 1);
        self.set_selected_row(i as usize);
        inv.push(self.bounds());
    }

    /// 가시 행 i의 펼침 토글.
    fn toggle_row(&mut self, i: usize, inv: &mut Invalidations) {
        if let Some(row) = self.rows().get(i) {
            let path = row.path.clone();
            self.model_mut().toggle(&path);
            inv.push(self.bounds());
        }
    }

    /// →(펼침) / ←(접힘) — 선택 행 기준.
    fn expand_selected(&mut self, on: bool, inv: &mut Invalidations) {
        let i = self.selected_row();
        if let Some(row) = self.rows().get(i) {
            if row.has_children {
                let path = row.path.clone();
                self.model_mut().set_expanded(&path, on);
                inv.push(self.bounds());
            }
        }
    }

    /// (x,y) → (가시 행 인덱스, 셰브론을 눌렀는가). 스크롤 오프셋 반영.
    fn row_hit(&self, x: i32, y: i32) -> Option<(usize, bool)> {
        let rh = self.s(ROW_H).max(1);
        let top = self.tree_top();
        let (sx, sy) = self.scroll();
        if y < top || y >= self.bounds().bottom() {
            return None;
        }
        let i = ((y - top + sy) / rh) as usize;
        let rows = self.rows();
        let row = rows.get(i)?;
        // 셰브론 영역: 깊이 들여쓰기 지점(가로 스크롤 반영).
        let chev_x = self.bounds().x + self.s(4) + self.s(INDENT) * row.depth as i32 - sx;
        let on_chev = row.has_children && x >= chev_x && x < chev_x + self.s(CHEV_W);
        Some((i, on_chev))
    }

    /// 트리 열 한 칸을 그린다(들여쓰기 + 셰브론 + 라벨) — TreeView 전체 / TreeGrid 첫 열 공용.
    fn paint_tree_cell(
        &self,
        ctx: &mut dyn DrawCtx,
        theme: &Theme,
        row: &FlatRow,
        cell: Rect,
        selected: bool,
    ) {
        if selected {
            ctx.fill_rect(
                cell,
                if self.is_active() {
                    theme.sel_bg
                } else {
                    theme.sel_bg_inactive
                },
            );
        }
        let chev_x = cell.x + self.s(4) + self.s(INDENT) * row.depth as i32;
        let cy = cell.y + (cell.h - self.s(CHEV_W)) / 2;
        let chev = Rect::new(chev_x, cy, self.s(CHEV_W), self.s(CHEV_W));
        if row.has_children {
            let color = theme.text_dim;
            if row.expanded {
                draw_chevron_down(ctx, chev, color);
            } else {
                draw_chevron_right(ctx, chev, color);
            }
        }
        let mut tx = chev.right() + self.s(4);
        // 선행 이미지(옵션 · 셰브론과 별개) — 공용 아이콘 크기(콤보/버튼과 동일 원천).
        if let Some(img) = row.image.as_deref() {
            let isz = self.s(super::LEADING_ICON);
            let boxr = Rect::new(tx, cell.y + (cell.h - isz) / 2, isz, isz);
            let fit = image_fit_contain(boxr, img.w as i32, img.h as i32);
            ctx.image_scaled(fit, img, cell);
            tx += isz + self.s(4);
        }
        ctx.select_font(FontSlot::Base, false);
        let ty = cell.y + (cell.h - ctx.text_height()) / 2;
        ctx.text(tx, ty, cell, &row.label, theme.text);
    }
}

/// 공통 이벤트 처리(TreeView/TreeGrid 공용).
fn tree_event<T: TreeControl + ?Sized>(t: &mut T, ev: &InputEvent, inv: &mut Invalidations) {
    // 오버레이 스크롤바 먼저(휠·드래그·호버). 소비되면 콘텐츠로 넘기지 않는다.
    let vp = t.rows_viewport();
    let (cw, ch) = t.content_size();
    let (sx, sy) = t.scroll();
    let scale = t.base().scale;
    let (nx, ny, consumed) = t.bars_mut().on_event(ev, vp, cw, ch, sx, sy, scale);
    if nx != sx || ny != sy {
        t.set_scroll(nx, ny);
        inv.push(t.bounds());
    }
    if consumed || matches!(ev, InputEvent::MouseMove { .. }) {
        inv.push(t.bounds());
    }
    if consumed {
        return;
    }
    match *ev {
        InputEvent::MouseDown { x, y, .. } => {
            if let Some((i, on_chev)) = t.row_hit(x, y) {
                t.set_selected_row(i);
                if on_chev {
                    t.toggle_row(i, inv);
                } else {
                    inv.push(t.bounds());
                }
            }
        }
        InputEvent::Key { key, .. } if t.is_focused() => match key {
            Key::Up => t.move_selection(-1, inv),
            Key::Down => t.move_selection(1, inv),
            Key::Right => t.expand_selected(true, inv),
            Key::Left => t.expand_selected(false, inv),
            Key::Enter | Key::Space => {
                let i = t.selected_row();
                t.toggle_row(i, inv);
            }
            _ => {}
        },
        _ => {}
    }
}

// ───────────────────────────── TreeView(단일 열) ─────────────────────────────

/// 트리뷰 — 단일 열 계층 목록.
#[derive(Debug)]
pub struct TreeView {
    base: ControlBase,
    model: TreeModel,
    selected: usize,
    scroll_x: i32,
    scroll_y: i32,
    bars: ScrollBars,
    border: BorderSpec,
}

impl TreeView {
    /// 모델로 만든다.
    #[must_use]
    pub fn new(model: TreeModel) -> Self {
        Self {
            base: ControlBase::default(),
            model,
            selected: 0,
            scroll_x: 0,
            scroll_y: 0,
            bars: ScrollBars::new(),
            border: BorderSpec::default(),
        }
    }

    /// 외곽 테두리 설정(두께·색·투명도 · 두께 0 = 없음).
    pub fn set_border(&mut self, border: BorderSpec) {
        self.border = border;
    }
    /// 선택 행의 라벨.
    #[must_use]
    pub fn selected_label(&self) -> Option<String> {
        self.rows().get(self.selected).map(|r| r.label.clone())
    }

    /// 스크롤바 페이드 틱(호스트가 ~5Hz 호출) — 표시 상태 변화 시 `true`.
    pub fn tick(&mut self) -> bool {
        self.bars.tick()
    }
}

impl Control for TreeView {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}
impl TreeControl for TreeView {
    fn model(&self) -> &TreeModel {
        &self.model
    }
    fn model_mut(&mut self) -> &mut TreeModel {
        &mut self.model
    }
    fn selected_row(&self) -> usize {
        self.selected
    }
    fn set_selected_row(&mut self, i: usize) {
        self.selected = i;
    }
    fn scroll(&self) -> (i32, i32) {
        (self.scroll_x, self.scroll_y)
    }
    fn set_scroll(&mut self, x: i32, y: i32) {
        self.scroll_x = x;
        self.scroll_y = y;
    }
    fn bars_mut(&mut self) -> &mut ScrollBars {
        &mut self.bars
    }
}

impl Widget for TreeView {
    fn bounds(&self) -> Rect {
        self.base.bounds
    }
    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.base.bounds = bounds;
        inv.push(bounds);
    }
    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        tree_event(self, ev, inv);
    }
    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        let rh = self.s(ROW_H);
        let top = self.tree_top();
        let bottom = b.bottom();
        // 스크롤 오프셋 반영 · 뷰포트 안의 온전한 행만(수직 넘침 방지).
        for (i, row) in self.rows().iter().enumerate() {
            let ry = top - self.scroll_y + rh * i as i32;
            if ry < top || ry + rh > bottom {
                continue;
            }
            let cell = Rect::new(b.x - self.scroll_x, ry, b.w + self.scroll_x, rh);
            self.paint_tree_cell(ctx, theme, row, cell, i == self.selected);
        }
        let (cw, ch) = self.content_size();
        self.bars.paint(
            ctx,
            theme,
            self.rows_viewport(),
            cw,
            ch,
            self.scroll_x,
            self.scroll_y,
            self.base.scale,
        );
        draw_border(ctx, b, self.border, self.base.scale);
    }
}

/// 외곽 테두리 그리기(두께 0이면 생략) — 트리/그리드 공용.
fn draw_border(ctx: &mut dyn DrawCtx, b: Rect, border: BorderSpec, scale: f32) {
    if border.width <= 0.0 {
        return;
    }
    let w = (border.width * scale).max(0.5); // 소수 두께 허용(0.5px 얇은 선)
    ctx.stroke_round_rect_alpha(b, 0, border.color, w, border.alpha);
}

// ───────────────────────────── TreeGrid(그리드+트리) ─────────────────────────────

/// 그리드 열 정의.
#[derive(Clone, Debug)]
pub struct GridColumn {
    /// 헤더 제목.
    pub title: String,
    /// 열 폭(논리 px).
    pub width: i32,
}

impl GridColumn {
    /// (제목, 폭).
    pub fn new(title: impl Into<String>, width: i32) -> Self {
        Self {
            title: title.into(),
            width,
        }
    }
}

/// 트리 그리드 — 첫 열이 트리, 나머지는 셀 값(같은 [`TreeModel`] 재사용).
#[derive(Debug)]
pub struct TreeGrid {
    base: ControlBase,
    model: TreeModel,
    selected: usize,
    /// 열 정의(첫 열 = 트리 열).
    columns: Vec<GridColumn>,
    scroll_x: i32,
    scroll_y: i32,
    bars: ScrollBars,
    border: BorderSpec,
}

impl TreeGrid {
    /// 모델 + 열 정의로 만든다(첫 열이 트리 열).
    #[must_use]
    pub fn new(model: TreeModel, columns: Vec<GridColumn>) -> Self {
        Self {
            base: ControlBase::default(),
            model,
            selected: 0,
            columns,
            scroll_x: 0,
            scroll_y: 0,
            bars: ScrollBars::new(),
            border: BorderSpec::default(),
        }
    }

    /// 외곽 테두리 설정(두께·색·투명도 · 두께 0 = 없음).
    pub fn set_border(&mut self, border: BorderSpec) {
        self.border = border;
    }

    /// 전체 열 폭 합(물리 px).
    fn columns_width(&self) -> i32 {
        self.columns.iter().map(|c| self.s(c.width)).sum()
    }

    /// 스크롤바 페이드 틱(호스트가 ~5Hz 호출) — 표시 상태 변화 시 `true`.
    pub fn tick(&mut self) -> bool {
        self.bars.tick()
    }
}

impl Control for TreeGrid {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}
impl TreeControl for TreeGrid {
    fn model(&self) -> &TreeModel {
        &self.model
    }
    fn model_mut(&mut self) -> &mut TreeModel {
        &mut self.model
    }
    fn selected_row(&self) -> usize {
        self.selected
    }
    fn set_selected_row(&mut self, i: usize) {
        self.selected = i;
    }
    fn scroll(&self) -> (i32, i32) {
        (self.scroll_x, self.scroll_y)
    }
    fn set_scroll(&mut self, x: i32, y: i32) {
        self.scroll_x = x;
        self.scroll_y = y;
    }
    fn bars_mut(&mut self) -> &mut ScrollBars {
        &mut self.bars
    }
    /// 그리드는 헤더 아래부터 트리 행.
    fn tree_top(&self) -> i32 {
        self.base.bounds.y + self.s(HEADER_H)
    }
    /// 가로 콘텐츠 = 전체 열 폭 합(길면 좌우 스크롤).
    fn content_size(&self) -> (i32, i32) {
        let h = self.rows().len() as i32 * self.s(ROW_H);
        (self.columns_width(), h)
    }
}

impl Widget for TreeGrid {
    fn bounds(&self) -> Rect {
        self.base.bounds
    }
    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.base.bounds = bounds;
        inv.push(bounds);
    }
    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        tree_event(self, ev, inv);
    }
    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        let ox = self.scroll_x; // 가로 스크롤(헤더·셀 공통 이동)

        // 헤더(가로 스크롤 반영 · b.w로 텍스트 클립). 텍스트는 헤더 높이의 세로 중앙.
        let header = Rect::new(b.x, b.y, b.w, self.s(HEADER_H));
        ctx.fill_rect(header, theme.chrome_bg);
        let mut cx = b.x - ox;
        ctx.select_font(FontSlot::Status, false);
        let hty = header.y + (header.h - ctx.text_height()) / 2; // 상·하 여백 동일(실측)
        for col in &self.columns {
            let w = self.s(col.width);
            ctx.text(cx + self.s(8), hty, header, &col.title, theme.text_dim);
            cx += w;
            ctx.fill_rect(Rect::new(cx - 1, header.y, 1, b.h), theme.border);
        }
        ctx.fill_rect(Rect::new(b.x, header.bottom() - 1, b.w, 1), theme.border);

        // 행(세로 스크롤 반영 · 온전한 행만 · 가로 스크롤 반영).
        let rh = self.s(ROW_H);
        let top = self.tree_top();
        let bottom = b.bottom();
        let tree_w = self.columns.first().map_or(b.w, |c| self.s(c.width));
        for (i, row) in self.rows().iter().enumerate() {
            let y = top - self.scroll_y + rh * i as i32;
            if y < top || y + rh > bottom {
                continue;
            }
            if i == self.selected {
                ctx.fill_rect(
                    Rect::new(b.x, y, b.w, rh),
                    if self.is_active() {
                        theme.sel_bg
                    } else {
                        theme.sel_bg_inactive
                    },
                );
            }
            // 첫 열 = 트리 셀(배경 재도색 방지 selected=false · 가로 스크롤).
            let tree_cell = Rect::new(b.x - ox, y, tree_w, rh);
            self.paint_tree_cell(ctx, theme, row, tree_cell, false);
            // 나머지 열 = 셀 값.
            let mut colx = b.x + tree_w - ox;
            for (ci, col) in self.columns.iter().enumerate().skip(1) {
                let w = self.s(col.width);
                if let Some(val) = row.cells.get(ci - 1) {
                    ctx.select_font(FontSlot::Base, false);
                    let th = ctx.text_height();
                    ctx.text(
                        colx + self.s(8),
                        y + (rh - th) / 2,
                        Rect::new(colx, y, w, rh),
                        val,
                        theme.text,
                    );
                }
                colx += w;
            }
        }

        // 오버레이 스크롤바.
        let (cw, ch) = self.content_size();
        self.bars.paint(
            ctx,
            theme,
            self.rows_viewport(),
            cw,
            ch,
            self.scroll_x,
            self.scroll_y,
            self.base.scale,
        );
        draw_border(ctx, b, self.border, self.base.scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> TreeModel {
        TreeModel::new(vec![
            TreeNode::branch(
                "Path Finder",
                vec![
                    TreeNode::leaf("About Path Finder"),
                    TreeNode::branch("Trash", vec![TreeNode::leaf("Empty Trash")]),
                ],
            ),
            TreeNode::leaf("Show Desktop"),
        ])
    }

    fn view() -> (TreeView, Invalidations) {
        let mut v = TreeView::new(model());
        let mut inv = Invalidations::default();
        v.set_bounds(Rect::new(0, 0, 300, 300), &mut inv);
        (v, inv)
    }
    fn click(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }
    fn key(k: Key) -> InputEvent {
        InputEvent::Key {
            key: k,
            shift: false,
            primary: false,
        }
    }

    #[test]
    fn flatten_respects_expansion() {
        let m = model();
        // 기본: Path Finder(펼침) → About, Trash(펼침) → Empty Trash, 그리고 Show Desktop = 5행.
        assert_eq!(m.flatten().len(), 5);
    }

    #[test]
    fn collapse_hides_children() {
        let mut m = model();
        m.toggle(&[0]); // Path Finder 접기
        let rows = m.flatten();
        // Path Finder + Show Desktop만.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "Path Finder");
        assert!(!rows[0].expanded);
    }

    #[test]
    fn click_chevron_toggles() {
        let (mut v, mut inv) = view();
        assert_eq!(v.rows().len(), 5);
        // Path Finder 행(0)의 셰브론 클릭 — depth 0, chev_x ≈ x+4.
        v.on_event(&click(6, 6), &mut inv);
        assert_eq!(v.rows().len(), 2, "접힘");
    }

    #[test]
    fn keyboard_navigates_and_expands() {
        let (mut v, mut inv) = view();
        v.set_focused(true);
        v.on_event(&key(Key::Down), &mut inv);
        assert_eq!(v.selected_row(), 1);
        // 선택을 Path Finder(0)로 되돌려 접기.
        v.on_event(&key(Key::Up), &mut inv);
        v.on_event(&key(Key::Left), &mut inv);
        assert_eq!(v.rows().len(), 2, "← 접힘");
        v.on_event(&key(Key::Right), &mut inv);
        assert_eq!(v.rows().len(), 5, "→ 펼침");
    }

    #[test]
    fn tree_grid_shares_model_and_adds_columns() {
        let m = TreeModel::new(vec![TreeNode::branch(
            "Path Finder",
            vec![TreeNode::leaf("Settings…").with_cells(vec!["⌘,".into()])],
        )]);
        let cols = vec![
            GridColumn::new("Menu", 200),
            GridColumn::new("Command", 100),
        ];
        let mut g = TreeGrid::new(m, cols);
        let mut inv = Invalidations::default();
        g.set_bounds(Rect::new(0, 0, 300, 300), &mut inv);
        // 같은 평탄화 로직 재사용.
        assert_eq!(g.rows().len(), 2);
        assert_eq!(g.rows()[1].cells, vec!["⌘,".to_string()]);
        // 헤더 아래부터 트리 행.
        assert!(g.tree_top() > g.bounds().y);
    }

    #[test]
    fn many_rows_scroll_vertically() {
        // 행이 많고 뷰포트가 작으면 세로 스크롤.
        let nodes: Vec<TreeNode> = (0..40)
            .map(|i| TreeNode::leaf(format!("row {i}")))
            .collect();
        let mut v = TreeView::new(TreeModel::new(nodes));
        let mut inv = Invalidations::default();
        v.set_bounds(Rect::new(0, 0, 200, 100), &mut inv);
        let (_cw, ch) = v.content_size();
        assert!(ch > 100, "콘텐츠가 뷰포트보다 큼(40행×24=960)");
        v.set_focused(true);
        v.on_event(&InputEvent::Wheel { delta: -300 }, &mut inv);
        assert_eq!(v.scroll(), (0, 100), "휠 세로 스크롤");
    }

    #[test]
    fn wide_columns_scroll_horizontally() {
        let m = TreeModel::new(vec![
            TreeNode::leaf("A").with_cells(vec!["a".into()]),
            TreeNode::leaf("B").with_cells(vec!["b".into()]),
        ]);
        let cols = vec![
            GridColumn::new("Menu", 300),
            GridColumn::new("Command", 200),
        ];
        let mut g = TreeGrid::new(m, cols);
        let mut inv = Invalidations::default();
        g.set_bounds(Rect::new(0, 0, 300, 300), &mut inv); // 창(300) < 열 합(500)
        let (cw, _ch) = g.content_size();
        assert_eq!(cw, 500, "열 폭 합");
        g.on_event(&InputEvent::HWheel { delta: 300 }, &mut inv);
        assert_eq!(g.scroll().0, 100, "가로 스크롤(delta/3)");
    }

    #[test]
    fn tree_grid_row_hit_accounts_for_header() {
        let m = TreeModel::new(vec![TreeNode::leaf("A"), TreeNode::leaf("B")]);
        let mut g = TreeGrid::new(m, vec![GridColumn::new("Menu", 200)]);
        let mut inv = Invalidations::default();
        g.set_bounds(Rect::new(0, 0, 300, 300), &mut inv);
        g.set_focused(true);
        // 헤더 영역 클릭은 행 아님.
        g.on_event(&click(10, 5), &mut inv);
        // 첫 행(헤더 아래) 클릭.
        let y = g.tree_top() + 2;
        g.on_event(&click(10, y), &mut inv);
        assert_eq!(g.selected_row(), 0);
    }
}
