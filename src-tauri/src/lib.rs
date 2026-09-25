// Linggo 应用装配：窗口、托盘、单实例、设置、模块接线、命令门面、事件总线。
// 本文件为脚手架版（步骤 2）：仅装配单实例 + 最小命令，后续步骤逐步接线各模块。

mod settings;
mod state;
mod clipboard;
mod constants;
mod llama_backend;
mod segmenter;
mod prompt;
mod hotkeys;
mod fullscreen;
mod ocr;
mod win32;
mod winutil;
mod mt_engine;
mod google_engine;
mod pkg_index;
mod explorer;
mod updater;

use tauri::Manager;
use tray_icon::menu::{Menu as TrayMenu, MenuItem as TrayMenuItem};
use tray_icon::TrayIconBuilder;
use tauri::AppHandle;
use tauri::tray::{MouseButton, MouseButtonState};
use std::sync::atomic::{AtomicBool, Ordering};

/// 托盘「退出」置位；此后 CloseRequested 不再拦截为隐藏，程序正常退出
static QUITTING: AtomicBool = AtomicBool::new(false);

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 返回当前版本号（设置面板「关于」使用）
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 弹系统文件选择框挑 GGUF 模型（设置面板「浏览」；rfd 原生对话框，全离线）
#[tauri::command]
fn pick_model() -> Option<String> {
    use rfd::FileDialog;
    FileDialog::new()
        .set_title("选择本地 GGUF 模型")
        .add_filter("GGUF 模型", &["gguf", "bin"])
        .pick_file()
        .map(|p| p.to_string_lossy().to_string())
}

pub fn run() {
    let state = crate::state::AppState::new(crate::settings::load());
    tauri::Builder::default()
        // 全局状态必须在任何窗口/WebView 创建前注册：
        // 配置里的窗口在 setup 之前就被创建，网页一加载即 invoke 各命令，
        // 若等到 setup 再 manage 会出现 state() called before manage() 竞态崩溃。
        .manage(state)
        .plugin(tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, shortcut, event| {
                crate::hotkeys::on_shortcut(app, shortcut, event);
            })
            .build())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 第二次启动：调出已有主窗口，不多开
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            // 开机自启按配置向注册表兜底（手动删值/卸载残留后恢复）并保持静默
            if crate::settings::current(app.handle()).autostart {
                let _ = crate::winutil::set_autostart(true);
            }
            // 启动模型工作线程（单所有者：加载/推理/卸载/闲置卸载全部在此串行执行）
            crate::llama_backend::spawn_worker(app.handle());
            // NMT 闲置内存巡检（沿 idle_timeout_secs 规则释放 NLLB 大权重）
            crate::mt_engine::spawn_idle_loop(app.handle().clone());
            // 启动全屏/游戏窗口监控
            crate::fullscreen::spawn_monitor(app.handle().clone());
            // 按配置注册全局热键
            crate::hotkeys::register_all(app.handle());

            // 托盘（自建 tray-icon，不经过 Tauri 封装）：拿到窗口句柄用于系统气球通知。
            // 左键调出主窗口；右键菜单「打开主窗口 / 退出」。句柄须常驻（Box::leak）。
            let show = TrayMenuItem::with_id("show", "打开主窗口", true, None);
            let quit = TrayMenuItem::with_id("quit", "退出", true, None);
            let menu = TrayMenu::new();
            let _ = menu.append(&show);
            let _ = menu.append(&quit);
            let icon_img = app.default_window_icon().expect("builtin icon");
            let icon = tray_icon::Icon::from_rgba(
                icon_img.rgba().to_vec(),
                icon_img.width(),
                icon_img.height(),
            )
            .map_err(|e| format!("托盘图标转换失败：{e}"))?;
            let tray = Box::leak(Box::new(
                TrayIconBuilder::new()
                    .with_id("main-tray")
                    .with_tooltip("Linggo · 离线翻译")
                    .with_icon(icon)
                    .with_menu(Box::new(menu))
                    .build()
                    .map_err(|e| format!("托盘创建失败：{e}"))?,
            ));
            crate::updater::set_tray_handle(tray.window_handle());

            // 托盘交互：路由功能必须挂到 Tauri 的 on_menu_event / on_tray_icon_event
            // （Tauri 已在运行时抢占 tray_icon 的全局 set_event_handler，自建 tray-icon 菜单
            //  点击事件经由 Tauri 内部 EventLoopMessage 再分发给下列监听器，切勿再用 set_event_handler，
            //  否则 OnceCell.set 静默失败、菜单「打开主窗口/退出」全部失灵）。
            app.on_tray_icon_event(|_app, event| {
                if let tauri::tray::TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main(_app);
                }
            });
            app.on_menu_event(|_app, event| match event.id().as_ref() {
                "show" => show_main(_app),
                "quit" => {
                    QUITTING.store(true, Ordering::SeqCst);
                    _app.exit(0);
                }
                _ => {}
            });

            // 启动后静默检查更新（5s 后发起；开关关闭则跳过）
            crate::updater::spawn_auto_check(app.handle().clone());

            // 主窗口「✕」= 隐藏（托盘可退出），而非退出程序；托盘退出时放行
            if let Some(w) = app.get_webview_window("main") {
                let h = w.clone();
                w.on_window_event(move |e| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                        if !QUITTING.load(Ordering::SeqCst) {
                            let _ = h.hide();
                            api.prevent_close();
                        }
                    }
                });
            }

            // 常规启动显示主窗口；加 --silent（开机自启/静默）则常驻托盘不打扰
            let silent = std::env::args().any(|a| a == "--silent");
            if !silent {
                show_main(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            pick_model,
            crate::settings::settings_get,
            crate::settings::settings_set,
            crate::settings::autostart_get,
            crate::settings::autostart_set,
            crate::llama_backend::model_status,
            crate::llama_backend::model_load,
            crate::llama_backend::model_unload,
            crate::llama_backend::translate_text,
            crate::llama_backend::translate_lines,
            crate::llama_backend::history_get,
            crate::llama_backend::history_clear,
            crate::mt_engine::nmt_status,
            crate::mt_engine::nmt_pick_dir,
            crate::mt_engine::nmt_unload,
            crate::google_engine::google_probe,
            crate::google_engine::google_status,
            crate::pkg_index::pkg_list,
            crate::pkg_index::pkg_download,
            crate::pkg_index::pkg_cancel_download,
            crate::pkg_index::pkg_delete,
            crate::ocr::capture_screen,
            crate::ocr::ocr_snip,
            crate::ocr::snip_crop_png,
crate::clipboard::copy_text,
              crate::clipboard::commit_paste,
              crate::clipboard::copy_image,
              crate::winutil::system_accent_color,
              crate::win32::windows_at,
              crate::win32::screen_bounds,
              crate::hotkeys::popup_reposition,
              crate::hotkeys::restore_target_focus,
              crate::explorer::create_folder,
              crate::updater::check_update,
              crate::winutil::open_url
          ])
        .run(tauri::generate_context!())
        .expect("Linggo 启动失败");
}