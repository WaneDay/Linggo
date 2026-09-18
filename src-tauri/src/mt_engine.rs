// Linggo NMT 引擎层：OPUS-MT（含中文的语对）+ NLLB-200（全语种兜底），CTranslate2 CPU 推理。
//
// 引擎选择（settings.engine_order 引擎排序）：
//   - 凡 models 目录下存在对应「opus-mt-<src>-<tgt>-...」语言包的语对，OPUS 命中
//     （快、小、质量佳）；缺失时回落 NLLB；
//   - 其他语对走 NLLB；NLLB 也缺失时 translate_lines_engine 返回 None，
//     由上层（llama_backend）按排序决定是否回落大模型。
//   - 上层按 settings.engine_order 逐引擎调用 translate_lines_engine，命中即返回。
//
// 模型目录（可移植）：
//   - 自动发现：<exe>/models、<cwd>/models、exe 同目录、exe/cwd 上一级目录的直接子目录
//     （覆盖「NLLB / OPUS 模型放在工程外一层或与 exe 并列」的布局）；
//   - 也可在设置里指定绝对路径（nmt_opus_zh_en/nmt_opus_en_zh/nmt_nllb_dir），仅 zh↔en 历史字段。
//
// 目标语言 token 约定：
//   - NLLB 用无下划线的 Flores 码（如 "eng_Latn"），与 ct2rs 示例一致（实测共享词表即此形式）；
//   - 翻译器实例常驻缓存，首次使用惰性加载。

use anyhow::Result;
use ct2rs::tokenizers::auto::Tokenizer as AutoTokenizer;
use ct2rs::{ComputeType, Config, TranslationOptions, Translator};
use sentencepiece_rs::SentencePieceProcessor;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

// ---------------------------------------------------------------------------
// OPUS tokenizer（argos 式：source.spm/target.spm + shared_vocabulary）
// ---------------------------------------------------------------------------

pub(crate) struct OpusTokenizer {
    src: SentencePieceProcessor,
    tgt: SentencePieceProcessor,
}

impl OpusTokenizer {
    fn new(model_dir: &Path) -> Result<Self> {
        Ok(Self {
            src: SentencePieceProcessor::open(model_dir.join("source.spm"))?,
            tgt: SentencePieceProcessor::open(model_dir.join("target.spm"))?,
        })
    }
}

impl ct2rs::Tokenizer for OpusTokenizer {
    fn encode(&self, input: &str) -> Result<Vec<String>> {
        let mut toks = self.src.encode(input)?;
        // Marian CT2 需要句末 </s>（OPUS gaudi 转档实测必须；Python 与 Rust 一致）
        toks.push("</s>".to_string());
        Ok(toks)
    }
    fn decode(&self, tokens: Vec<String>) -> Result<String> {
        let filtered: Vec<String> = tokens
            .into_iter()
            .filter(|t| t != "</s>" && t != "<pad>")
            .collect();
        self.tgt.decode(&filtered).map_err(Into::into)
    }
}

// ---------------------------------------------------------------------------
// 已加载引擎缓存（进程级单例；worker 线程串行访问，静态避免 State 生命周期麻烦）
// ---------------------------------------------------------------------------

pub struct MtLoaded {
    /// 任意已安装 OPUS 语言包，key = 语对（如 "zh-de"），惰性加载、常驻缓存
    pub opus: std::collections::HashMap<String, Translator<OpusTokenizer>>,
    pub nllb: Option<Translator<AutoTokenizer>>,
    /// 最近一次 NMT 使用时刻（NLLB 闲置内存回收用）
    pub last_used: std::time::Instant,
}

impl Default for MtLoaded {
    fn default() -> Self {
        Self {
            opus: std::collections::HashMap::new(),
            nllb: None,
            last_used: std::time::Instant::now(),
        }
    }
}

static MT_LOADED: OnceLock<Mutex<MtLoaded>> = OnceLock::new();

fn loaded() -> std::sync::MutexGuard<'static, MtLoaded> {
    MT_LOADED.get_or_init(Default::default).lock().unwrap()
}

/// 释放全部已加载 NMT 引擎（设置/卸载时调用，下次使用自动重载）
pub fn unload_all() {
    *loaded() = MtLoaded::default();
}

