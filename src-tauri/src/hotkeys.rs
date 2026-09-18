// Linggo 全局热键模块：按 settings.hotkeys 注册/重注册 F1..F5（global-shortcut），
// 游戏模式屏蔽联动，F1..F5 动作分发。
//
// F1 划词翻译 / F2 打字翻译(回车回贴) / F3 截图OCR翻译 / F4 新建文件夹 / F5 主窗口。
// 动作只负责「捕获 + 开窗 + 发事件」，具体渲染交给前端窗口（步骤 8/9）。

use crate::state::AppState;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

/// 解析配置里的加速键字符串（非法则忽略该键）
fn parse(acc: &str) -> Option<Shortcut> {
    use std::str::FromStr;
    Shortcut::from_str(acc.trim()).ok()
}

/// 同键判断：配置串解析后的 id == 触发键 id
fn same_key(acc: &str, id: u32) -> bool {
    parse(acc).map(|s| s.id() == id).unwrap_or(false)
}

/// 游戏全屏且启用「游戏时自动禁用热键」→ 屏蔽（try_state 防极早期 webview 回调竞态崩）
fn blocked(app: &AppHandle) -> bool {
    let st = app
        .try_state::<AppState>()
        .map(|s| s.is_game())
        .unwrap_or(false);
    st && crate::settings::safe_current(app).game_mode_block
}

/// 记录当前前台窗口（F2 回贴目标 / F1 划词来源）。
/// 若前台已是 Linggo 自己的窗口（或桌面 0），保持旧目标不变；
/// 防止「弹窗关闭后焦点落在 Linggo 主窗口 → 下次 F1 把 Linggo 自己当复制来源」。
fn capture_target(app: &AppHandle) -> isize {
    let hwnd = crate::win32::foreground_hwnd();
    let is_own = hwnd == 0
        || crate::win32::process_name_of(hwnd).eq_ignore_ascii_case("linggo.exe");
    let state = app.state::<AppState>();
    let mut slot = state.target_hwnd.lock().unwrap();
    if !is_own {
        *slot = Some(hwnd);
    }
    slot.unwrap_or(hwnd)
}

/// 把弹窗定位到鼠标附近（鼠标落在弹窗内部）：
/// 鼠标在屏幕中心偏右或居中心 → 落在弹窗横轴左半（偏左）；
/// 鼠标偏左 → 落在弹窗右半（偏右）。垂直居中，整体钳制在工作区内，绝不出屏。
fn position_popup(popup: &tauri::WebviewWindow) {
    let Some((mx, my)) = crate::win32::cursor_position() else { return };
    let Some((wl, wt, wr, wb)) = crate::win32::work_area_at(mx, my) else { return };
    let center_x = ((wl + wr) as f64) / 2.0;
    let ratio: f64 = if (mx as f64) >= center_x { 0.25 } else { 0.75 };
    let Ok(size) = popup.inner_size() else { return };
    let (w, h) = (size.width as i32, size.height as i32);
    let mut x = mx - (w as f64 * ratio) as i32;
    let mut y = my - h / 2;
    x = x.clamp(wl, (wr - w).max(wl));
    y = y.clamp(wt, (wb - h).max(wt));
    let _ = popup.set_position(tauri::Position::Physical(tauri::PhysicalPosition { x, y }));
}

/// 前端视图切换/高度变化后按当前鼠标重新对位（保证「鼠标在弹窗内」始终成立）
#[tauri::command]
pub fn popup_reposition(app: tauri::AppHandle) {
    if let Some(p) = app.get_webview_window("popup") {
        position_popup(&p);
    }
}

/// 弹窗关闭时把焦点还给划词/回贴来源窗口，保证下次 F1 划词前台仍是源窗口。
/// 在弹窗还持焦点时调用（本进程是前台进程，SetForegroundWindow 不会被 Windows 拒绝）。
#[tauri::command]
pub fn restore_target_focus(app: tauri::AppHandle) {
    if let Some(hwnd) = *app.state::<AppState>().target_hwnd.lock().unwrap() {
        crate::win32::focus_restore(hwnd);
    }
}

fn show_popup(app: &AppHandle, payload: serde_json::Value) {
    if let Some(p) = app.get_webview_window("popup") {
        position_popup(&p);
        let _ = p.show();
        let _ = p.set_focus();
        // 首次显示时 WebView2 attach 会重置窗口位置（表现为弹窗飞到屏幕右上角），
        // 延迟一拍再按鼠标对位一次，保证最终一定落在鼠标附近。
        let w = p.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(180));
            position_popup(&w);
        });
    }
    let _ = app.emit_to("popup", "popup-open", payload);
}

