// Linggo Win32: foreground/fullscreen detection, focus restore, virtual screen capture (GDI -> DXGI fallback),
// window enumeration (windows_at) for snip auto-detect under cursor.

use serde::Serialize;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetDC, GetDIBits, GetMonitorInfoW, MonitorFromPoint, ReleaseDC, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetWindowLongW, GetWindowRect,
    GetWindowThreadProcessId, SetForegroundWindow, GWL_STYLE,
};

pub struct ScreenShot {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub rgba: Vec<u8>,
}

#[derive(Serialize, Clone)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

pub fn foreground_hwnd() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

pub fn process_name_of(hwnd: isize) -> String {
    use windows::Win32::Foundation::{CloseHandle, HMODULE};
    use windows::Win32::System::ProcessStatus::GetModuleBaseNameW;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid)); }
    if pid == 0 { return String::new(); }
    unsafe {
        let h = match OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) {
            Ok(h) => h,
            Err(_) => return String::new(),
        };
        let mut buf = [0u16; 260];
        let n = GetModuleBaseNameW(h, HMODULE::default(), &mut buf);
        let _ = CloseHandle(h);
        if n == 0 { return String::new(); }
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

pub fn cursor_position() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    if unsafe { GetCursorPos(&mut p) }.is_ok() {
        Some((p.x, p.y))
    } else {
        None
    }
}

pub fn work_area_at(x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO::default();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let r = &mi.rcWork;
            Some((r.left, r.top, r.right, r.bottom))
        } else {
            None
        }
    }
}

pub fn focus_restore(hwnd: isize) {
    if hwnd == 0 { return; }
    unsafe {
        let target = HWND(hwnd as *mut _);
        let fg = GetForegroundWindow();
        if fg.0 == target.0 { return; }
        let target_tid = GetWindowThreadProcessId(target, None);
        let current_tid = windows::Win32::System::Threading::GetCurrentThreadId();
        if target_tid == 0 || current_tid == 0 {
            let _ = SetForegroundWindow(target);
            return;
        }
        let _ = windows::Win32::System::Threading::AttachThreadInput(current_tid, target_tid, true);
        let _ = SetForegroundWindow(target);
        let _ = windows::Win32::System::Threading::AttachThreadInput(current_tid, target_tid, false);
    }
}