// ---------------------------------------------------------------------------
// CT2 配置与选项
// ---------------------------------------------------------------------------

fn ct2_threads() -> usize {
    std::thread::available_parallelism()
        .map(|k| (k.get() / 2).clamp(2, 8))
        .unwrap_or(4)
}

fn ct2_config() -> Config {
    Config {
        compute_type: ComputeType::INT8,
        num_threads_per_replica: ct2_threads(),
        ..Default::default()
    }
}

fn ct2_options() -> TranslationOptions<String, String> {
    TranslationOptions {
        beam_size: 4,
        max_decoding_length: 512,
        // NLLB 1.3B 蒸馏版偶发生成 <unk>，直接禁掉（OPUS 不受影响）
        disable_unk: true,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 模型目录发现
// ---------------------------------------------------------------------------

fn strip_unc(p: &str) -> String {
    p.strip_prefix(r"\\?\").unwrap_or(p).to_string()
}

/// <exe>/models 或 <cwd>/models 中第一个真实存在的目录（OPUS 语言包下载目标）
pub fn models_base() -> Option<PathBuf> {
    [
        std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("models"))),
        std::env::current_dir().ok().map(|c| c.join("models")),
    ]
    .into_iter()
    .flatten()
    .find(|d| d.is_dir())
}

/// 便携式 models 根目录候选（打包后 <exe>/models 最优先）
pub(crate) fn scan_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            v.push(p.join("models"));
            v.push(p.to_path_buf());
            if let Some(pp) = p.parent() {
                v.push(pp.to_path_buf());
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        v.push(cwd.join("models"));
        v.push(cwd.to_path_buf());
        if let Some(cp) = cwd.parent() {
            v.push(cp.to_path_buf());
        }
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|p| seen.insert(p.clone()));
    v
}

/// 明显不含模型的工程/系统噪音目录名
pub const SKIP_DIRS: [&str; 12] = [
    "node_modules", "target", ".git", ".github", "dist", "docs", "src",
    "vendor", ".cargo", "__pycache__", "models", "icons",
];

/// OPUS 语言包目录：含 model.bin + source.spm + target.spm（语对由目录名解析，见 dir_pair）
pub fn is_valid_opus_dir(dir: &Path) -> bool {
    dir.is_dir()
        && dir.join("model.bin").exists()
        && dir.join("source.spm").exists()
        && dir.join("target.spm").exists()
}

pub fn is_valid_nllb_dir(dir: &Path) -> bool {
    dir.is_dir() && dir.join("model.bin").exists() && dir.join("tokenizer.json").exists()
}

/// 目录名 → 语对：`opus-mt-<src>-<tgt>-...`（如 opus-mt-zh-de-ct2 → ("zh","de")）
fn dir_pair(name: &str) -> Option<(String, String)> {
    let n = name.to_ascii_lowercase();
    let rest = n.strip_prefix("opus-mt-")?;
    let mut it = rest.split('-');
    let (a, b) = (it.next()?, it.next()?);
    if a.is_empty() || b.is_empty() {
        return None;
    }
    Some((a.to_string(), b.to_string()))
}

fn dir_matches_pair(dir: &Path, src: &str, tgt: &str) -> bool {
    dir.file_name()
        .and_then(|n| n.to_str())
        .and_then(dir_pair)
        .map(|(a, b)| a == src && b == tgt)
        .unwrap_or(false)
}

/// 查找语对（src→tgt）的 OPUS 目录：显式设置（仅 zh↔en 历史字段）优先，其次扫描 models 根目录。
fn find_opus_dir(app: &AppHandle, src: &str, tgt: &str) -> Option<PathBuf> {
    let src = src.trim().to_ascii_lowercase();
    let tgt = tgt.trim().to_ascii_lowercase();
    if src.is_empty() || tgt.is_empty() {
        return None;
    }
    let s = crate::settings::current(app);
    let explicit = match (src.as_str(), tgt.as_str()) {
        ("zh", "en") => s.nmt_opus_zh_en.clone(),
        ("en", "zh") => s.nmt_opus_en_zh.clone(),
        _ => String::new(),
    };
    let explicit = strip_unc(&explicit);
    if !explicit.is_empty() {
        let p = PathBuf::from(&explicit);
        if is_valid_opus_dir(&p) && dir_matches_pair(&p, &src, &tgt) {
            return Some(p);
        }
    }
    for base in scan_roots() {
        let Ok(rd) = base.read_dir() else { continue };
        for ent in rd.flatten() {
            let p = ent.path();
            if !p.is_dir() {
                continue;
            }
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if SKIP_DIRS.contains(&name) {
                    continue;
                }
            }
            if dir_matches_pair(&p, &src, &tgt) && is_valid_opus_dir(&p) {
                return Some(p);
            }
        }
    }
    None
}