/// F1：记录前台窗口 → 后台 Ctrl+C 抓取选中文本（此时不得先弹窗抢焦点）→ 再开弹窗展示
fn action_f1(app: AppHandle) {
    let target = capture_target(&app);
    // 先发 Ctrl+C 抓取：弹窗 show/focus 会把焦点夺走，导致复制落在弹窗上而非原窗口。
    // 因此抓取阶段不显示弹窗，抓到文本后才开窗（loading → 译文）。
    tauri::async_runtime::spawn(async move {
        let text = crate::clipboard::grab_selection_async(target).await;
        let text = text.trim().to_string();
        if text.is_empty() {
            show_popup(&app, json!({ "mode": "warning", "message": "未捕获到选中文本" }));
            return;
        }
        let s = crate::settings::current(&app);
        show_popup(&app, json!({ "mode": "loading", "message": "正在翻译…" }));
        match crate::llama_backend::translate_text(
            app.clone(),
            text.clone(),
            s.default_source.clone(),
            "auto".to_string(), // F1 统一逻辑：首选/次选 → 后台 quick_target 决定目标
            None, // F1 按引擎排序（快速功能）
        )
        .await
        {
            Ok(t) => show_popup(
                &app,
                json!({
                    "mode": "translation",
                    "source": s.default_source,
                    "target": "auto",
                    "text": text,
                    "translated": t,
                }),
            ),
            Err(e) => show_popup(&app, json!({ "mode": "warning", "message": e })),
        }
    });
}

/// F2：弹窗输入框模式（回车后前端经 commit_paste 把译文粘贴回原窗口）
fn action_f2(app: AppHandle) {
    capture_target(&app);
    show_popup(&app, json!({ "mode": "input" }));
}

/// F3：先截整屏缓存帧，再铺透明的 snip 框选压盖（淡灰遮罩 + 自动窗口预选框）。
/// 框选完成后由前端依次调 ocr_snip → translate → 开 pin 贴图。
/// 注意：必须先截屏再显示压盖，否则会把压盖自身（淡灰遮罩）截进缓存帧。
fn action_f3(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Some(b) = crate::win32::virtual_screen_bounds() else {
            let _ = app.emit_to("popup", "warning", json!({ "message": "无法获取屏幕尺寸" }));
            return;
        };
        // 上次会话残留的压盖先隐藏，避免被本次截屏带入
        if let Some(w) = app.get_webview_window("snip") {
            let _ = w.hide();
        }
        // 先截整屏缓存帧（贴图 / OCR 用），此时压盖不可见 → 得到干净桌面
        let _ = crate::ocr::capture_screen(app.clone()).await;
        // 再铺上透明压盖（不冻结桌面、不黑屏）
        let _ = app.emit_to(
            "snip",
            "snip-open",
            json!({
                "left": b.0,
                "top": b.1,
                "width": b.2,
                "height": b.3,
            }),
        );
        if let Some(w) = app.get_webview_window("snip") {
            let _ = w.show();
            let _ = w.set_focus();
        }
    });
}

/// F4：新建文件夹（在鼠标所在的文件夹/桌面内创建）。
/// 按下瞬间解析目标目录（避免弹窗抢焦点后取不到），解析失败以警告视图提示。
fn action_f4(app: AppHandle) {
    capture_target(&app);
    match crate::explorer::folder_under_cursor() {
        Some(base) => show_popup(&app, json!({ "mode": "folder", "base": base })),
        None => show_popup(
            &app,
            json!({ "mode": "warning", "message": "未找到鼠标所在的文件夹：请把鼠标移到文件管理器窗口或桌面上再按 F4" }),
        ),
    }
}

/// F5：主窗口（双栏 + 全部设置）
fn action_f5(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 全局热键事件处理器（插件 with_handler 注册）
pub fn on_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state != ShortcutState::Pressed {
        return;
    }
    if blocked(app) {
        return;
    }
    let hs = crate::settings::current(app).hotkeys;
    let id = shortcut.id();
    if same_key(&hs.f1, id) {
        action_f1(app.clone());
    } else if same_key(&hs.f2, id) {
        action_f2(app.clone());
    } else if same_key(&hs.f3, id) {
        action_f3(app.clone());
    } else if same_key(&hs.f4, id) {
        action_f4(app.clone());
    } else if same_key(&hs.f5, id) {
        action_f5(app.clone());
    }
}

/// 按最新配置（重）注册全部热键；设置变更时由 settings::apply 触发
pub fn register_all(app: &AppHandle) {
    let _ = app.global_shortcut().unregister_all();
    let hs = crate::settings::current(app).hotkeys;
    for acc in [hs.f1, hs.f2, hs.f3, hs.f4, hs.f5] {
        if let Some(s) = parse(&acc) {
            let _ = app.global_shortcut().register(s);
        }
    }
}

/// 设置变更后的回调（settings::apply → 此函数）
pub fn on_settings_changed(app: &AppHandle) {
    register_all(app);
}