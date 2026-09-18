// Linggo 应用状态（跨模块共享，均由 tauri 管理为 State<AppState>）。
// 设计原则：账单式单实例 —— 所有可变共享状态集中于此，模块间不互相持有状态。

use crate::llama_backend::ModelStatus;
use crate::settings::Settings;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc;

pub struct AppState {
    /// 当前生效配置（镜像 settings.json，内存读速；settings_set 时同步落盘）
    pub settings: Mutex<Settings>,
    /// F1/F2 触发时记录的目标窗口句柄（HWND 原始值 as isize），用于把焦点还给原窗口
    pub target_hwnd: Mutex<Option<isize>>,
    /// 前台是否为全屏（独占/无边框）——由 fullscreen 监控线程维护；为 true 时热键被屏蔽
    pub game_fullscreen: AtomicBool,
    /// 模型工作线程命令通道（setup 时由 spawn_worker 注入）
    pub cmd_tx: Mutex<Option<mpsc::UnboundedSender<crate::llama_backend::ModelCmd>>>,
    /// 当前模型状态（工作线程持续维护，前端轮询/事件双通道获取）
    pub model_status: Mutex<ModelStatus>,
    /// F3 最新一次整屏截图帧（snip 框选裁剪/OCR/贴图共用；OCR 消耗大故不入全量级缓存数组）
    pub screen_capture: Mutex<Option<ScreenFrame>>,
    /// 最近翻译历史（内存镜像，写入时同步落盘 history.json）
    pub history: Mutex<Vec<crate::winutil::HistoryItem>>,
}

/// F3 截屏帧（整块虚拟桌面，物理像素、RGBA8）
#[derive(Debug, Clone)]
pub struct ScreenFrame {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl AppState {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings: Mutex::new(settings),
            target_hwnd: Mutex::new(None),
            game_fullscreen: AtomicBool::new(false),
            cmd_tx: Mutex::new(None),
            model_status: Mutex::new(ModelStatus::default()),
            screen_capture: Mutex::new(None),
            history: Mutex::new(crate::winutil::load_history()),
        }
    }

    pub fn is_game(&self) -> bool {
        self.game_fullscreen.load(Ordering::SeqCst)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(Settings::default())
    }
}