/// 已安装的全部 OPUS 语对（去重，供状态展示）。
fn installed_opus_pairs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for base in scan_roots() {
        let Ok(rd) = base.read_dir() else { continue };
        for ent in rd.flatten() {
            let p = ent.path();
            if !p.is_dir() {
                continue;
            }
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if SKIP_DIRS.contains(&name) || !is_valid_opus_dir(&p) {
                continue;
            }
            if let Some((a, b)) = dir_pair(name) {
                if seen.insert(format!("{a}-{b}")) {
                    out.push((a, b));
                }
            }
        }
    }
    out
}

/// NLLB 模型目录（显式设置优先，其次自动发现）。
fn discover_nllb_dir(app: &AppHandle) -> Option<PathBuf> {
    let explicit = strip_unc(&crate::settings::current(app).nmt_nllb_dir);
    if !explicit.is_empty() {
        let p = PathBuf::from(&explicit);
        if is_valid_nllb_dir(&p) {
            return Some(p);
        }
    }
    for base in scan_roots() {
        let Ok(rd) = base.read_dir() else { continue };
        for ent in rd.flatten() {
            let p = ent.path();
            if !p.is_dir() {
                continue;
            }
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if SKIP_DIRS.contains(&name) {
                    continue;
                }
            }
            if is_valid_nllb_dir(&p) {
                return Some(p);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 状态（供设置面板与 nmt_status 命令）
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MtStatus {
    pub enabled: bool,
    pub priority: String,
    pub opus_zh_en: String,
    pub opus_en_zh: String,
    pub nllb: String,
    /// 当前可选引擎中实际将使用的："opus" / "nllb" / "none"
    pub active: String,
    /// 面向用户的可读提示
    pub note: String,
}

fn calc_status(app: &AppHandle) -> MtStatus {
    let s = crate::settings::current(app);
    let order: Vec<String> = s.engine_order.clone();
    let opus_zh_en = find_opus_dir(app, "zh", "en");
    let opus_en_zh = find_opus_dir(app, "en", "zh");
    let nllb = discover_nllb_dir(app);
    let pairs = installed_opus_pairs();
    let opus_ok = !pairs.is_empty();
    let nllb_ok = nllb.is_some();
    let active = if !s.nmt_enabled {
        "none".to_string()
    } else {
        order
            .iter()
            .find(|e| (*e == "opus" && opus_ok) || (*e == "nllb" && nllb_ok))
            .cloned()
            .unwrap_or_else(|| "none".to_string())
    };
    let note = if !s.nmt_enabled {
        "NMT 已停用，翻译将全部走大模型（GGUF）".to_string()
    } else if active == "none" {
        "设置 → 模型市场可下载安装语言包；NLLB 也可自行指定目录".to_string()
    } else if active == "opus" {
        let desc = pairs
            .iter()
            .map(|(a, b)| format!("{a}→{b}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("OPUS 引擎优先（已安装 {desc}），其余语对回落 NLLB/大模型")
    } else {
        "NLLB 引擎优先（全语种 × 中文）".to_string()
    };
    MtStatus {
        enabled: s.nmt_enabled,
        priority: order.join(">"),
        opus_zh_en: opus_zh_en
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        opus_en_zh: opus_en_zh
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        nllb: nllb.map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        active,
        note,
    }
}

pub fn emit_status(app: &AppHandle) {
    let _ = app.emit("nmt-status", &calc_status(app));
}

// ---------------------------------------------------------------------------
// 内存管理
//   - OPUS（每个语言包 ≈80–310MB）与 NLLB（≈1.38GB 加载权）均为进程级惰性缓存；
//   - NLLB 体积大，按设置 idle_timeout_secs 闲置自动回收（0=常驻），OPUS 小而常驻；
//   - 「释放 NMT 内存」按钮 = unload_all 全量释放（下次翻译自动重载）。
// ---------------------------------------------------------------------------

fn nmt_idle_secs(app: &AppHandle) -> u64 {
    let s = crate::settings::current(app);
    if s.idle_timeout_secs == 0 { 0 } else { s.idle_timeout_secs }
}

/// 闲置到期则释放 NLLB 引擎（OPUS 保持缓存）。
pub fn prune_idle(app: &AppHandle) {
    let secs = nmt_idle_secs(app);
    if secs == 0 {
        return;
    }
    let mut st = loaded();
    if st.nllb.is_some() && st.last_used.elapsed().as_secs() >= secs {
        st.nllb = None;
        drop(st);
        emit_status(app);
    }
}

/// 后台巡检线程：每 5s 检查一次 NLLB 闲置状态（沿 idle_timeout_secs 规则）。
pub fn spawn_idle_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            prune_idle(&app);
        }
    });
}

/// 手动释放全部 NMT 模型内存（OPUS + NLLB；下次翻译自动重载）。
#[tauri::command]
pub fn nmt_unload(app: AppHandle) -> Result<String, String> {
    unload_all();
    let _ = app.emit("nmt-status", &calc_status(&app));
    Ok("已释放 NMT 模型内存（OPUS + NLLB），下次翻译自动重载".to_string())
}

// ---------------------------------------------------------------------------
// NLLB 语言 token（Flores 码，与 NLLB-200 CT2 共享词表一致；无下划线）
// ---------------------------------------------------------------------------

fn nllb_lang_token(code: &str) -> Option<&'static str> {
    Some(match code {
        "en" => "eng_Latn",
        "zh" => "zho_Hans",
        "ja" => "jpn_Jpan",
        "ko" => "kor_Hang",
        "fr" => "fra_Latn",
        "de" => "deu_Latn",
        "es" => "spa_Latn",
        "it" => "ita_Latn",
        "pt" => "por_Latn",
        "ru" => "rus_Cyrl",
        "ar" => "arb_Arab",
        "hi" => "hin_Deva",
        "vi" => "vie_Latn",
        "th" => "tha_Thai",
        "id" => "ind_Latn",
        "ms" => "msa_Latn",
        "tr" => "tur_Latn",
        "nl" => "nld_Latn",
        "pl" => "pol_Latn",
        "uk" => "ukr_Cyrl",
        "sv" => "swe_Latn",
        "da" => "dan_Latn",
        "fi" => "fin_Latn",
        "no" => "nob_Latn",
        "cs" => "ces_Latn",
        "hu" => "hun_Latn",
        "ro" => "ron_Latn",
        "bg" => "bul_Cyrl",
        "hr" => "hrv_Latn",
        "sk" => "slk_Latn",
        "sl" => "slv_Latn",
        "he" => "heb_Hebr",
        "el" => "ell_Grek",
        _ => return None,
    })
}

