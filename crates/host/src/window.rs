#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Asked {
    Load(String),
    Save(String),
    Delete(String),
}

#[cfg(windows)]
mod real {
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;

    use super::Asked;

    type Handle = *mut c_void;

    #[repr(C)]
    struct PaintHeld {
        dc: Handle,
        erase: i32,
        rect: [i32; 4],
        restore: i32,
        incomplete: i32,
        reserved: [u8; 32],
    }

    #[repr(C)]
    struct TrackMouse {
        size: u32,
        flags: u32,
        window: Handle,
        hover: u32,
    }

    #[repr(C)]
    struct Message {
        window: Handle,
        what: u32,
        first: usize,
        second: isize,
        time: u32,
        x: i32,
        y: i32,
        private: u32,
    }

    #[repr(C)]
    struct WindowClass {
        size: u32,
        style: u32,
        procedure: Option<unsafe extern "system" fn(Handle, u32, usize, isize) -> isize>,
        class_extra: i32,
        window_extra: i32,
        instance: Handle,
        icon: Handle,
        cursor: Handle,
        background: Handle,
        menu: *const u16,
        name: *const u16,
        small_icon: Handle,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassExW(class: *const WindowClass) -> u16;
        fn CreateWindowExW(
            extra: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: Handle,
            menu: Handle,
            instance: Handle,
            param: *mut c_void,
        ) -> Handle;
        fn DestroyWindow(window: Handle) -> i32;
        fn ShowWindow(window: Handle, how: i32) -> i32;
        fn DefWindowProcW(window: Handle, what: u32, first: usize, second: isize) -> isize;
        fn PeekMessageW(message: *mut Message, window: Handle, low: u32, high: u32, remove: u32) -> i32;
        fn TranslateMessage(message: *const Message) -> i32;
        fn DispatchMessageW(message: *const Message) -> isize;
        fn AdjustWindowRectEx(rect: *mut [i32; 4], style: u32, menu: i32, extra: u32) -> i32;
        fn GetClientRect(window: *mut c_void, rect: *mut [i32; 4]) -> i32;
        fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
        fn MoveWindow(window: Handle, x: i32, y: i32, width: i32, height: i32, repaint: i32) -> i32;
        fn SetWindowPos(window: Handle, after: Handle, x: i32, y: i32, width: i32, height: i32, how: u32) -> i32;
        fn SetForegroundWindow(window: Handle) -> i32;
        fn SetFocus(window: Handle) -> Handle;
        fn GetDpiForWindow(window: Handle) -> u32;
        fn GetSystemMetrics(which: i32) -> i32;
        fn FillRect(dc: Handle, rect: *const [i32; 4], brush: Handle) -> i32;
        fn DrawTextW(dc: Handle, text: *const u16, length: i32, rect: *mut [i32; 4], how: u32) -> i32;
        fn InvalidateRect(window: Handle, rect: *const [i32; 4], erase: i32) -> i32;
        fn BeginPaint(window: Handle, paint: *mut PaintHeld) -> Handle;
        fn EndPaint(window: Handle, paint: *const PaintHeld) -> i32;
        fn SetCapture(window: Handle) -> Handle;
        fn ReleaseCapture() -> i32;
        fn TrackMouseEvent(track: *mut TrackMouse) -> i32;
        fn GetWindowLongW(window: Handle, which: i32) -> isize;
        fn GetWindowRect(window: Handle, rect: *mut [i32; 4]) -> i32;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn GetStockObject(which: i32) -> Handle;
        fn CreateSolidBrush(colour: u32) -> Handle;
        fn SetTextColor(dc: Handle, colour: u32) -> u32;
        fn SetBkMode(dc: Handle, mode: i32) -> i32;
        fn CreatePen(style: i32, width: i32, colour: u32) -> Handle;
        #[allow(clippy::too_many_arguments)]
        fn CreateFontW(height: i32, width: i32, escape: i32, orient: i32, weight: i32, italic: u32, under: u32, strike: u32, set: u32, precision: u32, clip: u32, quality: u32, pitch: u32, face: *const u16) -> Handle;
        fn SelectObject(dc: Handle, object: Handle) -> Handle;
        fn MoveToEx(dc: Handle, x: i32, y: i32, was: *mut [i32; 2]) -> i32;
        fn LineTo(dc: Handle, x: i32, y: i32) -> i32;
        fn GetTextExtentPoint32W(dc: Handle, text: *const u16, length: i32, size: *mut [i32; 2]) -> i32;
        fn DeleteObject(object: Handle) -> i32;
        fn RoundRect(dc: Handle, left: i32, top: i32, right: i32, bottom: i32, across: i32, down: i32) -> i32;
    }

