// Linggo 全屏/游戏窗口监控模块：
// 轮询前台窗口矩形 vs 其所在监视器，判定「游戏/独占全屏」（浏览器全屏不算，见 win32::is_game_fullscreen），
// 维护 AppState.game_fullscreen；状态变化 → 广播 ui-status {gameMode}，hotkeys.rs 据此屏蔽/恢复热键。
// 周期 ~600ms，Win32 轻量调用，不占主线程。

use crate::state::AppState;
use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub fn spawn_monitor(app: AppHandle) {
    std::thread::spawn(move || loop {
        // 前台窗口变化频繁，仅在全屏状态翻转时广播；顺带每轮刷新到 AppState
        let fg = crate::win32::foreground_hwnd();
        let fullscreen = fg != 0 && crate::win32::is_game_fullscreen(fg);
        let st = app.state::<AppState>();
        let prev = st.game_fullscreen.swap(fullscreen, Ordering::SeqCst);
        drop(st);
        if prev != fullscreen {
            let _ = app.emit("ui-status", json!({ "gameMode": fullscreen }));
        }
        std::thread::sleep(Duration::from_millis(600));
    });
}