fn clean_nllb(text: &str) -> String {
    text.replace("<unk>", "").replace("<pad>", "").replace("</s>", "")
}

// ---------------------------------------------------------------------------
// 引擎构造
// ---------------------------------------------------------------------------

fn build_opus(dir: &Path) -> Result<Translator<OpusTokenizer>> {
    Translator::with_tokenizer(dir, OpusTokenizer::new(dir)?, &ct2_config())
}

fn build_nllb(dir: &Path) -> Result<Translator<AutoTokenizer>> {
    Translator::new(dir, &ct2_config())
}

// ---------------------------------------------------------------------------
// 对外翻译入口
// ---------------------------------------------------------------------------

/// 脚本启发式源语言识别（仅用于 source="auto"，替代大模型识别）：
/// 常见脚本 → 语言码；识别不了返回 None（交还大模型或报错）。
/// F1–F4 首选/次选方向规则也复用此识别（见 llama_backend::resolve_quick_pair）。
pub(crate) fn detect_script(text: &str) -> Option<&'static str> {
    let mut has_han = false;
    let mut has_latin = false;
    for ch in text.chars() {
        let u = ch as u32;
        if (u >= 0x3040 && u <= 0x30ff) || (u >= 0x31f0 && u <= 0x31ff) {
            return Some("ja"); // 假名优先（日文常含汉字）
        }
        if (u >= 0xac00 && u <= 0xd7af) || (u >= 0x1100 && u <= 0x11ff) {
            return Some("ko");
        }
        if (u >= 0x0400 && u <= 0x052f) {
            return Some("ru"); // 西里尔
        }
        if (u >= 0x0e00 && u <= 0x0e7f) {
            return Some("th");
        }
        if (u >= 0x0600 && u <= 0x06ff) {
            return Some("ar");
        }
        if (u >= 0x4e00 && u <= 0x9fff) || (u >= 0x3400 && u <= 0x4dbf) {
            has_han = true; // 仅汉字 → zh
        }
        if ch.is_ascii_alphabetic() {
            has_latin = true;
        }
    }
    if has_han {
        Some("zh")
    } else if has_latin {
        Some("en")
    } else {
        None
    }
}

