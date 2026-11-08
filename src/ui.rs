#![allow(unsafe_op_in_unsafe_fn)]
use std::{
    cell::RefCell,
    collections::HashMap,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
pub use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::LibraryLoader::*,
    UI::{
        Controls::Dialogs::*, Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*,
        WindowsAndMessaging::*,
    },
};
pub fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | g << 8 | b << 16
}
pub const TBM_GETPOS: u32 = WM_USER;
pub const SS_BLACKRECT: u32 = 4;
pub const MK_LBUTTON: usize = 1;
pub const MK_SHIFT: usize = 4;
#[derive(Clone, Copy)]
pub struct Colors {
    pub bg: u32,
    pub panel: u32,
    pub field: u32,
    pub fg: u32,
    pub muted: u32,
    pub line: u32,
    pub accent: u32,
    pub ink: u32,
    pub selection: u32,
}
impl Colors {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                bg: rgb(18, 18, 21),
                panel: rgb(25, 25, 29),
                field: rgb(34, 34, 40),
                fg: rgb(238, 238, 243),
                muted: rgb(150, 150, 163),
                line: rgb(49, 49, 59),
                accent: rgb(216, 216, 227),
                ink: rgb(25, 25, 31),
                selection: rgb(51, 51, 62),
            }
        } else {
            Self {
                bg: rgb(244, 244, 247),
                panel: rgb(255, 255, 255),
                field: rgb(233, 233, 239),
                fg: rgb(30, 30, 39),
                muted: rgb(107, 107, 122),
                line: rgb(210, 210, 221),
                accent: rgb(58, 58, 76),
                ink: rgb(255, 255, 255),
                selection: rgb(219, 219, 232),
            }
        }
    }
}
#[derive(Clone, Copy)]
struct Snapshot {
    colors: Colors,
    fonts: [HFONT; 4],
    brushes: [HBRUSH; 2],
    dpi: f64,
}
thread_local! {static SNAPSHOTS:RefCell<HashMap<usize,Snapshot>>=RefCell::new(HashMap::new());}
pub struct Ui {
    pub hwnd: HWND,
    pub dpi: f64,
    pub dark: bool,
    pub colors: Colors,
    pub fonts: [HFONT; 4],
    pub brushes: [HBRUSH; 2],
    pub controls: Vec<(usize, HWND)>,
}
impl Ui {
    pub unsafe fn new(hwnd: HWND, dark: bool) -> Self {
        let dpi = GetDpiForWindow(hwnd) as f64 / 96.;
        let colors = Colors::new(dark);
        let mut fonts = [null_mut(); 4];
        for (i, (h, weight, face)) in [
            (14, 400, "Segoe UI"),
            (27, 600, "Segoe UI"),
            (18, 600, "Segoe UI"),
            (17, 400, "Segoe MDL2 Assets"),
        ]
        .iter()
        .enumerate()
        {
            fonts[i] = CreateFontW(
                -(*h as f64 * dpi).round() as i32,
                0,
                0,
                0,
                *weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                w(face).as_ptr(),
            );
        }
        let mut ui = Self {
            hwnd,
            dpi,
            dark,
            colors,
            fonts,
            brushes: [CreateSolidBrush(colors.bg), CreateSolidBrush(colors.field)],
            controls: vec![],
        };
        ui.theme(dark);
        ui
    }
    pub fn s(&self, n: i32) -> i32 {
        (n as f64 * self.dpi).round() as i32
    }
    pub unsafe fn size(&self) -> (i32, i32) {
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        (
            (r.right as f64 / self.dpi).round() as i32,
            (r.bottom as f64 / self.dpi).round() as i32,
        )
    }
    pub unsafe fn update_dpi(&mut self, dpi: f64) {
        self.dpi = dpi;
        let old = self.fonts;
        for (i, (h, weight, face)) in [
            (14, 400, "Segoe UI"),
            (27, 600, "Segoe UI"),
            (18, 600, "Segoe UI"),
            (17, 400, "Segoe MDL2 Assets"),
        ]
        .iter()
        .enumerate()
        {
            self.fonts[i] = CreateFontW(
                -self.s(*h),
                0,
                0,
                0,
                *weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                w(face).as_ptr(),
            );
        }
        self.snapshot();
        for (_, h) in &self.controls {
            SendMessageW(*h, WM_SETFONT, self.fonts[0] as usize, 1);
        }
        for f in old {
            DeleteObject(f);
        }
    }
    fn snapshot(&self) {
        SNAPSHOTS.with(|x| {
            x.borrow_mut().insert(
                self.hwnd as usize,
                Snapshot {
                    colors: self.colors,
                    fonts: self.fonts,
                    brushes: self.brushes,
                    dpi: self.dpi,
                },
            )
        });
    }
    pub fn get(&self, id: usize) -> HWND {
        self.controls
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, h)| *h)
            .unwrap_or(null_mut())
    }
    pub unsafe fn add(&mut self, id: usize, class: &str, text: &str, style: u32) {
        let h = CreateWindowExW(
            0,
            w(class).as_ptr(),
            w(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | style,
            0,
            0,
            0,
            0,
            self.hwnd,
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        SendMessageW(h, WM_SETFONT, self.fonts[0] as usize, 1);
        self.controls.push((id, h));
        SetWindowTheme(
            h,
            w(if self.dark {
                "DarkMode_Explorer"
            } else {
                "Explorer"
            })
            .as_ptr(),
            null(),
        );
    }
    pub unsafe fn button(&mut self, id: usize, text: &str) {
        self.add(id, "BUTTON", text, BS_OWNERDRAW as u32);
    }
    pub unsafe fn edit(&mut self, id: usize, value: &str) {
        self.add(id, "EDIT", value, ES_AUTOHSCROLL as u32 | WS_BORDER);
        SendMessageW(self.get(id), EM_SETLIMITTEXT, 244, 0);
        let margin = self.s(10) as isize;
        SendMessageW(
            self.get(id),
            EM_SETMARGINS,
            (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
            margin | (margin << 16),
        );
    }
    pub unsafe fn value(&self, id: usize) -> String {
        let h = self.get(id);
        let mut value = vec![0; GetWindowTextLengthW(h).max(0) as usize + 1];
        let len = GetWindowTextW(h, value.as_mut_ptr(), value.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&value[..len])
    }
    pub unsafe fn slider(&mut self, id: usize, min: i32, max: i32, value: i32) {
        self.add(id, "msctls_trackbar32", "", TBS_NOTICKS);
        SendMessageW(self.get(id), TBM_SETRANGEMIN, 0, min as isize);
        SendMessageW(self.get(id), TBM_SETRANGEMAX, 1, max as isize);
        SendMessageW(self.get(id), TBM_SETPOS, 1, value as isize);
    }
    pub unsafe fn pos(&self, id: usize) -> i32 {
        SendMessageW(self.get(id), TBM_GETPOS, 0, 0) as i32
    }
    pub unsafe fn set_text(&self, id: usize, s: &str) {
        SetWindowTextW(self.get(id), w(s).as_ptr());
    }
    pub unsafe fn place(&self, id: usize, x: i32, y: i32, width: i32, height: i32) {
        MoveWindow(
            self.get(id),
            self.s(x),
            self.s(y),
            self.s(width),
            self.s(height),
            1,
        );
    }
    pub unsafe fn show(&self, id: usize, on: bool) {
        ShowWindow(self.get(id), if on { SW_SHOW } else { SW_HIDE });
    }
    pub unsafe fn enable(&self, id: usize, on: bool) {
        EnableWindow(self.get(id), i32::from(on));
    }
    pub unsafe fn theme(&mut self, dark: bool) {
        self.dark = dark;
        self.colors = Colors::new(dark);
        for b in self.brushes {
            DeleteObject(b);
        }
        self.brushes = [
            CreateSolidBrush(self.colors.bg),
            CreateSolidBrush(self.colors.field),
        ];
        self.snapshot();
        let value = i32::from(dark);
        DwmSetWindowAttribute(self.hwnd, 20, &value as *const _ as *const _, 4);
        for (_, h) in &self.controls {
            SetWindowTheme(
                *h,
                w(if dark {
                    "DarkMode_Explorer"
                } else {
                    "Explorer"
                })
                .as_ptr(),
                null(),
            );
            InvalidateRect(*h, null(), 1);
        }
        self.refresh();
    }
    pub unsafe fn refresh(&self) {
        InvalidateRect(self.hwnd, null(), 0);
    }
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn label(
        &self,
        dc: HDC,
        s: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        color: u32,
        font: usize,
    ) {
        text(
            dc,
            s,
            RECT {
                left: self.s(x),
                top: self.s(y),
                right: self.s(x + width),
                bottom: self.s(y + height),
            },
            color,
            self.fonts[font],
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn rect(
        &self,
        dc: HDC,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        radius: i32,
        fill_color: u32,
        border: u32,
    ) {
        round(
            dc,
            RECT {
                left: self.s(x),
                top: self.s(y),
                right: self.s(x + width),
                bottom: self.s(y + height),
            },
            self.s(radius),
            fill_color,
            border,
        );
    }
}
impl Drop for Ui {
    fn drop(&mut self) {
        SNAPSHOTS.with(|s| s.borrow_mut().remove(&(self.hwnd as usize)));
        unsafe {
            for f in self.fonts {
                DeleteObject(f);
            }
            for b in self.brushes {
                DeleteObject(b);
            }
        }
    }
}
pub unsafe fn text(dc: HDC, s: &str, mut r: RECT, color: u32, font: HFONT, flags: u32) {
    let old = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, color);
    DrawTextW(dc, w(s).as_ptr(), -1, &mut r, flags);
    SelectObject(dc, old);
}
pub unsafe fn fill(dc: HDC, r: RECT, color: u32) {
    let b = CreateSolidBrush(color);
    FillRect(dc, &r, b);
    DeleteObject(b);
}
pub unsafe fn round(dc: HDC, r: RECT, radius: i32, c: u32, border: u32) {
    let b = CreateSolidBrush(c);
    let pen = CreatePen(PS_SOLID, 1, border);
    let ob = SelectObject(dc, b);
    let op = SelectObject(dc, pen);
    RoundRect(dc, r.left, r.top, r.right, r.bottom, radius * 2, radius * 2);
    SelectObject(dc, ob);
    SelectObject(dc, op);
    DeleteObject(b);
    DeleteObject(pen);
}
pub unsafe fn paint(hwnd: HWND, mut painter: impl FnMut(HDC, i32, i32)) {
    let mut ps: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let mem = CreateCompatibleDC(dc);
    let dirty = ps.rcPaint;
    let ww = (dirty.right - dirty.left).max(1);
    let hh = (dirty.bottom - dirty.top).max(1);
    let bmp = CreateCompatibleBitmap(dc, ww, hh);
    let old = SelectObject(mem, bmp);
    SetViewportOrgEx(mem, -dirty.left, -dirty.top, null_mut());
    painter(mem, r.right, r.bottom);
    BitBlt(
        dc, dirty.left, dirty.top, ww, hh, mem, dirty.left, dirty.top, SRCCOPY,
    );
    SelectObject(mem, old);
    DeleteObject(bmp);
    DeleteDC(mem);
    EndPaint(hwnd, &ps);
}
/// Render the application's own hidden test window, including native child controls.
pub unsafe fn render_test(
    ui: &Ui,
    path: &std::path::Path,
    mut painter: impl FnMut(HDC, i32, i32),
) -> std::io::Result<()> {
    use std::io::Write;
    let mut rect: RECT = zeroed();
    GetClientRect(ui.hwnd, &mut rect);
    let width = rect.right;
    let height = rect.bottom;
    let dc = GetDC(ui.hwnd);
    let mem = CreateCompatibleDC(dc);
    let bmp = CreateCompatibleBitmap(dc, width, height);
    let old = SelectObject(mem, bmp);
    painter(mem, width, height);
    for (_, h) in &ui.controls {
        if GetWindowLongW(*h, GWL_STYLE) as u32 & WS_VISIBLE == 0 {
            continue;
        }
        let mut r: RECT = zeroed();
        GetWindowRect(*h, &mut r);
        let mut point = POINT {
            x: r.left,
            y: r.top,
        };
        MapWindowPoints(null_mut(), ui.hwnd, &mut point, 1);
        let saved = SaveDC(mem);
        SetViewportOrgEx(mem, point.x, point.y, null_mut());
        SendMessageW(
            *h,
            WM_PRINTCLIENT,
            mem as usize,
            (PRF_CLIENT | PRF_ERASEBKGND) as isize,
        );
        RestoreDC(mem, saved);
    }
    SetViewportOrgEx(mem, 0, 0, null_mut());
    SelectObject(mem, old);
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: 40,
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..zeroed()
    };
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    GetDIBits(
        mem,
        bmp,
        0,
        height as u32,
        pixels.as_mut_ptr() as *mut _,
        &mut info,
        DIB_RGB_COLORS,
    );
    let mut file = std::fs::File::create(path)?;
    file.write_all(b"BM")?;
    file.write_all(&(54 + pixels.len() as u32).to_le_bytes())?;
    file.write_all(&[0; 4])?;
    file.write_all(&54u32.to_le_bytes())?;
    let header = std::slice::from_raw_parts((&info.bmiHeader) as *const _ as *const u8, 40);
    file.write_all(header)?;
    file.write_all(&pixels)?;
    DeleteObject(bmp);
    DeleteDC(mem);
    ReleaseDC(ui.hwnd, dc);
    Ok(())
}
pub unsafe fn early(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let snap = SNAPSHOTS.with(|s| s.borrow().get(&(hwnd as usize)).copied())?;
    let p = snap.colors;
    let scale = |n: i32| (n as f64 * snap.dpi).round() as i32;
    match msg {
        WM_ERASEBKGND => Some(1),
        WM_MEASUREITEM => {
            (*(lp as *mut MEASUREITEMSTRUCT)).itemHeight = scale(31) as u32;
            Some(1)
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            SetBkColor(wp as HDC, p.field);
            SetTextColor(wp as HDC, p.fg);
            Some(snap.brushes[1] as isize)
        }
        WM_CTLCOLORBTN | WM_CTLCOLORSTATIC => {
            SetBkColor(wp as HDC, p.bg);
            SetTextColor(wp as HDC, p.fg);
            Some(snap.brushes[0] as isize)
        }
        WM_DRAWITEM => {
            let d = &*(lp as *const DRAWITEMSTRUCT);
            let combo = d.CtlType == ODT_COMBOBOX;
            let selected = d.itemState & ODS_SELECTED != 0;
            let disabled = d.itemState & ODS_DISABLED != 0;
            let label = if combo {
                if d.itemID == u32::MAX {
                    String::new()
                } else {
                    let n = SendMessageW(d.hwndItem, CB_GETLBTEXTLEN, d.itemID as usize, 0).max(0)
                        as usize;
                    let mut b = vec![0u16; n + 1];
                    SendMessageW(
                        d.hwndItem,
                        CB_GETLBTEXT,
                        d.itemID as usize,
                        b.as_mut_ptr() as isize,
                    );
                    String::from_utf16_lossy(&b[..n])
                }
            } else {
                let n = GetWindowTextLengthW(d.hwndItem).max(0) as usize;
                let mut b = vec![0u16; n + 1];
                GetWindowTextW(d.hwndItem, b.as_mut_ptr(), b.len() as i32);
                String::from_utf16_lossy(&b[..n])
            };
            if let Some(toggle) = label.strip_prefix('~') {
                fill(d.hDC, d.rcItem, p.bg);
                let on = toggle.ends_with(":on");
                let name = toggle.rsplit_once(':').map(|(s, _)| s).unwrap_or(toggle);
                let mut r = d.rcItem;
                r.right -= scale(65);
                let caption = if let Some((icon, caption)) = name.split_once('|') {
                    let mut ir = r;
                    ir.right = ir.left + scale(24);
                    text(
                        d.hDC,
                        icon,
                        ir,
                        if disabled { p.muted } else { p.fg },
                        snap.fonts[3],
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                    );
                    r.left += scale(34);
                    caption
                } else {
                    name
                };
                text(
                    d.hDC,
                    caption,
                    r,
                    if disabled { p.muted } else { p.fg },
                    snap.fonts[0],
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
                let x = d.rcItem.right - scale(48);
                let y = (d.rcItem.top + d.rcItem.bottom) / 2;
                let color = if on { p.accent } else { p.line };
                round(
                    d.hDC,
                    RECT {
                        left: x,
                        top: y - scale(11),
                        right: x + scale(44),
                        bottom: y + scale(11),
                    },
                    scale(11),
                    color,
                    color,
                );
                let cx = x + scale(if on { 32 } else { 12 });
                round(
                    d.hDC,
                    RECT {
                        left: cx - scale(8),
                        top: y - scale(8),
                        right: cx + scale(8),
                        bottom: y + scale(8),
                    },
                    scale(8),
                    if on { p.ink } else { p.fg },
                    color,
                );
            } else {
                let tile = label.starts_with('^');
                let label = label.trim_start_matches('^');
                let active = label.starts_with('=') || (tile && label.starts_with('!'));
                let primary = !tile && label.starts_with('!');
                let label = label.trim_start_matches(['!', '=']);
                let bg = if primary && !disabled {
                    p.accent
                } else if active && !disabled {
                    p.selection
                } else if selected {
                    p.line
                } else {
                    p.field
                };
                let fg = if disabled {
                    p.muted
                } else if primary {
                    p.ink
                } else {
                    p.fg
                };
                fill(d.hDC, d.rcItem, p.bg);
                let border = if active && !disabled { p.accent } else { bg };
                round(d.hDC, d.rcItem, scale(7), bg, border);
                let mut r = d.rcItem;
                r.left += scale(10);
                r.right -= scale(10);
                if let Some((icon, caption)) = label.split_once('|') {
                    let mut ir = r;
                    if tile {
                        ir.top += scale(8);
                        ir.bottom = ir.top + scale(26);
                        text(
                            d.hDC,
                            icon,
                            ir,
                            fg,
                            snap.fonts[3],
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                        );
                        r.top = ir.bottom + scale(3);
                        text(
                            d.hDC,
                            caption,
                            r,
                            fg,
                            snap.fonts[0],
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                        );
                    } else {
                        ir.right = ir.left + scale(24);
                        text(
                            d.hDC,
                            icon,
                            ir,
                            fg,
                            snap.fonts[3],
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                        );
                        r.left += scale(29);
                        text(
                            d.hDC,
                            caption,
                            r,
                            fg,
                            snap.fonts[0],
                            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                        );
                    }
                } else {
                    text(
                        d.hDC,
                        label,
                        r,
                        fg,
                        snap.fonts[0],
                        if combo {
                            DT_LEFT | DT_VCENTER | DT_SINGLELINE
                        } else {
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE
                        },
                    );
                }
            }
            if d.itemState & ODS_FOCUS != 0 {
                let mut r = d.rcItem;
                InflateRect(&mut r, -3, -3);
                DrawFocusRect(d.hDC, &r);
            }
            Some(1)
        }
        _ => None,
    }
}
#[allow(clippy::manual_dangling_ptr)] // Win32 MAKEINTRESOURCE(1), not a dereferenceable pointer.
pub unsafe fn register(name: &str, proc: WNDPROC) {
    let instance = GetModuleHandleW(null());
    let icon = LoadIconW(instance, 1usize as *const u16);
    let name = w(name);
    let c = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: icon,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: name.as_ptr(),
        hIconSm: icon,
    };
    RegisterClassExW(&c);
}
pub unsafe fn window(
    name: &str,
    title: &str,
    width: i32,
    height: i32,
    parent: HWND,
    param: *const std::ffi::c_void,
) -> HWND {
    let dpi = GetDpiForSystem();
    let style = WS_OVERLAPPED
        | WS_CAPTION
        | WS_SYSMENU
        | WS_MINIMIZEBOX
        | WS_CLIPCHILDREN
        | if parent.is_null() {
            WS_MAXIMIZEBOX | WS_THICKFRAME
        } else {
            0
        };
    let mut r = RECT {
        left: 0,
        top: 0,
        right: width * dpi as i32 / 96,
        bottom: height * dpi as i32 / 96,
    };
    AdjustWindowRectExForDpi(&mut r, style, 0, 0, dpi);
    CreateWindowExW(
        0,
        w(name).as_ptr(),
        w(title).as_ptr(),
        style,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        r.right - r.left,
        r.bottom - r.top,
        parent,
        null_mut(),
        GetModuleHandleW(null()),
        param,
    )
}