pub fn is_game_fullscreen(hwnd: isize) -> bool {
    unsafe {
        let h = HWND(hwnd as *mut _);
        let mut wr = RECT::default();
        if GetWindowRect(h, &mut wr).is_err() { return false; }
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        if style & 0x00C0_0000 != 0 { return false; } // WS_CAPTION -> browser fullscreen
        let (mx, my) = match cursor_position() {
            Some(p) => p,
            None => (wr.left + (wr.right - wr.left) / 2, wr.top + (wr.bottom - wr.top) / 2),
        };
        let hmon = MonitorFromPoint(POINT { x: mx, y: my }, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO::default();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if !GetMonitorInfoW(hmon, &mut mi).as_bool() { return false; }
        let wa = &mi.rcWork;
        let ww = wr.right - wr.left;
        let wh = wr.bottom - wr.top;
        let aw = wa.right - wa.left;
        let ah = wa.bottom - wa.top;
        ww >= aw - 4 && wh >= ah - 4
    }
}

pub fn capture_virtual_screen() -> Option<ScreenShot> {
    let (shot, is_black) = capture_virtual_screen_gdi();
    if let Some(s) = shot {
        if !is_black { return Some(s); }
        if let Some(dxgi) = capture_virtual_screen_dxgi() { return Some(dxgi); }
        return Some(s);
    }
    capture_virtual_screen_dxgi()
}

fn capture_virtual_screen_gdi() -> (Option<ScreenShot>, bool) {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, SelectObject,
        HGDIOBJ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if w <= 0 || h <= 0 { return (None, false); }

        let hdc_screen = GetDC(HWND::default());
        if hdc_screen.is_invalid() { return (None, false); }
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        if hdc_mem.is_invalid() { let _ = ReleaseDC(HWND::default(), hdc_screen); return (None, false); }
        let hbmp = CreateCompatibleBitmap(hdc_screen, w, h);
        if hbmp.is_invalid() { let _ = DeleteDC(hdc_mem); let _ = ReleaseDC(HWND::default(), hdc_screen); return (None, false); }
        let old = SelectObject(hdc_mem, HGDIOBJ::from(hbmp));
        let _ = BitBlt(hdc_mem, 0, 0, w, h, hdc_screen, x, y, SRCCOPY);

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
        let _ = GetDIBits(hdc_mem, hbmp, 0, h as u32, Some(rgba.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);

        let _ = SelectObject(hdc_mem, old);
        let _ = DeleteObject(HGDIOBJ::from(hbmp));
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(HWND::default(), hdc_screen);

        let mut black_count: u64 = 0;
        let total = (w as u64) * (h as u64);
        for chunk in rgba.chunks_exact_mut(4) {
            let b = chunk[0]; let g = chunk[1]; let r = chunk[2];
            chunk[0] = r; chunk[1] = g; chunk[2] = b; chunk[3] = 255;
            if r == 0 && g == 0 && b == 0 { black_count += 1; }
        }
        let is_black = total > 0 && (black_count as f64 / total as f64) > 0.995;
        (Some(ScreenShot { left: x, top: y, width: w, height: h, rgba }), is_black)
    }
}

fn capture_virtual_screen_dxgi() -> Option<ScreenShot> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput1, IDXGIResource,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ,
        D3D11_BIND_FLAG, D3D11_RESOURCE_MISC_FLAG, D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
    use windows::core::Interface;

    unsafe {
        let factory: IDXGIFactory1 = match CreateDXGIFactory1() { Ok(f) => f, Err(_) => return None };

        struct OutInfo { ai: u32, oi: u32, left: i32, top: i32, w: u32, h: u32 }
        let mut infos: Vec<OutInfo> = Vec::new();
        let (mut mr, mut mb, mut ml, mut mt) = (0i32, 0i32, i32::MAX, i32::MAX);

        for ai in 0.. {
            let adapter = match factory.EnumAdapters1(ai) { Ok(a) => a, Err(_) => break };
            for oi in 0.. {
                let output = match adapter.EnumOutputs(oi) { Ok(o) => o, Err(_) => break };
                let desc = match output.GetDesc() { Ok(d) => d, Err(_) => continue };
                if desc.Monitor == windows::Win32::Graphics::Gdi::HMONITOR::default() { continue; }
                let dm = desc.DesktopCoordinates;
                let (l, t) = (dm.left, dm.top);
                let (w, h) = ((dm.right - dm.left) as u32, (dm.bottom - dm.top) as u32);
                if w == 0 || h == 0 { continue; }
                ml = ml.min(l); mt = mt.min(t);
                mr = mr.max(l + w as i32); mb = mb.max(t + h as i32);
                infos.push(OutInfo { ai, oi, left: l, top: t, w, h });
            }
        }
        if infos.is_empty() { return None; }
        let (vx, vy) = (ml, mt);
        let (vw, vh) = ((mr - ml) as u32, (mb - mt) as u32);
        if vw == 0 || vh == 0 { return None; }

        let mut composite = vec![0u8; (vw as usize) * (vh as usize) * 4];
        let mut cur_ai: u32 = u32::MAX;
        let mut dev: Option<ID3D11Device> = None;
        let mut ctx: Option<ID3D11DeviceContext> = None;

        for info in &infos {
            if info.ai != cur_ai {
                dev = None; ctx = None;
                let adapter = match factory.EnumAdapters1(info.ai) { Ok(a) => a, Err(_) => continue };
                let (mut d, mut c): (Option<ID3D11Device>, Option<ID3D11DeviceContext>) = (None, None);
                let _ = D3D11CreateDevice(
                    &adapter, D3D_DRIVER_TYPE_UNKNOWN, None,
                    D3D11_CREATE_DEVICE_FLAG(0),
                    None, D3D11_SDK_VERSION,
                    Some(&mut d), None, Some(&mut c),
                );
                dev = d; ctx = c; cur_ai = info.ai;
            }
            let dev = match &dev { Some(d) => d, None => continue };
            let ctx = match &ctx { Some(c) => c, None => continue };
            let adapter = match factory.EnumAdapters1(info.ai) { Ok(a) => a, Err(_) => continue };
            let output = match adapter.EnumOutputs(info.oi) { Ok(o) => o, Err(_) => continue };
            let output1: IDXGIOutput1 = match output.cast() { Ok(o) => o, Err(_) => continue };
            let dup = match output1.DuplicateOutput(dev) { Ok(d) => d, Err(_) => continue };

            let mut resource: Option<IDXGIResource> = None;
            for _ in 0..3 {
                match dup.AcquireNextFrame(500, &mut Default::default(), &mut resource) {
                    Ok(()) => break,
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
            let resource = match resource {
                Some(r) => r,
                None => { let _ = dup.ReleaseFrame(); continue; }
            };
            let frame = match resource.cast::<ID3D11Texture2D>() {
                Ok(f) => f,
                Err(_) => { let _ = dup.ReleaseFrame(); continue; }
            };
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            frame.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = D3D11_BIND_FLAG(0).0 as u32;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = D3D11_RESOURCE_MISC_FLAG(0).0 as u32;

            let mut staging: Option<ID3D11Texture2D> = None;
            if dev.CreateTexture2D(&desc, None, Some(&mut staging)).is_err() {
                let _ = dup.ReleaseFrame(); continue;
            }
            let staging = match staging { Some(t) => t, None => { let _ = dup.ReleaseFrame(); continue; } };
            ctx.CopyResource(&staging, &frame);
            let _ = dup.ReleaseFrame();

            let mut mapped = windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE::default();
            if ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).is_err() { continue; }
            let src = mapped.pData as *const u8;
            let sp = mapped.RowPitch as usize;
            let dp = (info.w as usize) * 4;
            let rx = (info.left - vx) as usize;
            let ry = (info.top - vy) as usize;
            for row in 0..info.h as usize {
                let off = ((ry + row) * vw as usize + rx) * 4;
                if off + dp > composite.len() { continue; }
                for col in 0..info.w as usize {
                    let si = row * sp + col * 4;
                    let di = off + col * 4;
                    composite[di] = *src.add(si + 2);
                    composite[di + 1] = *src.add(si + 1);
                    composite[di + 2] = *src.add(si);
                    composite[di + 3] = 255;
                }
            }
            ctx.Unmap(&staging, 0);
        }
        Some(ScreenShot { left: vx, top: vy, width: vw as i32, height: vh as i32, rgba: composite })
    }
}

fn own_pid() -> u32 { std::process::id() }

struct EnumCtx { target: POINT, result: Option<ScreenRect> }

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
        IsWindowVisible, GWL_STYLE, GWL_EXSTYLE,
    };
    let ctx = &mut *(lparam.0 as *mut EnumCtx);

    if !IsWindowVisible(hwnd).as_bool() { return BOOL(1); }
    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex & 0x0000_0080 != 0 { return BOOL(1); }
    let st = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    if st & 0x4000_0000 != 0 { return BOOL(1); }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == own_pid() { return BOOL(1); }
    let mut r = RECT::default();
    if GetWindowRect(hwnd, &mut r).is_err() { return BOOL(1); }
    if (r.right - r.left) < 40 || (r.bottom - r.top) < 40 { return BOOL(1); }
    let mut cls = [0u16; 64];
    let n = GetClassNameW(hwnd, &mut cls);
    let cls = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
    if matches!(
        cls.as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Windows.UI.Core.CoreWindow"
    ) { return BOOL(1); }

    let t = ctx.target;
    if t.x >= r.left && t.x <= r.right && t.y >= r.top && t.y <= r.bottom {
        ctx.result = Some(ScreenRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom });
        return BOOL(0);
    }
    BOOL(1)
}

pub fn virtual_screen_bounds() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if w <= 0 || h <= 0 { return None; }
    Some((left, top, w, h))
}

#[tauri::command]
pub fn windows_at(x: i32, y: i32) -> Option<ScreenRect> {
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    let mut ctx = EnumCtx { target: POINT { x, y }, result: None };
    let _ = unsafe { EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut EnumCtx as isize)) };
    ctx.result
}

#[tauri::command]
pub fn screen_bounds() -> Option<ScreenRect> {
    let (left, top, w, h) = virtual_screen_bounds()?;
    Some(ScreenRect { left, top, right: left + w, bottom: top + h })
}