/// 仅尝试指定 NMT 引擎（"opus" / "nllb"）。返回 Ok(None) 表示不适用
/// （停用/语对不支持/无模型），由上层按引擎排序回落。
pub fn translate_lines_engine(
    app: &AppHandle,
    texts: &[String],
    source: &str,
    target: &str,
    engine: &str,
) -> Result<Option<Vec<String>>, String> {
    let s = crate::settings::current(app);
    if !s.nmt_enabled || texts.is_empty() {
        return Ok(None);
    }
    let tgt = target.trim();
    let src = if source.trim() == "auto" {
        // NMT 支持显式源语言；"auto" 用脚本启发式识别，命中不了才交还大模型
        match detect_script(&texts.join("\n")) {
            Some(l) => l,
            None => return Ok(None),
        }
    } else {
        source.trim()
    };
    if src.is_empty() || tgt.is_empty() || src == tgt {
        return Ok(None);
    }
    match engine {
        "opus" => {
            let Some(dir) = find_opus_dir(app, src, tgt) else {
                return Ok(None);
            };
            let pair = format!("{src}-{tgt}");
            let mut st = loaded();
            st.last_used = std::time::Instant::now();
            if !st.opus.contains_key(&pair) {
                let t = build_opus(&dir).map_err(|e| format!("OPUS 加载失败: {e}"))?;
                st.opus.insert(pair.clone(), t);
            }
            // guard 保持存活（translator 借用内部）；worker 线程串行，无并发风险
            let slot = st.opus.get(&pair).unwrap();
            let out = slot
                .translate_batch(texts, &ct2_options(), None)
                .map_err(|e| format!("OPUS 翻译失败: {e}"))?;
            Ok(Some(out.into_iter().map(|(t, _)| t).collect()))
        }
        "nllb" => {
            if nllb_lang_token(tgt).is_none() {
                return Ok(None);
            }
            let Some(dir) = discover_nllb_dir(app) else {
                return Ok(None);
            };
            let tgt_tok = nllb_lang_token(tgt).expect("checked above");
            let mut st = loaded();
            st.last_used = std::time::Instant::now();
            if st.nllb.is_none() {
                st.nllb =
                    Some(build_nllb(&dir).map_err(|e| format!("NLLB 加载失败: {e}"))?);
            }
            let translator = st.nllb.as_ref().unwrap();
            let prefixes = vec![vec![tgt_tok.to_string()]; texts.len()];
            let out = translator
                .translate_batch_with_target_prefix(texts, &prefixes, &ct2_options(), None)
                .map_err(|e| format!("NLLB 翻译失败: {e}"))?;
            Ok(Some(out.into_iter().map(|(t, _)| clean_nllb(&t)).collect()))
        }
        _ => Ok(None),
    }
}

/// 单段翻译（按指定 NMT 引擎尝试）。上层按引擎排序循环调用。
pub fn translate_text_engine(
    app: &AppHandle,
    text: &str,
    source: &str,
    target: &str,
    engine: &str,
) -> Result<Option<String>, String> {
    let texts = vec![text.to_string()];
    Ok(translate_lines_engine(app, &texts, source, target, engine)?
        .map(|mut v| v.remove(0)))
}

