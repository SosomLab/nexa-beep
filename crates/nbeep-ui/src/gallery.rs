//! 컨트롤 갤러리 — 커스텀 컨트롤을 한 화면에서 눈으로 확인하는 **임시 데모**(사용자 요청 08-08).
//!
//! 새 컨트롤([`crate::controls`])을 각각 하나씩 배치해 포커스 링·도움말 "?"·창 활성 색·상호작용을
//! 실물로 확인한다. **제품 화면이 아니라 검수용** — 정식 UI 통합은 별도.

use crate::controls::{
    BorderSpec, Button, Checkbox, Choose, ChoosePicker, ColorPicker, Combo, ComboControl,
    ComboItem, Control, GridColumn, ImageFit, LabelSide, MenuBar, MenuDef, MenuEntry,
    PositionPicker, RadioGroup, RadioOption, ScrollBars, TextBox, TimeoutButton, ToolIcon,
    ToolItem, Toolbar, TreeGrid, TreeModel, TreeNode, TreeView,
};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::InputEvent;
use crate::geom::{Point, Rect};
use crate::theme::{Color, IconImage, Theme};
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
    tree_img: TreeView,
    grid: TreeGrid,
    grid_img: TreeGrid,
    btn_text: Button,
    btn_icon: Button,
    btn_front: Button,
    btn_img: Button,
    // ── 08-10 추가 반영(사용자 요청 — "전체 기능이 식별되도록") ──
    /// 메뉴 바(자체 렌더 풀다운 — M3 도구 배치).
    menubar: MenuBar,
    /// 툴바(SVG 알파 마스크 아이콘 — 테마 틴트).
    toolbar: Toolbar,
    /// 스스로 눌리는 버튼(수신 승인·발신 대기 UX — M4). 만료 시 데모로 재시작한다.
    timeout_btn: TimeoutButton,
    /// 갤러리 자체 시계(데모 전용 — 호스트 tick과 무관하게 자립).
    t0: std::time::Instant,
    tb_started: bool,
    /// 3×3 위치 선택 그리드(타입어헤드 HUD 위치 설정 — M3).
    posgrid: PositionPicker,
    /// 색상 선택기(테마 색 설정 — 08-10).
    colorpick: ColorPicker,
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

        // 이미지 데모 = **브랜딩 이미지(앱 아이콘)** — raw RGBA로 임베드(런타임 디코더 불요).
        let brand = Rc::new(IconImage::from_rgba(
            64,
            64,
            include_bytes!("../assets/brand-64.rgba").to_vec(),
        ));
        let img = |_rgb: (u8, u8, u8)| brand.clone();
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

        // 행 앞 이미지가 들어간 트리/그리드 모델(우측 버전용) — 브랜딩 이미지 사용.
        let ti = |_rgb: (u8, u8, u8)| brand.clone();
        let tree_img_model = TreeModel::new(vec![
            TreeNode::branch(
                "Path Finder",
                vec![
                    TreeNode::leaf("About Path Finder").with_image(ti((0x3D, 0x8B, 0xFF))),
                    TreeNode::branch(
                        "Trash",
                        vec![TreeNode::leaf("Empty Trash").with_image(ti((0xE5, 0x53, 0x4B)))],
                    )
                    .with_image(ti((0x8A, 0x91, 0x9C))),
                ],
            )
            .with_image(ti((0x3D, 0x8B, 0xFF))),
            TreeNode::leaf("Show Desktop").with_image(ti((0x2E, 0xA0, 0x43))),
        ]);
        let grid_img_model = TreeModel::new(vec![TreeNode::branch(
            "Path Finder",
            vec![
                TreeNode::leaf("About Path Finder")
                    .with_cells(vec![String::new()])
                    .with_image(ti((0x3D, 0x8B, 0xFF))),
                TreeNode::leaf("Settings…")
                    .with_cells(vec!["⌘,".into()])
                    .with_image(ti((0x8A, 0x91, 0x9C))),
                TreeNode::leaf("Empty Trash")
                    .with_cells(vec!["⇧⌘⌫".into()])
                    .with_image(ti((0xE5, 0x53, 0x4B))),
            ],
        )
        .with_image(ti((0x3D, 0x8B, 0xFF)))]);

        // 트리/그리드 외곽 테두리 두께 0.5px(반투명 · 사용자 확정).
        let border = BorderSpec::new(0.5, Color::from_rgb(0x8A, 0x91, 0x9C), 0.6);
        let cols = || vec![GridColumn::new("Menu", 220), GridColumn::new("Command", 90)];
        let mut tree = TreeView::new(tree_model);
        tree.set_border(border);
        let mut tree_img = TreeView::new(tree_img_model);
        tree_img.set_border(border);
        let mut grid = TreeGrid::new(grid_model, cols());
        grid.set_border(border);
        let mut grid_img = TreeGrid::new(grid_img_model, cols());
        grid_img.set_border(border);

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
            tree,
            tree_img,
            grid,
            grid_img,
            btn_text: Button::new("Save").with_image(img((0x3D, 0x8B, 0xFF))),
            btn_icon: Button::icon(img((0x2E, 0xA0, 0x43))),
            btn_front: Button::new("Send")
                .with_image(img((0xB5, 0x7C, 0x1E)))
                .image_front(true),
            btn_img: Button::icon(img((0x5B, 0x6C, 0xFF))).image_fill(ImageFit::Cover),
            menubar: MenuBar::new(vec![
                MenuDef::new(
                    "Menu",
                    vec![
                        MenuEntry::Item(ComboItem::new("about", "About Nexa Beep")),
                        MenuEntry::Separator,
                        MenuEntry::Item(ComboItem::new("settings", "Settings…")),
                    ],
                ),
                MenuDef::new(
                    "Help",
                    vec![MenuEntry::Item(ComboItem::new("docs", "Documentation"))],
                ),
            ]),
            toolbar: Toolbar::new(vec![
                ToolItem::new(
                    "refresh",
                    ToolIcon::Mask {
                        w: crate::icons::REFRESH_SIZE,
                        h: crate::icons::REFRESH_SIZE,
                        alpha: crate::icons::REFRESH_ALPHA,
                    },
                ),
                ToolItem::new(
                    "add",
                    ToolIcon::Mask {
                        w: crate::icons::ADD_SIZE,
                        h: crate::icons::ADD_SIZE,
                        alpha: crate::icons::ADD_ALPHA,
                    },
                ),
                ToolItem::new(
                    "quarantine",
                    ToolIcon::Mask {
                        w: crate::icons::SHIELD_SIZE,
                        h: crate::icons::SHIELD_SIZE,
                        alpha: crate::icons::SHIELD_ALPHA,
                    },
                ),
            ]),
            timeout_btn: TimeoutButton::new("Cancel — demo", 30_000),
            t0: std::time::Instant::now(),
            tb_started: false,
            posgrid: PositionPicker::new(),
            colorpick: ColorPicker::new("#3D8BFF"),
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
        self.tree_img.set_scale(s);
        self.grid.set_scale(s);
        self.grid_img.set_scale(s);
        self.btn_text.set_scale(s);
        self.btn_icon.set_scale(s);
        self.btn_front.set_scale(s);
        self.btn_img.set_scale(s);
        self.menubar.set_scale(s);
        self.toolbar.set_scale(s);
        self.timeout_btn.set_scale(s);
        self.posgrid.set_scale(s);
        self.colorpick.set_scale(s);
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
        let (btn_tw, btn_iw, btn_fw, btn_imw, btn_imh) =
            (self.s(120), self.s(44), self.s(160), self.s(64), self.s(48));
        let top = self.bounds.y + pad - self.scroll;
        let mut y = top;
        y = place(&mut self.cb_right, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.cb_left, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.cb_only, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.radio, x, y, w, radio_h, label_h, gap, inv);
        y = place(&mut self.textbox, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.combo, x, y, w, ctrl_h, label_h, gap, inv);
        y = place(&mut self.ext, x, y, w, ctrl_h, label_h, gap, inv);
        // 트리 쌍(좌: 기본 · 우: 행 이미지) — 같은 y에 나란히.
        let tcolw = self.s(210);
        let ty = y + label_h;
        self.tree.set_bounds(Rect::new(x, ty, tcolw, tree_h), inv);
        self.tree_img
            .set_bounds(Rect::new(x + tcolw + gap, ty, tcolw, tree_h), inv);
        y = ty + tree_h + gap;
        // 그리드 쌍(좌: 기본 · 우: 행 이미지).
        let gcolw = self.s(320);
        let gy = y + label_h;
        self.grid.set_bounds(Rect::new(x, gy, gcolw, grid_h), inv);
        self.grid_img
            .set_bounds(Rect::new(x + gcolw + gap, gy, gcolw, grid_h), inv);
        y = gy + grid_h + gap;
        y = place(&mut self.btn_text, x, y, btn_tw, ctrl_h, label_h, gap, inv);
        y = place(&mut self.btn_icon, x, y, btn_iw, ctrl_h, label_h, gap, inv);
        y = place(&mut self.btn_front, x, y, btn_fw, ctrl_h, label_h, gap, inv);
        y = place(&mut self.btn_img, x, y, btn_imw, btn_imh, label_h, gap, inv);
        // ── 08-10 추가 컨트롤 ──
        let (menu_h, tool_h, tb_w, tb_h, pos_w, pos_h) = (
            self.s(28),
            self.s(36),
            self.s(220),
            self.s(30),
            self.s(128),
            self.s(96),
        );
        y = place(&mut self.menubar, x, y, w, menu_h, label_h, gap, inv);
        y = place(&mut self.toolbar, x, y, w, tool_h, label_h, gap, inv);
        y = place(&mut self.timeout_btn, x, y, tb_w, tb_h, label_h, gap, inv);
        y = place(&mut self.posgrid, x, y, pos_w, pos_h, label_h, gap, inv);
        let (cp_w, cp_h) = (self.colorpick.preferred_width(), self.s(30));
        y = place(&mut self.colorpick, x, y, cp_w, cp_h, label_h, gap, inv);
        // 콘텐츠 총 크기 — 가장 넓은 행(트리·그리드 쌍) 기준.
        let widest = w.max(tcolw * 2 + gap).max(gcolw * 2 + gap);
        self.content_h = (y - top) + pad;
        self.content_w = widest + pad * 2;
        inv.push(self.bounds);
    }

    /// Choose 컨트롤에 찾기 창 어댑터를 꽂는다(인라인 오버레이 모드 · Adapter 패턴).
    pub fn set_choose_picker(&mut self, picker: Box<dyn ChoosePicker>) {
        self.ext.set_picker(picker);
    }

    /// "Choose…"가 눌렸는가(1회성) — 어댑터 미장착 시 호스트가 **별도 모달 창**을 연다.
    pub fn take_choose_request(&mut self) -> bool {
        self.ext.take_chose()
    }

    /// 모달 선택 결과를 Choose 값으로 반영.
    pub fn set_choose_value(&mut self, v: &str, inv: &mut Invalidations) {
        self.ext.set_text(v);
        inv.push(self.bounds);
    }

    /// 스크롤바 페이드 틱(호스트가 ~5Hz 호출) — 갤러리 자체 + 내부 트리/그리드 바 모두.
    /// 표시 상태가 바뀌면 `true`(재그리기 필요).
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let mut changed = self.bars.tick(now_ms);
        changed |= self.tree.tick(now_ms);
        changed |= self.grid.tick(now_ms);
        // TimeoutButton 데모 — 갤러리 자체 시계로 카운트다운, 만료(발화)하면 재시작.
        let now = u64::try_from(self.t0.elapsed().as_millis()).unwrap_or(u64::MAX);
        if !self.tb_started {
            self.timeout_btn.start(now);
            self.tb_started = true;
            changed = true;
        }
        changed |= self.timeout_btn.tick(now);
        if self.timeout_btn.take_fired().is_some() {
            self.timeout_btn.start(now);
            changed = true;
        }
        changed
    }

    /// 스크롤 오프셋을 콘텐츠·뷰포트 범위로 클램프한다.
    fn clamp_scroll(&mut self) {
        let max_y = (self.content_h - self.bounds.h).max(0);
        let max_x = (self.content_w - self.bounds.w).max(0);
        self.scroll = self.scroll.clamp(0, max_y);
        self.scroll_x = self.scroll_x.clamp(0, max_x);
    }

    fn labels() -> [&'static str; 20] {
        [
            "Checkbox — 라벨 오른쪽 (+ 도움말 ?)",
            "Checkbox — 라벨 왼쪽",
            "Checkbox — 체크만",
            "RadioGroup (옵션 박스)",
            "TextBox — placeholder",
            "Combo — 일반 (∨)",
            "Choose — 확장 (⇕ · Choose…)",
            "TreeView — 계층",
            "TreeView — 행 이미지",
            "TreeGrid — 그리드 + 트리",
            "TreeGrid — 행 이미지",
            "Button — 이미지 + 텍스트",
            "Button — 이미지만",
            "Button — 이미지 앞 고정(텍스트 중앙)",
            "Button — 이미지 버튼(Cover)",
            "MenuBar — 자체 렌더 풀다운",
            "Toolbar — SVG 알파 마스크 아이콘(테마 틴트)",
            "TimeoutButton — 스스로 눌리는 버튼(만료 시 데모 재시작)",
            "PositionPicker — 3×3 위치 그리드",
            "ColorPicker — 스와치 · #RRGGBB · 프리셋(테마 색 설정)",
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
        // ★ 모달 캡처 — 콤보/Choose/메뉴 오버레이가 열려 있으면 그 컨트롤만 이벤트를 받는다.
        // (드롭다운 항목 클릭이 뒤에 있는 다른 컨트롤로 전파되던 버그 차단.)
        if self.combo.is_open() {
            self.combo.on_event(ev, inv);
            inv.push(self.bounds);
            return;
        }
        if self.ext.is_open() || self.ext.is_picking() {
            self.ext.on_event(ev, inv);
            inv.push(self.bounds);
            return;
        }
        if self.menubar.is_open() {
            self.menubar.on_event(ev, inv);
            let _ = self.menubar.take_picked(); // 데모 — 선택은 소비만 한다
            inv.push(self.bounds);
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
            self.tree_img
                .set_focused(self.tree_img.bounds().contains(p));
            self.grid.set_focused(self.grid.bounds().contains(p));
            self.grid_img
                .set_focused(self.grid_img.bounds().contains(p));
            self.btn_text
                .set_focused(self.btn_text.bounds().contains(p));
            self.btn_icon
                .set_focused(self.btn_icon.bounds().contains(p));
            self.btn_front
                .set_focused(self.btn_front.bounds().contains(p));
            self.btn_img.set_focused(self.btn_img.bounds().contains(p));
            self.timeout_btn
                .set_focused(self.timeout_btn.bounds().contains(p));
            self.posgrid.set_focused(self.posgrid.bounds().contains(p));
            if !self.colorpick.bounds().contains(p) {
                self.colorpick.set_focused(false);
            }
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
        self.tree_img.on_event(ev, inv);
        self.grid.on_event(ev, inv);
        self.grid_img.on_event(ev, inv);
        self.btn_text.on_event(ev, inv);
        self.btn_icon.on_event(ev, inv);
        self.btn_front.on_event(ev, inv);
        self.btn_img.on_event(ev, inv);
        self.menubar.on_event(ev, inv);
        let _ = self.menubar.take_picked(); // 데모 — 선택은 소비만 한다
        self.toolbar.on_event(ev, inv);
        let _ = self.toolbar.take_clicked();
        self.timeout_btn.on_event(ev, inv);
        if self.timeout_btn.take_fired().is_some() {
            // 클릭 발화 데모 — 즉시 재시작(다음 tick에서 시계가 이어진다).
            let now = u64::try_from(self.t0.elapsed().as_millis()).unwrap_or(u64::MAX);
            self.timeout_btn.start(now);
        }
        self.posgrid.on_event(ev, inv);
        let _ = self.posgrid.take_changed();
        self.colorpick.on_event(ev, inv);
        let _ = self.colorpick.take_changed(); // 데모 — 값은 소비만
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        ctx.fill_rect(self.bounds, theme.panel_bg);
        let labels = Self::labels();
        let widgets: [&dyn Widget; 20] = [
            &self.cb_right,
            &self.cb_left,
            &self.cb_only,
            &self.radio,
            &self.textbox,
            &self.combo,
            &self.ext,
            &self.tree,
            &self.tree_img,
            &self.grid,
            &self.grid_img,
            &self.btn_text,
            &self.btn_icon,
            &self.btn_front,
            &self.btn_img,
            &self.menubar,
            &self.toolbar,
            &self.timeout_btn,
            &self.posgrid,
            &self.colorpick,
        ];
        // 섹션 라벨(각 컨트롤 위 — 컨트롤의 x에 맞춰 그린다: 좌·우 나란한 트리 쌍 라벨 분리).
        ctx.select_font(FontSlot::Status, false);
        for (label, w) in labels.iter().zip(widgets.iter()) {
            let b = w.bounds();
            let lr = Rect::new(b.x, b.y - self.s(LABEL_H), self.bounds.w, self.s(LABEL_H));
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
        if self.menubar.is_open() {
            self.menubar.paint(ctx, theme); // 풀다운 오버레이 — 아래 컨트롤 위에 겹침
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
