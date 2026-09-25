// Linggo 设置模块：Settings 结构体、默认值、settings.json 读写（%APPDATA%\Linggo\settings.json）、
// 命令 settings_get/settings_set、settings-changed 事件广播。

use crate::constants::{DEFAULT_HISTORY_LIMIT, DEFAULT_HOTKEYS, DEFAULT_IDLE_TIMEOUT_SECS, DEFAULT_PREFERRED_LANG, DEFAULT_SECONDARY_LANG};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

/// 0.1.1 及更早的 F5 默认引擎（升级迁移用，见 normalize）
const LEGACY_DEFAULT_F5_ENGINE: &str = "gguf";

/// %APPDATA%\Linggo —— 固定目录，卸载清理脚本可精准定位（勿随 identifier 变动）
pub fn app_data_dir() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("Linggo")
}

pub fn settings_path() -> PathBuf {
    app_data_dir().join("settings.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hotkeys {
    pub f1: String,
    pub f2: String,
    pub f3: String,
    pub f4: String,
    pub f5: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        let mut h = Hotkeys {
            f1: String::new(),
            f2: String::new(),
            f3: String::new(),
            f4: String::new(),
            f5: String::new(),
        };
        for (key, acc) in DEFAULT_HOTKEYS {
            match key {
                "f1" => h.f1 = acc.to_string(),
                "f2" => h.f2 = acc.to_string(),
                "f3" => h.f3 = acc.to_string(),
                "f4" => h.f4 = acc.to_string(),
                "f5" => h.f5 = acc.to_string(),
                _ => {}
            }
        }
        h
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// auto / light / dark（跟随系统 or 强制）
    pub theme: String,
    pub hotkeys: Hotkeys,
    /// 闲置超时（秒），5/15/30/0=永久常驻
    pub idle_timeout_secs: u64,
    /// 默认源语言："auto" 自动检测
    pub default_source: String,
    /// 默认目标语言
    pub default_target: String,
    /// 本地 GGUF 模型路径（Hy-MT2-1.8B-Q4_K_M）
    pub model_path: String,
    /// OCR 语言："auto"/zh/en/ja/ko/de/fr/es/it/pt/ru
    pub ocr_lang: String,
    /// 游戏全屏时自动屏蔽热键
    pub game_mode_block: bool,
    /// 历史记录数量上限（0=不保留）
    pub history_limit: u32,
    /// 开机自启
    pub autostart: bool,
    /// F3 贴图默认「翻译覆盖图」模式（true）；false=默认原图
    pub pin_default_overlay: bool,
    /// 贴图时是否显示快捷键提示文字（true=显示，默认）
    pub pin_show_tip: bool,
    /// GPU 卸载层数：999=全部卸载到 GPU；0=纯 CPU（无 N 卡/显存不足时用）
    pub gpu_layers: i32,
    /// 启用 NMT 快速引擎（OPUS/NLLB，CT2 CPU 推理）；false 则全部走大模型
    pub nmt_enabled: bool,
    /// 引擎排序（F1–F4 快速功能按此顺序尝试）：["opus","nllb","gguf"]
    pub engine_order: Vec<String>,
    /// F5 主窗口优先引擎（默认 "gguf" 最高质量；可在设置单独修改）
    pub f5_engine: String,
    /// OPUS zh→en 语言包目录（空 = 自动在 models 目录发现）
    pub nmt_opus_zh_en: String,
    /// OPUS en→zh 语言包目录（空 = 自动在 models 目录发现）
    pub nmt_opus_en_zh: String,
    /// NLLB CT2 模型目录（空 = 自动在 models 目录发现）
    pub nmt_nllb_dir: String,
    /// NMT 不适用/不可用时，是否回落大模型（false 则直接报错）
    pub nmt_llm_fallback: bool,
    /// 模型市场索引 URL（空 = 内置语言包索引；仿 argos-translate ARGOS_PACKAGE_INDEX）
    pub package_index_url: String,
    /// 首选语言：F1–F4 识别到首选语言时译为次选语言
    pub preferred_lang: String,
    /// 次选语言：F1–F4 识别到非首选语言时译为首选语言
    pub secondary_lang: String,
    /// 启动时自动检查新版本（仅 GitHub 检查提示，不自动下载；关闭后「检查更新」按钮仍可用）
    pub check_updates_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "auto".to_string(),
            hotkeys: Hotkeys::default(),
            idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
            default_source: "auto".to_string(),
            default_target: "zh".to_string(),
            model_path: String::new(),
            ocr_lang: "auto".to_string(),
            game_mode_block: true,
            history_limit: DEFAULT_HISTORY_LIMIT,
            autostart: false,
            pin_default_overlay: true,
            pin_show_tip: true,
            gpu_layers: 999,
            nmt_enabled: true,
            engine_order: crate::constants::DEFAULT_ENGINE_ORDER
                .iter()
                .map(|s| s.to_string())
                .collect(),
            f5_engine: crate::constants::DEFAULT_F5_ENGINE.to_string(),
            nmt_opus_zh_en: String::new(),
            nmt_opus_en_zh: String::new(),
            nmt_nllb_dir: String::new(),
            nmt_llm_fallback: true,
            package_index_url: String::new(),
            check_updates_enabled: true,
            preferred_lang: DEFAULT_PREFERRED_LANG.to_string(),
            secondary_lang: DEFAULT_SECONDARY_LANG.to_string(),
        }
    }
}