// ---------------------------------------------------------------------------
// 命令面
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn nmt_status(app: AppHandle) -> MtStatus {
    calc_status(&app)
}

/// 弹目录选择框挑 NMT 模型目录（设置面板「浏览」；rfd 原生对话框，全离线）
/// kind: "nllb" / "opus_zh_en" / "opus_en_zh"
#[tauri::command]
pub fn nmt_pick_dir(app: AppHandle, kind: String) -> Result<Option<String>, String> {
    use rfd::FileDialog;
    let title = match kind.as_str() {
        "nllb" => "选择 NLLB 模型目录（含 model.bin 与 tokenizer.json）",
        "opus_zh_en" => "选择 OPUS zh→en 语言包目录",
        "opus_en_zh" => "选择 OPUS en→zh 语言包目录",
        _ => return Err(format!("未知类型: {kind}")),
    };
    let picked = FileDialog::new().set_title(title).pick_folder();
    if let Some(p) = picked {
        let p = p.to_string_lossy().to_string();
        let mut s = crate::settings::current(&app);
        let key = match kind.as_str() {
            "nllb" => &mut s.nmt_nllb_dir,
            "opus_zh_en" => &mut s.nmt_opus_zh_en,
            _ => &mut s.nmt_opus_en_zh,
        };
        *key = p.clone();
        crate::settings::apply(&app, s)?;
        Ok(Some(p))
    } else {
        Ok(None)
    }
}

/// curl.exe 下载（Win10 1803+ 系统自带；--ssl-no-revoke 规避吊销检查离线失败，-C - 断点续传）
/// CREATE_NO_WINDOW：curl 是控制台程序，GUI 进程拉起时会额外弹 cmd 窗口，必须隐藏。
pub fn download_file(url: &str, target: &Path) -> bool {
    download_file_cancel(url, target, &std::sync::atomic::AtomicBool::new(false))
}

/// curl.exe 下载（可取消）：每 250ms 检查 cancel 标志，被置位则强杀 curl 子进程并返回 false，
/// 半成品文件留给调用方清理。用于模型包下载的「取消下载」即时生效（无需等大文件下完）。
pub fn download_file_cancel(
    url: &str,
    target: &Path,
    cancel: &std::sync::atomic::AtomicBool,
) -> bool {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    let src = target.to_string_lossy().to_string();
    let mut cmd = std::process::Command::new("curl.exe");
    cmd.args(["-sSL", "--ssl-no-revoke", "--retry", "3", "--retry-all-errors", "-C", "-"])
        .args(["-o", &src, url]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return false,
    };
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        match child.try_wait() {
            Ok(Some(st)) => return st.success(),
            Ok(None) => {}
            Err(_) => return false,
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(root: &std::path::Path, name: &str) -> PathBuf {
        let p = root.join(name);
        let _ = std::fs::create_dir_all(&p);
        p
    }

    #[test]
    fn opus_dir_pair_parses_case_insensitive() {
        assert_eq!(dir_pair("opus-mt-zh-de-ct2"), Some(("zh".into(), "de".into())));
        assert_eq!(dir_pair("opus-mt-de-ZH-ct2"), Some(("de".into(), "zh".into())));
        assert_eq!(dir_pair("opus-mt-sv-ZH-ctranslate2-android"), Some(("sv".into(), "zh".into())));
        assert_eq!(dir_pair("nllb-1.3b"), None);
        assert_eq!(dir_pair("opus-mt-zh"), None);
    }

    #[test]
    fn dir_match_pair_requires_valid_ct2_files() {
        let root = std::env::temp_dir().join(format!("linggo_pair_test_{}", std::process::id()));
        let dir = tmp(&root, "opus-mt-zh-nl-ct2");
        assert!(dir_matches_pair(&dir, "zh", "nl"));
        assert!(!dir_matches_pair(&dir, "nl", "zh"));
        assert!(!is_valid_opus_dir(&dir));
        std::fs::write(dir.join("model.bin"), b"m").unwrap();
        std::fs::write(dir.join("source.spm"), b"s").unwrap();
        std::fs::write(dir.join("target.spm"), b"t").unwrap();
        assert!(is_valid_opus_dir(&dir));
        assert!(dir_matches_pair(&dir, "zh", "nl"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