    #[link(name = "uxtheme")]
    extern "system" {
    }

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(window: Handle, attribute: u32, value: *const c_void, size: u32) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
    }

    const OVERLAPPED_WINDOW: u32 = 0x00CF_0000;
    const FIXED_WINDOW: u32 = 0x00CA_0000;
    const CLIP_CHILDREN: u32 = 0x0200_0000;
    const CHILD: u32 = 0x4000_0000;
    const VISIBLE: u32 = 0x1000_0000;
    const SHOW: i32 = 5;
    const GWL_STYLE: i32 = -16;
    const TOP: Handle = std::ptr::null_mut();
    const KEEP_SIZE: u32 = 0x0001;
    const KEEP_PLACE: u32 = 0x0002;
    const AND_SHOW: u32 = 0x0040;
    const REMOVE: u32 = 1;
    const CLOSE: u32 = 0x0010;
    const SIZE: u32 = 0x0005;
    const ARROW: usize = 32512;
    const GUI_FONT: i32 = 17;
    const DARK_MODE_BEFORE_2004: u32 = 19;
    const DARK_MODE: u32 = 20;
    const BORDER_COLOUR: u32 = 34;
    const CAPTION_COLOUR: u32 = 35;
    const CAPTION_TEXT_COLOUR: u32 = 36;
    const TRANSPARENT_BACKGROUND: i32 = 1;
    const CORNER: i32 = 7;
    const CHEVRON: i32 = 4;
    const POPUP: u32 = 0x8000_0000;
    const TOP_MOST_WINDOW: u32 = 0x0000_0008;
    const SHOW_NO_FOCUS: i32 = 4;
    const KILL_FOCUS: u32 = 0x0008;
    const NO_CLIP_ELLIPSIS: u32 = 0x0004_0000;
    const NAME_HINT: &str = "Name this preset";
    const NAME_WIDEST: i32 = 300;
    const TEXT_HEIGHT: i32 = 15;
    const FONT_FACE: &str = "Segoe UI";
    const PAINT_MESSAGE: u32 = 0x000F;
    const LEFT_DOWN: u32 = 0x0201;
    const LEFT_UP: u32 = 0x0202;
    const MOUSE_MOVED: u32 = 0x0200;
    const MOUSE_LEFT: u32 = 0x02A3;
    const CHARACTER: u32 = 0x0102;
    const LEAVE_WANTED: u32 = 0x0000_0002;
    const TEXT_INSET: i32 = 10;
    const CENTRED: u32 = 0x0001;
    const LEFT_ALIGNED: u32 = 0x0000;
    const MIDDLE: u32 = 0x0004;
    const ONE_LINE: u32 = 0x0020;
    const BAR: i32 = 44;
    const GAP: i32 = 9;
    const LIST_WIDTH: i32 = 260;
    const SAVE_WIDTH: i32 = 86;
    const DELETE_WIDTH: i32 = 86;
    const ROW: i32 = 30;
    const PICK: &str = "Presets";

    #[derive(Clone, Copy)]
    struct Parts {
        area: Handle,
    }

    thread_local! {
        static PARTS: Cell<Option<Parts>> = const { Cell::new(None) };
        static ASKED: RefCell<Vec<Asked>> = const { RefCell::new(Vec::new()) };
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }


    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Spot {
        Picker,
        Name,
        Save,
        Delete,
    }

    struct Bar {
        names: Vec<String>,
        chosen: Option<usize>,
        typed: String,
        typing: bool,
        hover: Option<Spot>,
        held: Option<Spot>,
        listing: bool,
        lit: Option<usize>,
    }

    impl Bar {
        const fn empty() -> Self {
            Self { names: Vec::new(), chosen: None, typed: String::new(), typing: false, hover: None, held: None, listing: false, lit: None }
        }

        fn showing(&self) -> String {
            match self.chosen.and_then(|at| self.names.get(at)) {
                Some(name) => name.clone(),
                None => PICK.to_string(),
            }
        }
    }

    thread_local! {
        static STRIP: RefCell<Bar> = const { RefCell::new(Bar::empty()) };
        static LIST_WINDOW: Cell<Handle> = const { Cell::new(std::ptr::null_mut()) };
        static FOLLOWING: RefCell<Option<Box<dyn Fn(i32, i32)>>> = const { RefCell::new(None) };
    }

    thread_local! {
        static SCALED: Cell<f32> = const { Cell::new(1.0) };
        static LETTERING: Cell<Handle> = const { Cell::new(std::ptr::null_mut()) };
    }

    unsafe fn measure_screen(window: Handle) {
        let dots = GetDpiForWindow(window);
        let scale = if dots == 0 { 1.0 } else { dots as f32 / 96.0 };
        SCALED.with(|held| held.set(scale));
        let was = LETTERING.with(Cell::get);
        if !was.is_null() {
            DeleteObject(was);
        }
        let face = wide(FONT_FACE);
        let made = CreateFontW(-grown(TEXT_HEIGHT), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
        LETTERING.with(|held| held.set(made));
    }

    fn grown(size: i32) -> i32 {
        let scale = SCALED.with(Cell::get);
        ((size as f32 * scale).round() as i32).max(1)
    }

    unsafe fn lettering() -> Handle {
        let found = LETTERING.with(Cell::get);
        if found.is_null() {
            GetStockObject(GUI_FONT)
        } else {
            found
        }
    }

    fn spots(width: i32) -> [(Spot, [i32; 4]); 4] {
        let (bar, row, gap) = (grown(BAR), grown(ROW), grown(GAP));
        let (picker, save_wide, delete_wide) = (grown(LIST_WIDTH), grown(SAVE_WIDTH), grown(DELETE_WIDTH));
        let top = (bar - row) / 2;
        let bottom = top + row;
        let save_at = width - gap - save_wide - gap - delete_wide;
        let delete_at = width - gap - delete_wide;
        let name_at = gap * 2 + picker;
        let name_to = (save_at - gap).min(name_at + grown(NAME_WIDEST)).max(name_at + grown(60));
        [
            (Spot::Picker, [gap, top, gap + picker, bottom]),
            (Spot::Name, [name_at, top, name_to, bottom]),
            (Spot::Save, [save_at, top, save_at + save_wide, bottom]),
            (Spot::Delete, [delete_at, top, delete_at + delete_wide, bottom]),
        ]
    }

    fn spot_at(width: i32, x: i32, y: i32) -> Option<Spot> {
        spots(width)
            .into_iter()
            .find(|(_, rect)| x >= rect[0] && x < rect[2] && y >= rect[1] && y < rect[3])
            .map(|(spot, _)| spot)
    }

    unsafe fn text_out(dc: Handle, text: &str, rect: [i32; 4], colour: u32, centred: bool) {
        let wide_text = wide(text);
        let mut area = rect;
        if !centred {
            area[0] += grown(TEXT_INSET);
            area[2] -= grown(TEXT_INSET);
        }
        SetTextColor(dc, colour);
        SetBkMode(dc, TRANSPARENT_BACKGROUND);
        let how = if centred { CENTRED } else { LEFT_ALIGNED } | MIDDLE | ONE_LINE | NO_CLIP_ELLIPSIS;
        DrawTextW(dc, wide_text.as_ptr(), (wide_text.len() - 1) as i32, &mut area, how);
    }

    unsafe fn panel_in(dc: Handle, rect: [i32; 4], fill: u32, edge: u32) {
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(0, 1, edge);
        let was_brush = SelectObject(dc, brush);
        let was_pen = SelectObject(dc, pen);
        let round = grown(CORNER);
        RoundRect(dc, rect[0], rect[1], rect[2], rect[3], round, round);
        SelectObject(dc, was_pen);
        SelectObject(dc, was_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }

    unsafe fn chevron(dc: Handle, rect: [i32; 4], colour: u32) {
        let pen = CreatePen(0, 1, colour);
        let was_pen = SelectObject(dc, pen);
        let middle = (rect[1] + rect[3]) / 2;
        let right = rect[2] - grown(TEXT_INSET);
        let tick = grown(CHEVRON);
        for step in 0..tick {
            MoveToEx(dc, right - tick * 2 + step, middle - tick / 2 + step, std::ptr::null_mut());
            LineTo(dc, right - tick * 2 + step + 1, middle - tick / 2 + step + 1);
            MoveToEx(dc, right - step, middle - tick / 2 + step, std::ptr::null_mut());
            LineTo(dc, right - step - 1, middle - tick / 2 + step + 1);
        }
        SelectObject(dc, was_pen);
        DeleteObject(pen);
    }

    const LIST_CLASS: &str = "LoupePresetList";
    const ROW_TALL: i32 = 32;
    const LONGEST_NAME: usize = 48;

    unsafe extern "system" fn list_proc(window: Handle, what: u32, first: usize, second: isize) -> isize {
        match what {
            PAINT_MESSAGE => {
                let shade = paint();
                let mut held = std::mem::zeroed::<PaintHeld>();
                let dc = BeginPaint(window, &mut held);
                let mut whole = [0i32; 4];
                GetClientRect(window, &mut whole);
                FillRect(dc, &whole, shade.panel);
                let was_font = SelectObject(dc, lettering());
                STRIP.with(|kept| {
                    let bar = kept.borrow();
                    for (at, name) in bar.names.iter().enumerate() {
                        let row = [1, 1 + at as i32 * grown(ROW_TALL), whole[2] - 1, 1 + (at as i32 + 1) * grown(ROW_TALL)];
                        if bar.lit == Some(at) {
                            FillRect(dc, &row, shade.accent_brush);
                        }
                        let colour = if bar.lit == Some(at) { shade.on_accent } else { shade.text };
                        text_out(dc, name, row, colour, false);
                    }
                    if bar.names.is_empty() {
                        text_out(dc, "No presets saved yet", [1, 1, whole[2] - 1, 1 + grown(ROW_TALL)], shade.dim, false);
                    }
                });
                SelectObject(dc, was_font);
                EndPaint(window, &held);
                return 0;
            }
            MOUSE_MOVED => {
                let y = ((second >> 16) & 0xFFFF) as i16 as i32;
                let over = usize::try_from((y - 1) / grown(ROW_TALL)).ok();
                let changed = STRIP.with(|kept| {
                    let mut bar = kept.borrow_mut();
                    let lit = over.filter(|at| *at < bar.names.len());
                    let was = bar.lit;
                    bar.lit = lit;
                    was != lit
                });
                if changed {
                    redraw(window);
                }
                return 0;
            }
            LEFT_UP => {
                let y = ((second >> 16) & 0xFFFF) as i16 as i32;
                let picked = usize::try_from((y - 1) / grown(ROW_TALL)).ok();
                let asked = STRIP.with(|kept| {
                    let mut bar = kept.borrow_mut();
                    let at = picked.filter(|at| *at < bar.names.len())?;
                    bar.chosen = Some(at);
                    bar.names.get(at).cloned().map(Asked::Load)
                });
                if let Some(asked) = asked {
                    ASKED.with(|kept| kept.borrow_mut().push(asked));
                }
                shut_list();
                return 0;
            }
            KILL_FOCUS => {
                shut_list();
                return 0;
            }
            _ => {}
        }
        DefWindowProcW(window, what, first, second)
    }

    unsafe fn shut_list() {
        let window = LIST_WINDOW.with(Cell::get);
        if !window.is_null() {
            LIST_WINDOW.with(|held| held.set(std::ptr::null_mut()));
            DestroyWindow(window);
        }
        STRIP.with(|held| {
            let mut bar = held.borrow_mut();
            bar.listing = false;
            bar.lit = None;
        });
    }

    unsafe fn open_list(parent: Handle) {
        if !LIST_WINDOW.with(Cell::get).is_null() {
            shut_list();
            return;
        }
        let rows = STRIP.with(|held| held.borrow().names.len().max(1)) as i32;
        let mut where_it_is = [0i32; 4];
        GetWindowRect(parent, &mut where_it_is);
        let mut inside = [0i32; 4];
        GetClientRect(parent, &mut inside);
        let edge = (where_it_is[2] - where_it_is[0] - inside[2]) / 2;
        let top = where_it_is[3] - (inside[3] - grown(BAR)) - edge - (grown(BAR) - grown(ROW)) / 2 - grown(ROW);
        let name = wide(LIST_CLASS);
        let instance = GetModuleHandleW(std::ptr::null());
        register(&name, list_proc, instance);
        let made = CreateWindowExW(
            TOP_MOST_WINDOW,
            name.as_ptr(),
            std::ptr::null(),
            POPUP | CLIP_CHILDREN,
            where_it_is[0] + edge + grown(GAP),
            top + grown(ROW),
            grown(LIST_WIDTH),
            rows * grown(ROW_TALL) + 2,
            parent,
            std::ptr::null_mut(),
            instance,
            std::ptr::null_mut(),
        );
        if made.is_null() {
            return;
        }
        LIST_WINDOW.with(|held| held.set(made));
        STRIP.with(|held| held.borrow_mut().listing = true);
        ShowWindow(made, SHOW_NO_FOCUS);
        SetFocus(made);
    }

    unsafe fn draw_bar(window: Handle, dc: Handle) {
        let shade = paint();
        let mut whole = [0i32; 4];
        GetClientRect(window, &mut whole);
        let width = whole[2];
        let bar_area = [0, 0, width, grown(BAR)];
        FillRect(dc, &bar_area, shade.panel);
        let line = [0, grown(BAR) - 1, width, grown(BAR)];
        FillRect(dc, &line, shade.edge_brush);
        let was_font = SelectObject(dc, lettering());
        STRIP.with(|held| {
            let bar = held.borrow();
            for (spot, rect) in spots(width) {
                let hovered = bar.hover == Some(spot) && bar.held.is_none();
                let pushed = bar.held == Some(spot);
                match spot {
                    Spot::Picker => {
                        let edge = if hovered || bar.listing { shade.accent_colour } else { shade.line_colour };
                        panel_in(dc, rect, shade.field_colour, edge);
                        let said = bar.showing();
                        let colour = if bar.chosen.is_some() { shade.text } else { shade.dim };
                        text_out(dc, &said, [rect[0], rect[1], rect[2] - CHEVRON * 3, rect[3]], colour, false);
                        chevron(dc, rect, shade.dim);
                    }
                    Spot::Name => {
                        let edge = if bar.typing { shade.accent_colour } else if hovered { shade.line_colour } else { shade.line_colour };
                        panel_in(dc, rect, shade.field_colour, edge);
                        if bar.typed.is_empty() && !bar.typing {
                            text_out(dc, NAME_HINT, rect, shade.dim, false);
                        } else {
                            text_out(dc, &bar.typed, rect, shade.text, false);
                            if bar.typing {
                                let width_of = measured(dc, &bar.typed);
                                let caret = rect[0] + TEXT_INSET + width_of;
                                let bar_rect = [caret, rect[1] + 4, caret + 1, rect[3] - 4];
                                FillRect(dc, &bar_rect, shade.caret);
                            }
                        }
                    }
                    Spot::Save | Spot::Delete => {
                        let can = if spot == Spot::Save { !bar.typed.trim().is_empty() } else { bar.chosen.is_some() };
                        let fill = if !can {
                            shade.panel_colour
                        } else if pushed {
                            shade.accent_colour
                        } else if hovered {
                            shade.line_colour
                        } else {
                            shade.field_colour
                        };
                        let edge = if can { shade.line_colour } else { shade.panel_colour };
                        panel_in(dc, rect, fill, edge);
                        let colour = if !can {
                            shade.dim
                        } else if pushed {
                            shade.on_accent
                        } else {
                            shade.text
                        };
                        text_out(dc, if spot == Spot::Save { "Save" } else { "Delete" }, rect, colour, true);
                    }
                }
            }
        });
        SelectObject(dc, was_font);
    }

    unsafe fn measured(dc: Handle, text: &str) -> i32 {
        let wide_text = wide(text);
        let mut size = [0i32; 2];
        GetTextExtentPoint32W(dc, wide_text.as_ptr(), (wide_text.len() - 1) as i32, size.as_mut_ptr() as *mut [i32; 2]);
        size[0]
    }

    unsafe fn redraw(window: Handle) {
        InvalidateRect(window, std::ptr::null(), 0);
    }

    unsafe extern "system" fn handle(window: Handle, what: u32, first: usize, second: isize) -> isize {
        match what {
            CLOSE => {
                ShowWindow(window, 0);
                return 0;
            }
            SIZE => {
                if let Some(parts) = PARTS.with(Cell::get) {
                    let width = (second & 0xFFFF) as i32;
                    let height = ((second >> 16) & 0xFFFF) as i32;
                    MoveWindow(parts.area, 0, grown(BAR), width, (height - grown(BAR)).max(0), 1);
                    FOLLOWING.with(|held| {
                        if let Some(follow) = held.borrow().as_ref() {
                            follow(width, (height - grown(BAR)).max(0));
                        }
                    });
                }
            }
            PAINT_MESSAGE => {
                let mut held = std::mem::zeroed::<PaintHeld>();
                let dc = BeginPaint(window, &mut held);
                draw_bar(window, dc);
                EndPaint(window, &held);
                return 0;
            }
            MOUSE_MOVED => {
                let mut whole = [0i32; 4];
                GetClientRect(window, &mut whole);
                let (x, y) = ((second & 0xFFFF) as i16 as i32, ((second >> 16) & 0xFFFF) as i16 as i32);
                let over = spot_at(whole[2], x, y);
                let changed = STRIP.with(|held| {
                    let mut bar = held.borrow_mut();
                    let was = bar.hover;
                    bar.hover = over;
                    was != over
                });
                if changed {
                    let mut track = TrackMouse { size: std::mem::size_of::<TrackMouse>() as u32, flags: LEAVE_WANTED, window, hover: 0 };
                    TrackMouseEvent(&mut track);
                    redraw(window);
                }
                return 0;
            }
            MOUSE_LEFT => {
                STRIP.with(|held| held.borrow_mut().hover = None);
                redraw(window);
                return 0;
            }
            LEFT_DOWN => {
                let mut whole = [0i32; 4];
                GetClientRect(window, &mut whole);
                let (x, y) = ((second & 0xFFFF) as i16 as i32, ((second >> 16) & 0xFFFF) as i16 as i32);
                let hit = spot_at(whole[2], x, y);
                STRIP.with(|held| {
                    let mut bar = held.borrow_mut();
                    bar.typing = hit == Some(Spot::Name);
                    bar.held = hit.filter(|spot| matches!(spot, Spot::Save | Spot::Delete));
                });
                if hit.is_some() {
                    SetCapture(window);
                    SetFocus(window);
                }
                if hit == Some(Spot::Picker) {
                    open_list(window);
                }
                redraw(window);
                return 0;
            }
            LEFT_UP => {
                let mut whole = [0i32; 4];
                GetClientRect(window, &mut whole);
                let (x, y) = ((second & 0xFFFF) as i16 as i32, ((second >> 16) & 0xFFFF) as i16 as i32);
                let hit = spot_at(whole[2], x, y);
                ReleaseCapture();
                let asked = STRIP.with(|held| {
                    let mut bar = held.borrow_mut();
                    let was = bar.held.take();
                    if was != hit {
                        return None;
                    }
                    match hit {
                        Some(Spot::Save) if !bar.typed.trim().is_empty() => Some(Asked::Save(bar.typed.trim().to_string())),
                        Some(Spot::Delete) => bar.chosen.and_then(|at| bar.names.get(at)).cloned().map(Asked::Delete),
                        _ => None,
                    }
                });
                if let Some(asked) = asked {
                    ASKED.with(|held| held.borrow_mut().push(asked));
                }
                redraw(window);
                return 0;
            }
            CHARACTER => {
                let typed = char::from_u32(first as u32);
                let acted = STRIP.with(|held| {
                    let mut bar = held.borrow_mut();
                    if !bar.typing {
                        return false;
                    }
                    match typed {
                        Some('\u{8}') => {
                            bar.typed.pop();
                            true
                        }
                        Some('\r') | Some('\n') => false,
                        Some(found) if !found.is_control() && bar.typed.chars().count() < LONGEST_NAME => {
                            bar.typed.push(found);
                            true
                        }
                        _ => false,
                    }
                });
                let entered = matches!(typed, Some('\r') | Some('\n'))
                    && STRIP.with(|held| {
                        let bar = held.borrow();
                        bar.typing && !bar.typed.trim().is_empty()
                    });
                if entered {
                    let name = STRIP.with(|held| held.borrow().typed.trim().to_string());
                    ASKED.with(|held| held.borrow_mut().push(Asked::Save(name)));
                }
                if acted || entered {
                    redraw(window);
                    return 0;
                }
            }
            _ => {}
        }
        DefWindowProcW(window, what, first, second)
    }

    unsafe extern "system" fn plain(window: Handle, what: u32, first: usize, second: isize) -> isize {
        DefWindowProcW(window, what, first, second)
    }

    thread_local! {
        static PAINT: Cell<Option<Paint>> = const { Cell::new(None) };
    }

    #[derive(Clone, Copy)]
    struct Paint {
        accent_brush: Handle,
        edge_brush: Handle,
        accent_colour: u32,
        caret: Handle,
        line_colour: u32,
        on_accent: u32,
        panel: Handle,
        text: u32,
        dim: u32,
        panel_colour: u32,
        field_colour: u32,
    }

    fn paint() -> Paint {
        if let Some(found) = PAINT.with(Cell::get) {
            return found;
        }
        let chrome = loupe_plugins::chrome::Chrome::from_the_app();
        let order = loupe_plugins::chrome::Chrome::windows_order;
        let made = unsafe {
            Paint {
                panel: CreateSolidBrush(order(chrome.panel)),
                accent_brush: CreateSolidBrush(order(chrome.accent)),
                edge_brush: CreateSolidBrush(order(chrome.line)),
                accent_colour: order(chrome.accent),
                caret: CreateSolidBrush(order(chrome.text)),
                line_colour: order(chrome.line),
                on_accent: order(0xFFFFFF),
                text: order(chrome.text),
                dim: order(chrome.text_dim),
                panel_colour: order(chrome.panel),
                field_colour: order(chrome.background),
            }
        };
        PAINT.with(|held| held.set(Some(made)));
        made
    }

    unsafe fn wear_dark(window: Handle) {
        let on: i32 = 1;
        let chrome = loupe_plugins::chrome::Chrome::from_the_app();
        let order = loupe_plugins::chrome::Chrome::windows_order;
        for attribute in [DARK_MODE_BEFORE_2004, DARK_MODE] {
            DwmSetWindowAttribute(window, attribute, &on as *const i32 as *const c_void, 4);
        }
        let caption = order(chrome.panel);
        let border = order(chrome.line);
        let writing = order(chrome.text);
        DwmSetWindowAttribute(window, CAPTION_COLOUR, &caption as *const u32 as *const c_void, 4);
        DwmSetWindowAttribute(window, BORDER_COLOUR, &border as *const u32 as *const c_void, 4);
        DwmSetWindowAttribute(window, CAPTION_TEXT_COLOUR, &writing as *const u32 as *const c_void, 4);
    }

    unsafe fn register(name: &[u16], procedure: unsafe extern "system" fn(Handle, u32, usize, isize) -> isize, instance: Handle) {
        let class = WindowClass {
            size: std::mem::size_of::<WindowClass>() as u32,
            style: 0,
            procedure: Some(procedure),
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: std::ptr::null_mut(),
            cursor: LoadCursorW(std::ptr::null_mut(), ARROW as *const u16),
            background: paint().panel,
            menu: std::ptr::null(),
            name: name.as_ptr(),
            small_icon: std::ptr::null_mut(),
        };
        RegisterClassExW(&class);
    }

    pub struct Window {
        handle: Handle,
        parts: Parts,
    }

    impl Window {
        pub fn open(title: &str, width: i32, height: i32, resizable: bool) -> Result<Self, String> {
            let frame = if resizable { OVERLAPPED_WINDOW } else { FIXED_WINDOW } | CLIP_CHILDREN;
            let name = wide("LoupePluginWindow");
            let area_name = wide("LoupePluginArea");
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                register(&name, handle, instance);
                register(&area_name, plain, instance);
                let mut rect = [0, 0, width, height + grown(BAR)];
                AdjustWindowRectEx(&mut rect, frame, 0, 0);
                let title = wide(title);
                let handle = CreateWindowExW(
                    0,
                    name.as_ptr(),
                    title.as_ptr(),
                    frame,
                    i32::MIN,
                    i32::MIN,
                    rect[2] - rect[0],
                    rect[3] - rect[1],
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null_mut(),
                );
                if handle.is_null() {
                    return Err("Loupe could not open a window for the plugin".into());
                }
                measure_screen(handle);
                let mut fitted = [0, 0, width, height + grown(BAR)];
                AdjustWindowRectEx(&mut fitted, frame, 0, 0);
                SetWindowPos(handle, TOP, 0, 0, fitted[2] - fitted[0], fitted[3] - fitted[1], KEEP_PLACE);
                wear_dark(handle);
                let area = CreateWindowExW(0, area_name.as_ptr(), std::ptr::null(), CHILD | VISIBLE | CLIP_CHILDREN, 0, grown(BAR), width, height, handle, std::ptr::null_mut(), instance, std::ptr::null_mut());
                if area.is_null() {
                    DestroyWindow(handle);
                    return Err("Loupe could not open a window for the plugin".into());
                }
                let parts = Parts { area };
                PARTS.with(|held| held.set(Some(parts)));
                Ok(Self { handle, parts })
            }
        }

        pub fn inner(&self) -> *mut c_void {
            self.parts.area
        }

        pub fn show(&self) {
            unsafe {
                ShowWindow(self.handle, SHOW);
                SetWindowPos(self.handle, TOP, 0, 0, 0, 0, KEEP_PLACE | KEEP_SIZE | AND_SHOW);
                SetForegroundWindow(self.handle);
            }
        }

        pub fn hide(&self) {
            unsafe {
                ShowWindow(self.handle, 0);
            }
        }

        pub fn screen_scale(&self) -> f32 {
            unsafe { measure_screen(self.handle) };
            SCALED.with(Cell::get)
        }

        pub fn screen_size(&self) -> (i32, i32) {
            unsafe { (GetSystemMetrics(0), GetSystemMetrics(1)) }
        }

        pub fn fit_around(&self, width: i32, height: i32) {
            unsafe {
                let mut rect = [0, 0, width, height + grown(BAR)];
                let style = GetWindowLongW(self.handle, GWL_STYLE) as u32;
                AdjustWindowRectEx(&mut rect, style, 0, 0);
                let mut where_it_is = [0i32; 4];
                GetWindowRect(self.handle, &mut where_it_is);
                MoveWindow(self.handle, where_it_is[0], where_it_is[1], rect[2] - rect[0], rect[3] - rect[1], 1);
            }
        }

        pub fn inside(&self) -> (i32, i32) {
            unsafe {
                let mut rect = [0i32; 4];
                if GetClientRect(self.parts.area, &mut rect) == 0 {
                    return (0, 0);
                }
                (rect[2] - rect[0], rect[3] - rect[1])
            }
        }

        pub fn presets(&self, names: &[String], chosen: Option<&str>) {
            STRIP.with(|held| {
                let mut bar = held.borrow_mut();
                bar.chosen = chosen.and_then(|wanted| names.iter().position(|name| name == wanted));
                bar.names = names.to_vec();
            });
            unsafe { redraw(self.handle) };
        }

        pub fn follow(&self, resize: Box<dyn Fn(i32, i32)>) {
            FOLLOWING.with(|held| *held.borrow_mut() = Some(resize));
        }

        pub fn clear_name(&self) {
            STRIP.with(|held| {
                let mut bar = held.borrow_mut();
                bar.typed.clear();
                bar.typing = false;
            });
            unsafe { redraw(self.handle) };
        }

        pub fn pump(&self) -> Vec<Asked> {
            unsafe {
                let mut message: Message = std::mem::zeroed();
                while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            ASKED.with(|held| std::mem::take(&mut *held.borrow_mut()))
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            PARTS.with(|held| held.set(None));
            FOLLOWING.with(|held| *held.borrow_mut() = None);
            unsafe { shut_list() };
            unsafe {
                DestroyWindow(self.handle);
            }
        }
    }
}

#[cfg(not(windows))]
mod real {
    use std::ffi::c_void;

    use super::Asked;

    pub struct Window;

    impl Window {
        pub fn open(_title: &str, _width: i32, _height: i32, _resizable: bool) -> Result<Self, String> {
            Err("plugin windows only open on Windows so far".into())
        }

        pub fn inner(&self) -> *mut c_void {
            std::ptr::null_mut()
        }

        pub fn show(&self) {}

        pub fn hide(&self) {}

        pub fn screen_scale(&self) -> f32 {
            1.0
        }

        pub fn screen_size(&self) -> (i32, i32) {
            (1920, 1080)
        }

        pub fn fit_around(&self, _width: i32, _height: i32) {}

        pub fn inside(&self) -> (i32, i32) {
            (0, 0)
        }

        pub fn presets(&self, _names: &[String], _chosen: Option<&str>) {}

        pub fn follow(&self, _resize: Box<dyn Fn(i32, i32)>) {}

        pub fn clear_name(&self) {}

        pub fn pump(&self) -> Vec<Asked> {
            Vec::new()
        }
    }
}

pub use real::Window;