/// 从磁盘读取配置；文件缺失/损坏时回退默认值。
/// 规范化后返回：低版本/越界配置（如缺首选/次选字段）在内存同步补全，
/// 与 apply() 的 normalize 保持同一口径（旧 settings.json 不带新字段时 String 默认空串）。
pub fn load() -> Settings {
    let s = fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    normalize(s)
}

/// 将配置写入磁盘（自动创建目录）
pub fn save(s: &Settings) -> Result<(), String> {
    let p = settings_path();
    if let Some(dir) = p.parent() {
        let _ = fs::create_dir_all(dir);
    }
    // 原子写：先写临时文件再重命名，避免中途崩溃产生半个 json
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(s).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

/// 规范化：把不受支持的值拉回合法范围（防御前端越界值）
pub fn normalize(mut s: Settings) -> Settings {
    if !matches!(s.theme.as_str(), "auto" | "light" | "dark") {
        s.theme = "auto".to_string();
    }
    if !crate::constants::IDLE_CHOICES.contains(&s.idle_timeout_secs) {
        s.idle_timeout_secs = DEFAULT_IDLE_TIMEOUT_SECS;
    }
    if !crate::constants::is_valid_lang(&s.default_source) {
        s.default_source = "auto".to_string();
    }
    if !crate::constants::is_valid_lang(&s.default_target) || s.default_target == "auto" {
        s.default_target = "zh".to_string();
    }
    if !crate::constants::HISTORY_CHOICES.contains(&s.history_limit) {
        s.history_limit = DEFAULT_HISTORY_LIMIT;
    }
    if !(0..=999).contains(&s.gpu_layers) {
        s.gpu_layers = 999;
    }
    // 引擎排序：只保留合法引擎、去重、顺序不变；旧版 3 引擎配置（0.1.1 及更早）自动把
    // Google 插到最前（升级即默认谷歌优先），缺项再按默认顺序补齐。
    let want = crate::constants::ENGINES.len();
    // 旧版配置标记：排序里没有 Google 即视为 0.1.1 及更早，需要连带迁移 F5 默认引擎
    let legacy_engine_order = !s
        .engine_order
        .iter()
        .any(|e| e == "google" && crate::constants::is_engine(e));
    let mut order: Vec<String> = Vec::new();
    for e in &s.engine_order {
        if crate::constants::is_engine(e) && !order.contains(e) && order.len() < want {
            order.push(e.clone());
        }
    }
    // 旧版 3 槽配置：Google 未出现 → 置顶（用户已在 0.1.2 面板里排好的 4 槽配置不受影响）
    if order.len() < want && !order.iter().any(|e| e == "google") {
        order.insert(0, "google".to_string());
    }
    for e in crate::constants::DEFAULT_ENGINE_ORDER {
        if order.len() < want && !order.iter().any(|x| x == e) {
            order.push(e.to_string());
        }
    }
    s.engine_order = if order.len() == want {
        order
    } else {
        crate::constants::DEFAULT_ENGINE_ORDER
            .iter()
            .map(|x| x.to_string())
            .collect()
    };
    // F5 引擎：非法/空 → 默认（Google）。仅当配置为旧版（排序里没有 Google）时，才把
    // 旧默认 "gguf" 迁移到 Google；0.1.2 里用户主动把 F5 改回 GGUF 会被保留。
    if !crate::constants::is_engine(&s.f5_engine)
        || (legacy_engine_order && s.f5_engine == LEGACY_DEFAULT_F5_ENGINE)
    {
        s.f5_engine = crate::constants::DEFAULT_F5_ENGINE.to_string();
    }
    if !crate::constants::is_valid_lang(&s.preferred_lang) || s.preferred_lang == "auto" {
        s.preferred_lang = DEFAULT_PREFERRED_LANG.to_string();
    }
    if !crate::constants::is_valid_lang(&s.secondary_lang) || s.secondary_lang == "auto" {
        s.secondary_lang = DEFAULT_SECONDARY_LANG.to_string();
    }
    // 首选/次选不得相同：相同则强改次选，保证 F1–F4 方向规则有效
    if s.secondary_lang == s.preferred_lang {
        s.secondary_lang = if s.preferred_lang == "en" { "zh" } else { "en" }.to_string();
    }
    s
}

/// 取当前生效配置副本（内存镜像）
pub fn current(app: &AppHandle) -> Settings {
    app.state::<AppState>()
        .settings
        .lock()
        .expect("settings lock poisoned")
        .clone()
}

/// 非恐慌版本：状态尚未 manage（如极端时序的 webview 早期回调）时回退默认配置，
/// 其余场景与 current() 一致。用于过早触发的防御路径。
pub fn safe_current(app: &AppHandle) -> Settings {
    match app.try_state::<AppState>() {
        Some(st) => st.settings.lock().expect("settings lock poisoned").clone(),
        None => Settings::default(),
    }
}

/// 保存 + 更新内存镜像 + 广播给所有窗口 + 联动系统（热键重注册 / 自启注册表）
pub fn apply(app: &AppHandle, s: Settings) -> Result<(), String> {
    let prev = current(app);
    let s = normalize(s);
    save(&s)?;
    if prev.autostart != s.autostart {
        let _ = crate::winutil::set_autostart(s.autostart);
    }
    *app.state::<AppState>()
        .settings
        .lock()
        .expect("settings lock poisoned") = s.clone();
    app.emit("settings-changed", &s).map_err(|e| e.to_string())?;
    // NMT 引擎状态联动（设置面板实时刷新）
    crate::mt_engine::emit_status(&app);
    // 步骤 6 挂接：热键按新配置重注册
    crate::hotkeys::on_settings_changed(app);
    Ok(())
}

#[tauri::command]
pub fn settings_get(app: AppHandle) -> Settings {
    current(&app)
}

#[tauri::command]
pub fn settings_set(app: AppHandle, value: Settings) -> Result<(), String> {
    apply(&app, value)
}

#[tauri::command]
pub fn autostart_get() -> Result<bool, String> {
    Ok(crate::winutil::is_autostart())
}

#[tauri::command]
pub fn autostart_set(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut s = current(&app);
    s.autostart = enabled;
    apply(&app, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_settings() -> Settings {
        Settings {
            engine_order: crate::constants::LEGACY_ENGINE_ORDER
                .iter()
                .map(|s| s.to_string())
                .collect(),
            f5_engine: "gguf".to_string(),
            ..Default::default()
        }
    }

    /// 0.1.1 → 0.1.2 升级：旧 3 引擎排序自动变 4 槽，Google 置顶；F5 默认也切到 Google
    #[test]
    fn upgrade_migrates_legacy_engine_order_with_google_first() {
        let s = normalize(legacy_settings());
        assert_eq!(s.engine_order, vec!["google", "opus", "nllb", "gguf"]);
        assert_eq!(s.f5_engine, "google");
    }

    /// 全新安装：4 引擎默认排序 + F5 默认 Google
    #[test]
    fn fresh_install_defaults_to_google_first() {
        let s = normalize(Settings::default());
        assert_eq!(s.engine_order, vec!["google", "opus", "nllb", "gguf"]);
        assert_eq!(s.f5_engine, "google");
    }

    /// 用户自定义的 4 槽排序（含 google 非首位）不被改写
    #[test]
    fn custom_four_slot_order_is_preserved() {
        let s = normalize(Settings {
            engine_order: vec!["gguf".into(), "google".into(), "opus".into(), "nllb".into()],
            f5_engine: "opus".into(),
            ..Default::default()
        });
        assert_eq!(s.engine_order, vec!["gguf", "google", "opus", "nllb"]);
        assert_eq!(s.f5_engine, "opus");
    }

    /// 旧配置里被用户改过的排序（如 nllb 优先）保留原相对顺序，Google 仍置顶
    #[test]
    fn legacy_custom_order_keeps_relative_order_with_google_on_top() {
        let s = normalize(Settings {
            engine_order: vec!["nllb".into(), "opus".into(), "gguf".into()],
            ..Default::default()
        });
        assert_eq!(s.engine_order, vec!["google", "nllb", "opus", "gguf"]);
    }

    /// 非法引擎值被剔除并补齐到 4 槽
    #[test]
    fn invalid_engines_are_dropped_and_filled() {
        let s = normalize(Settings {
            engine_order: vec!["bogus".into(), "opus".into(), "opus".into()],
            ..Default::default()
        });
        assert_eq!(s.engine_order, vec!["google", "opus", "nllb", "gguf"]);
    }

    /// 0.1.2 用户在 4 槽面板里主动把 F5 改回 GGUF，保存后不能再被强制改回 Google
    #[test]
    fn explicit_gguf_f5_survives_repeated_saves() {
        let picked = Settings {
            engine_order: vec!["google".into(), "opus".into(), "nllb".into(), "gguf".into()],
            f5_engine: "gguf".into(),
            ..Default::default()
        };
        // normalize 会在每次 settings_set 时跑，模拟连续保存三次
        let s = normalize(normalize(normalize(picked)));
        assert_eq!(s.f5_engine, "gguf");
        assert_eq!(s.engine_order, vec!["google", "opus", "nllb", "gguf"]);
    }
}