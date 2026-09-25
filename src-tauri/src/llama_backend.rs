// MT2Flash llama.cpp 模型后端（核心）。
//
// 满足需求的内存/并发/生命周期规则：
//  1) ctx=512 固定（需求硬性约束）；
//  2) 权重加载后常驻，连续调用不再重复加载；
//  3) 每次推理新建一个上下文，KV 随上下文释放 =「每次推理后清空 KV cache」，权重不动；
//  4) 闲置计时（可设 5/15/30/0=永久），计时窗口内有新请求到来则重置计时；
//  5) 超时后由本 worker 线程后台异步卸载并广播 model-status=unloaded；再次使用自动重载&广播 loading；
//  6) 单所有者 worker 线程 = 天然互斥锁，同一时刻最多一个 加载/卸载/推理；
//  7) 超长文本自动分段（segmenter），错误经 Result 上抛，前端可见。

#![allow(deprecated)] // llama-cpp-2 的 token_to_bytes/Special 仍是最稳的反分词 API，被标记 deprecated

use crate::constants;
use crate::segmenter;
use serde::Serialize;
use std::num::NonZeroU32;
use std::sync::OnceLock;
use tokio::sync::{mpsc, oneshot};
use tauri::{AppHandle, Emitter, Manager};

/// 固定上下文长度（勿改）
pub const CTX: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(512) };

/// 永久常驻：闲置秒数取 0 时的巨型兜底值（实际等于永不卸载）
const IDLE_FOREVER_SECS: u64 = 365 * 24 * 3600;

/// 模型状态（供前端状态栏/设置面板展示）
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub state: String,          // unloaded / loading / ready / translating / unloading
    pub path: String,
    pub error: Option<String>,
}

impl Default for ModelStatus {
    fn default() -> Self {
        Self { state: "unloaded".to_string(), path: String::new(), error: None }
    }
}

struct Loaded {
    path: String,
    model: llama_cpp_2::model::LlamaModel,
}

/// 全局 backend：llama.cpp 要求每进程只初始化一次
fn backend() -> &'static llama_cpp_2::llama_backend::LlamaBackend {
    static BACKEND: OnceLock<llama_cpp_2::llama_backend::LlamaBackend> = OnceLock::new();
    BACKEND.get_or_init(|| {
        let mut b = llama_cpp_2::llama_backend::LlamaBackend::init()
            .expect("llama backend init failed");
        b.void_logs(); // 关掉 llama.cpp 刷屏日志
        b
    })
}

fn n_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| ((n.get() / 2).max(1)) as i32)
        .unwrap_or(4)
}

fn strip_unc(p: &str) -> String {
    p.strip_prefix(r"\\?\").unwrap_or(p).to_string()
}

/// 模型路径（当前配置）；为空则自动发现 models 根目录下的 *.gguf，再找不到返回空串
fn resolve_path(app: &AppHandle) -> String {
    let p = strip_unc(&crate::settings::current(app).model_path);
    if !p.is_empty() {
        return p;
    }
    find_gguf_model()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 扫描 models 根目录（含直接子目录），返回第一个 *.gguf 文件（模型市场安装的大模型）。
fn find_gguf_model() -> Option<std::path::PathBuf> {
    let mut found: Option<std::path::PathBuf> = None;
    'outer: for base in crate::mt_engine::scan_roots() {
        let Ok(rd) = base.read_dir() else { continue };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                let Ok(sd) = p.read_dir() else { continue };
                for f in sd.flatten() {
                    let fp = f.path();
                    if fp.is_file()
                        && fp
                            .extension()
                            .map(|e| e.eq_ignore_ascii_case("gguf"))
                            .unwrap_or(false)
                    {
                        found = Some(fp);
                        break 'outer;
                    }
                }
            } else if p
                .extension()
                .map(|e| e.eq_ignore_ascii_case("gguf"))
                .unwrap_or(false)
            {
                found = Some(p);
                break 'outer;
            }
        }
    }
    found
}

/// 闲置超时秒数；0=永久常驻（巨数兜底）
fn idle_secs(app: &AppHandle) -> u64 {
    let s = crate::settings::current(app).idle_timeout_secs;
    if s == 0 { IDLE_FOREVER_SECS } else { s }
}

// ---------------------------------------------------------------------------
// 工作线程命令协议
// ---------------------------------------------------------------------------

pub enum ModelCmd {
    /// 加载（空路径 = 用设置中的 modelPath）
    Load { path: String, reply: oneshot::Sender<Result<String, String>> },
    /// 立即卸载（设置面板「释放内存」）
    Unload { reply: oneshot::Sender<Result<String, String>> },
    Translate {
        text: String,
        source: String,
        target: String,
        /// 调用方指定的优先引擎（F5 传设置中的 f5_engine；其他为 None 用引擎排序）
        engine: Option<String>,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// F3 覆盖原文：批量翻译逐行文本（如多行 OCR 结果）
    TranslateLines {
        texts: Vec<String>,
        source: String,
        target: String,
        engine: Option<String>,
        reply: oneshot::Sender<Result<Vec<String>, String>>,
    },
}

pub fn channel() -> (mpsc::UnboundedSender<ModelCmd>, mpsc::UnboundedReceiver<ModelCmd>) {
    mpsc::unbounded_channel()
}

/// 启动模型工作线程；在 setup 中调用一次。
pub fn spawn_worker(app: &AppHandle) {
    let (tx, rx) = channel();
    *app.state::<crate::state::AppState>().cmd_tx.lock().unwrap() = Some(tx);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        run_worker(handle, rx).await;
    });
}

fn send(app: &AppHandle, cmd: ModelCmd) -> Result<(), String> {
    let tx = app
        .state::<crate::state::AppState>()
        .cmd_tx
        .lock()
        .unwrap()
        .clone()
        .ok_or("模型工作线程未启动")?;
    tx.send(cmd).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 状态广播
// ---------------------------------------------------------------------------

fn set_status(app: &AppHandle, state: &str, error: Option<String>) {
    let status = ModelStatus {
        state: state.to_string(),
        path: resolve_path(app),
        error,
    };
    *app.state::<crate::state::AppState>().model_status.lock().unwrap() = status.clone();
    let _ = app.emit("model-status", &status);
}

// ---------------------------------------------------------------------------
// 推理核心（非流式；每次新上下文 = KV 自清）
// ---------------------------------------------------------------------------

fn resolve_target(text: &str, source: &str, target: &str) -> String {
    let t = target.trim();
    if !t.is_empty() && t != "auto" {
        return constants::lang_en(t);
    }
    let src_is_zh = source.trim().starts_with("zh")
        || (source.trim() == "auto"
            && text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)));
    if src_is_zh { "English".to_string() } else { "Chinese".to_string() }
}

fn generate(model: &llama_cpp_2::model::LlamaModel, prompt: &str, max_tokens: usize) -> Result<String, String> {
    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::{AddBos, Special};
    use llama_cpp_2::sampling::LlamaSampler;

    let tokens = model
        .str_to_token(prompt, AddBos::Always)
        .map_err(|e| format!("分词失败：{e}"))?;
    if tokens.is_empty() {
        return Ok(String::new());
    }
    let prompt_len = tokens.len() as u32;
    if prompt_len >= CTX.get() {
        return Err(format!(
            "输入过长：本次需 {prompt_len} token，超出固定上下文 {ctx}。",
            ctx = CTX.get()
        ));
    }
    let remaining = (CTX.get() - prompt_len).saturating_sub(8) as usize;
    let budget = max_tokens.min(remaining);

    // 上下文固定 ctx=512；每请求新建即 KV 全新，结束后随上下文释放 =「清空 KV cache，权重常驻」
    let threads = n_threads();
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(Some(CTX))
        .with_n_threads(threads)
        .with_n_threads_batch(threads);
    let mut ctx = model
        .new_context(backend(), ctx_params)
        .map_err(|e| format!("创建上下文失败：{e}"))?;

    // 分块喂入 prompt（正常单次即可，保留循环以兼容异常边界）
    let chunk = 512usize;
    let mut batch = LlamaBatch::new(chunk.min(tokens.len()), 1);
    let last = tokens.len() - 1;
    let mut i = 0usize;
    while i < tokens.len() {
        let end = (i + chunk).min(tokens.len());
        batch.clear();
        for (j, tok) in tokens[i..end].iter().enumerate() {
            let pos = (i + j) as i32;
            batch
                .add(*tok, pos, &[0], (i + j) == last)
                .map_err(|e| format!("batch add 失败：{e}"))?;
        }
        ctx.decode(&mut batch).map_err(|e| format!("decode 失败：{e}"))?;
        i = end;
    }

    // 翻译固定用贪心采样，输出最稳定
    let mut sampler = LlamaSampler::greedy();
    let mut out = String::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut n_cur = tokens.len() as i32;
    for _ in 0..budget {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);
        if model.is_eog_token(token) {
            break;
        }
        if let Ok(b) = model.token_to_bytes(token, Special::Plaintext) {
            pending.extend_from_slice(&b);
        }
        match std::str::from_utf8(&pending) {
            Ok(s) => {
                out.push_str(s);
                pending.clear();
            }
            Err(e) => {
                let valid = e.valid_up_to();
                out.push_str(&String::from_utf8_lossy(&pending[..valid]));
                pending.drain(..valid);
            }
        }
        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| format!("batch add 失败：{e}"))?;
        n_cur += 1;
        ctx.decode(&mut batch).map_err(|e| format!("decode 失败：{e}"))?;
    }
    Ok(out.trim().to_string())
}

// ---------------------------------------------------------------------------
// 加载 / 卸载 / 翻译（均在 worker 线程内执行 = 线程锁）
// ---------------------------------------------------------------------------

fn load_model(app: &AppHandle, slot: &mut Option<Loaded>, path: &str) -> Result<(), String> {
    let path = if path.trim().is_empty() {
        resolve_path(app)
    } else {
        strip_unc(path.trim())
    };
    if path.is_empty() {
        set_status(app, "unloaded", Some("未指定模型路径".to_string()));
        return Err("未设置模型路径".to_string());
    }
    if !std::path::Path::new(&path).is_file() {
        set_status(app, "unloaded", Some(format!("模型文件不存在：{path}")));
        return Err(format!("模型文件不存在：{path}"));
    }
    // 已加载同路径则幂等直接返回
    if let Some(l) = slot {
        if l.path == path {
            set_status(app, "ready", None);
            return Ok(());
        }
    }

    set_status(app, "loading", None);
    // CUDA 构建：默认 999 层全部卸载到 GPU（RTX 4060 Ti 8G 足够放 1.8B Q4）。
    // 无 N 卡/显存不足可在 settings.json 设 "gpuLayers": 0 回退纯 CPU。
    let gpu_layers = crate::settings::current(app).gpu_layers.max(0) as u32;
    let params = llama_cpp_2::model::params::LlamaModelParams::default()
        .with_n_gpu_layers(gpu_layers);
    let model = llama_cpp_2::model::LlamaModel::load_from_file(backend(), &path, &params)
        .map_err(|e| {
            set_status(app, "unloaded", Some(format!("模型加载失败：{e}")));
            format!("模型加载失败：{e}")
        })?;
    *slot = Some(Loaded { path: path.clone(), model });
    set_status(app, "ready", None);
    Ok(())
}

fn unload_model(app: &AppHandle, slot: &mut Option<Loaded>) -> usize {
    if slot.is_none() {
        set_status(app, "unloaded", Some("模型不在内存".to_string()));
        return 0;
    }
    set_status(app, "unloading", None);
    slot.take();
    set_status(app, "unloaded", Some("已释放内存".to_string()));
    1
}

fn resolve_source_name(text: &str, source: &str) -> String {
    let s = source.trim();
    if !s.is_empty() && s != "auto" {
        return constants::lang_en(s);
    }
    // auto：用脚本启发式识别实际语种（假名/谚文/西里尔/泰文/阿拉伯文等都能正确命名，
    // 不再把所有非汉字文本一律标成 "English"，避免误导大模型）。识别不到再回落英文。
    if let Some(code) = crate::mt_engine::detect_script(text) {
        return constants::lang_en(code);
    }
    "English".to_string()
}

/// 结合引擎排序与开关，计算本次请求实际尝试的引擎序列。
/// preferred = None 用用户引擎排序（F1–F4）；Some(引擎) = 该引擎置顶（F5）。
/// 兼容开关：NMT 停用则去掉 opus/nllb；NMT 启用但关闭大模型回落则去掉 gguf。
/// Google（在线翻译）不受 NMT 开关约束——它不是本地 NMT 引擎，停用 NMT 不应关掉它。
fn effective_engines(app: &AppHandle, preferred: Option<&str>) -> Result<Vec<String>, String> {
    let s = crate::settings::current(app);
    let mut order: Vec<String> = Vec::new();
    if let Some(p) = preferred {
        if crate::constants::is_engine(p) {
            order.push(p.to_string());
        }
    }
    for e in &s.engine_order {
        if !order.contains(e) && order.len() < crate::constants::ENGINES.len() {
            order.push(e.clone());
        }
    }
    if !s.nmt_enabled {
        order.retain(|e| e != "opus" && e != "nllb");
    } else if !s.nmt_llm_fallback {
        order.retain(|e| e != "gguf");
    }
    if order.is_empty() {
        return Err("没有可用翻译引擎：NMT 已停用且大模型翻译回落已关闭。请在设置中至少启用一个引擎。".to_string());
    }
    Ok(order)
}

fn translate_inner(
    app: &AppHandle,
    slot: &mut Option<Loaded>,
    text: &str,
    source: &str,
    target: &str,
    engine: Option<&str>,
) -> Result<String, String> {
    // 硬兑底疑问短句：命中即返回确定性译文，完全不经过任何模型/引擎（装完即用、永不退化）。
    if let Some(hit) = crate::prompt::hard_question_translation(source, target, text) {
        return Ok(hit);
    }
    set_status(app, "translating", None);
    let order = effective_engines(app, engine)?;
    // NMT 引擎即使「加载/翻译失败」也继续尝试后续引擎（如 NLLB 目录残缺时回落 GGUF），
    // 全部失败才报错——报错时用第一个真实错误（比笼统的「未命中」更有诊断价值）。
    let mut first_err: Option<String> = None;
    for eng in &order {
        match eng.as_str() {
            // 在线引擎（Google Translate）：断网 / 被墙 / 限流 → 记下错误继续下一引擎
            "google" => {
                match crate::google_engine::translate_text(app, text, source, target) {
                    Ok(Some(out)) => return Ok(out),
                    Ok(None) => {}
                    Err(err) => {
                        if first_err.is_none() {
                            first_err = Some(err);
                        }
                    }
                }
            }
            // NMT 快速引擎：命中即返回，不加载大模型
            e @ ("opus" | "nllb") => {
                match crate::mt_engine::translate_text_engine(app, text, source, target, e) {
                    Ok(Some(out)) => return Ok(out),
                    Ok(None) => {}
                    Err(err) => {
                        if first_err.is_none() {
                            first_err = Some(err);
                        }
                    }
                }
            }
            // 大模型（GGUF，最高质量）
            "gguf" => return translate_llm(app, slot, text, source, target),
            _ => {}
        }
    }
    Err(first_err.unwrap_or_else(|| {
        "没有可用翻译引擎命中。请检查引擎排序或安装相应模型。".to_string()
    }))
}

/// 判定一次生成是否不可接受（需换「配对补全」格式重试）：
/// 指令回声/问句/垃圾符号（looks_like_instruction_echo）、「我是翻译」式自我介绍（looks_like_self_answer），
/// 以及「clean 后译文仍等于原文」（如 `#insane#` 剥 `#` 后仍是 insane）——后者正是
/// F1 短词项「输入英文译文还是英文」的根因：模型把词回声成 #word#，剥完==原文必判坏重试。
fn is_bad_output(translated: &str, original: &str) -> bool {
    crate::prompt::looks_like_instruction_echo(translated)
        || crate::prompt::looks_like_self_answer(translated)
        || norm_eq(translated, original)
}

/// 归一后比较：去全角/半角空格、忽略大小写（"insane"≡"insane"、"谢谢"≡"谢谢"）。
fn norm_eq(a: &str, b: &str) -> bool {
    let na: String = a.chars().filter(|c| !c.is_whitespace()).collect();
    let nb: String = b.chars().filter(|c| !c.is_whitespace()).collect();
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    na.eq_ignore_ascii_case(&nb) || na == nb
}

/// 大模型翻译：加载 Hy-MT2 GGUF 并分段推理。
fn translate_llm(
    app: &AppHandle,
    slot: &mut Option<Loaded>,
    text: &str,
    source: &str,
    target: &str,
) -> Result<String, String> {
    if slot.is_none() {
        load_model(app, slot, "")?;
    }
    let target_name = resolve_target(text, source, target);
    let source_name = resolve_source_name(text, source);
    let segments = segmenter::segment_text(text)?;
    if segments.is_empty() {
        return Ok(String::new());
    }
    let m = &slot.as_ref().expect("model loaded").model;
    let mut parts = Vec::with_capacity(segments.len());
    for seg in &segments {
        let prompt = crate::prompt::translate_prompt(&source_name, &target_name, seg);
        let budget = segmenter::translate_budget(seg);
        let raw = generate(m, &prompt, budget)?;
        let mut got = crate::prompt::clean_translation(&raw);
        // 撞指令词（如「翻译」）主格式会回声指令；自我指代式元回答（如「我是翻译人员」/
        // 私は翻訳者です）说明模型把输入当成了直接提问。两者都换「配对补全」格式重试一次
        if is_bad_output(&got, seg) {
            let retry = crate::prompt::translate_prompt_retry(&source_name, &target_name, seg);
            let raw2 = generate(m, &retry, budget)?;
            let got2 = crate::prompt::clean_translation(&raw2);
            if !is_bad_output(&got2, &seg) {
                got = got2;
            }
        }
        parts.push(got);
    }
    Ok(parts.join("\n"))
}

/// 批量翻译逐行文本（覆盖原文用；逐行独立推理，行级文本天然各成一句）。
/// 返回与输入等长的 Vec：成功为译文，失败对应行退回原文（保证覆盖时行数对齐）。
fn translate_lines_inner(
    app: &AppHandle,
    slot: &mut Option<Loaded>,
    texts: &[String],
    source: &str,
    target: &str,
    engine: Option<&str>,
) -> Result<Vec<String>, String> {
    set_status(app, "translating", None);
    let order = effective_engines(app, engine)?;
    // 与 translate_inner 同策略：NMT 失败记下错误继续，全部失败才报错（优先展示首个真实错误）
    let mut first_err: Option<String> = None;
    for eng in &order {
        match eng.as_str() {
            // 在线引擎（Google Translate）：逐行并发请求，失败行退回原文保证行数对齐
            "google" => {
                match crate::google_engine::translate_lines(app, texts, source, target) {
                    Ok(Some(out)) => return Ok(out),
                    Ok(None) => {}
                    Err(err) => {
                        if first_err.is_none() {
                            first_err = Some(err);
                        }
                    }
                }
            }
            // F3 覆盖原文走 NMT 行级批量（一次 CT2 批推理，速度远超大模型）
            e @ ("opus" | "nllb") => {
                match crate::mt_engine::translate_lines_engine(app, texts, source, target, e) {
                    Ok(Some(out)) => return Ok(out),
                    Ok(None) => {}
                    Err(err) => {
                        if first_err.is_none() {
                            first_err = Some(err);
                        }
                    }
                }
            }
            "gguf" => {
                return translate_lines_llm(app, slot, texts, source, target);
            }
            _ => {}
        }
    }
    Err(first_err.unwrap_or_else(|| {
        "没有可用翻译引擎命中。请检查引擎排序或安装相应模型。".to_string()
    }))
}

/// 大模型行级翻译（逐行推理，失败行退回原文保证行数对齐）。
fn translate_lines_llm(
    app: &AppHandle,
    slot: &mut Option<Loaded>,
    texts: &[String],
    source: &str,
    target: &str,
) -> Result<Vec<String>, String> {
    if slot.is_none() {
        load_model(app, slot, "")?;
    }
    let target_name = resolve_target(&texts.join("\n"), source, target);
    let source_name = resolve_source_name(&texts.join("\n"), source);
    let m = &slot.as_ref().expect("model loaded").model;
    let mut out = Vec::with_capacity(texts.len());
    for seg in texts {
        let prompt = crate::prompt::translate_prompt(&source_name, &target_name, seg);
        let budget = segmenter::translate_budget(seg);
        let raw = match generate(m, &prompt, budget) {
            Ok(r) => r,
            Err(_) => {
                out.push(seg.clone());
                continue;
            }
        };
        let mut got = crate::prompt::clean_translation(&raw);
        if is_bad_output(&got, seg) {
            let retry = crate::prompt::translate_prompt_retry(&source_name, &target_name, seg);
            let raw2 = generate(m, &retry, budget)?;
            let got2 = crate::prompt::clean_translation(&raw2);
            if !is_bad_output(&got2, seg) {
                got = got2;
            }
        }
        out.push(got);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 工作线程主体（单所有者：加载/推理/卸载天然串行）
// ---------------------------------------------------------------------------

fn dispatch(app: &AppHandle, model: &mut Option<Loaded>, cmd: ModelCmd) {
    match cmd {
        ModelCmd::Load { path, reply } => {
            let result = load_model(app, model, &path);
            let _ = reply.send(result.map(|_| String::new()));
        }
        ModelCmd::Unload { reply } => {
            let n = unload_model(app, model);
            let _ = reply.send(Ok(format!("已释放 {n} 个模型")));
        }
        ModelCmd::Translate { text, source, target, engine, reply } => {
            let text = text.trim().to_string();
            let result = match text.as_str() {
                "" => Err("输入为空".to_string()),
                _ => translate_inner(app, model, &text, &source, &target, engine.as_deref()),
            };
            if let Ok(translated) = &result {
                if !translated.is_empty() {
                    record_history(app, &text, &source, &target, translated);
                }
            }
            let _ = reply.send(result);
            // NMT 命中时大模型未加载：状态复位为 unloaded，避免卡在「翻译中」
            if model.is_some() {
                set_status(app, "ready", None);
            } else {
                set_status(app, "unloaded", None);
            }
        }
        ModelCmd::TranslateLines { texts, source, target, engine, reply } => {
            let texts: Vec<String> = texts
                .into_iter()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            let result = translate_lines_inner(app, model, &texts, &source, &target, engine.as_deref());
            let _ = reply.send(result);
            if model.is_some() {
                set_status(app, "ready", None);
            } else {
                set_status(app, "unloaded", None);
            }
        }
    }
}

async fn run_worker(app: AppHandle, mut rx: mpsc::UnboundedReceiver<ModelCmd>) {
    let mut model: Option<Loaded> = None;

    loop {
        // 模型在内存：等待「新命令 或 闲置超时」两条路。每轮都重新武装计时，
        // 保证每条命令后闲置卸载都从零重置——旧实现在第二条命令后计时被永久旁路，
        // 模型此后再也无法自动释放内存。
        if model.is_some() {
            let secs = idle_secs(&app);
            let sleep = tokio::time::sleep(std::time::Duration::from_secs(secs));
            tokio::pin!(sleep);
            tokio::select! {
                cmd = rx.recv() => {
                    if let Some(cmd) = cmd {
                        dispatch(&app, &mut model, cmd);
                    }
                }
                _ = &mut sleep => {
                    set_status(&app, "unloading", None);
                    model.take();
                    set_status(&app, "unloaded", Some("闲置超时，已释放内存".to_string()));
                }
            }
            continue;
        }
        // 模型不在内存：阻塞等命令
        let Some(cmd) = rx.recv().await else { break };
        dispatch(&app, &mut model, cmd);
    }
}

// ---------------------------------------------------------------------------
// 命令面（供前端 / 热键分发调用）
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn model_status(app: AppHandle) -> ModelStatus {
    app.state::<crate::state::AppState>().model_status.lock().unwrap().clone()
}

#[tauri::command]
pub async fn model_load(app: AppHandle, path: String) -> Result<String, String> {
    if !path.trim().is_empty() {
        let mut s = crate::settings::current(&app);
        s.model_path = path.trim().to_string();
        crate::settings::apply(&app, s)?;
    }
    let (reply, rx) = oneshot::channel();
    send(&app, ModelCmd::Load { path, reply })?;
    rx.await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn model_unload(app: AppHandle) -> Result<String, String> {
    let (reply, rx) = oneshot::channel();
    send(&app, ModelCmd::Unload { reply })?;
    rx.await.map_err(|e| e.to_string())?
}

/// 历史缓存查找：同句同语言已翻过 → 直接复用旧译文，不再触发推理。
/// history_limit==0 时 history 向量始终为空，无需额外判断。
fn history_lookup(app: &AppHandle, text: &str, source: &str, target: &str) -> Option<String> {
    let state = app.state::<crate::state::AppState>();
    let items = state.history.lock().unwrap();
    items
        .iter()
        .find(|h| h.text == text && h.src == source && h.tgt == target && !h.translated.is_empty())
        .map(|h| h.translated.clone())
}

/// F1–F4 统一目标解析：target=="auto" 时按「首选/次选」语言规则决定实际目标。
/// 识别源语言：source 显式非 auto 直接用；否则用脚本启发式 detect_script。
/// 识别结果 == 首选 → 译入次选；识别结果 != 首选（含识别失败）→ 译入首选。
fn quick_target(text: &str, source: &str, target: &str, preferred: &str, secondary: &str) -> String {
    if target.trim() != "auto" {
        return target.trim().to_string();
    }
    let rec = if source.trim().is_empty() || source.trim() == "auto" {
        crate::mt_engine::detect_script(text).unwrap_or("").to_string()
    } else {
        source.trim().to_string()
    };
    if !rec.is_empty() && rec == preferred {
        secondary.to_string()
    } else {
        preferred.to_string()
    }
}

#[tauri::command]
pub async fn translate_text(
    app: AppHandle,
    text: String,
    source: String,
    target: String,
    engine: Option<String>,
) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("输入为空".to_string());
    }
    let target = {
        let s = crate::settings::current(&app);
        quick_target(&text, &source, &target, &s.preferred_lang, &s.secondary_lang)
    };
    // 历史命中直接返回，跳过模型推理与 record_history（避免重复记录）
    if let Some(cached) = history_lookup(&app, &text, &source, &target) {
        return Ok(cached);
    }
    let (reply, rx) = oneshot::channel();
    send(&app, ModelCmd::Translate { text, source, target, engine, reply })?;
    rx.await.map_err(|e| e.to_string())?
}

/// F3 覆盖原文：批量翻译逐行文本（一次串行推理；行级回落原文保证对齐）。
#[tauri::command]
pub async fn translate_lines(
    app: AppHandle,
    texts: Vec<String>,
    source: String,
    target: String,
    engine: Option<String>,
) -> Result<Vec<String>, String> {
    let texts: Vec<String> = texts
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let target = {
        let s = crate::settings::current(&app);
        quick_target(&texts.join("\n"), &source, &target, &s.preferred_lang, &s.secondary_lang)
    };
    let (reply, rx) = oneshot::channel();
    send(&app, ModelCmd::TranslateLines { texts, source, target, engine, reply })?;
    rx.await.map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 历史记录（镜像 + 落盘 + 事件）
// ---------------------------------------------------------------------------

/// 翻译成功后追加一条历史（限长由 settings.history_limit 决定；0=不保留）
fn record_history(app: &AppHandle, text: &str, source: &str, target: &str, translated: &str) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let limit = crate::settings::current(app).history_limit;
    if limit == 0 {
        return;
    }
    let item = crate::winutil::HistoryItem {
        ts: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        src: source.to_string(),
        tgt: target.to_string(),
        text: text.to_string(),
        translated: translated.to_string(),
    };
    {
        let state = app.state::<crate::state::AppState>();
        let mut v = state.history.lock().unwrap();
        v.insert(0, item);
        v.truncate(limit as usize);
        let _ = crate::winutil::save_history(&v);
    }
    let _ = app.emit("history-changed", ());
}

#[tauri::command]
pub fn history_get(app: AppHandle) -> Vec<crate::winutil::HistoryItem> {
    let state = app.state::<crate::state::AppState>();
    let items = state.history.lock().unwrap().clone();
    items
}

#[tauri::command]
pub fn history_clear(app: AppHandle) -> Result<(), String> {
    {
        let state = app.state::<crate::state::AppState>();
        let mut v = state.history.lock().unwrap();
        v.clear();
        crate::winutil::save_history(&v)?;
    }
    let _ = app.emit("history-changed", ());
    Ok(())
}

#[cfg(test)]
mod model_tests {
    use super::*;

    /// F1–F4 统一方向规则：target="auto" 时按首选/次选语言决定实际目标。
    /// 识别到首选 → 译入次选；识别到非首选（含识别失败）→ 译入首选。
    #[test]
    fn quick_target_prefers_preferred_secondary_pair() {
        let zh_en = |t: &str, s: &str| quick_target(t, s, "auto", "zh", "en");
        assert_eq!(zh_en("你好，世界", "auto"), "en"); // 中文=首选 → 译入英文
        assert_eq!(zh_en("Hello, world", "auto"), "zh"); // 英文≠首选 → 译入中文
        assert_eq!(zh_en("こんにちは", "auto"), "zh"); // 日文≠首选 → 译入中文
        assert_eq!(zh_en("12345", "auto"), "zh"); // 识别失败 → 译入首选
        assert_eq!(zh_en("你好", "en"), "zh"); // 显式源=en 且≠首选 → 译入首选(中文)
        assert_eq!(zh_en("Hello", "zh"), "en"); // 显式源=zh=首选 → 译入次选
        // 显式 target 优先级高于 auto 规则
        assert_eq!(quick_target("你好", "auto", "ja", "zh", "en"), "ja");
        // 非默认搭配：首选=en 次选=zh（识别到英文→中文）
        let en_zh = |t: &str, s: &str| quick_target(t, s, "auto", "en", "zh");
        assert_eq!(en_zh("Hello, world", "auto"), "zh");
        assert_eq!(en_zh("你好，世界", "auto"), "en");
    }

    /// resolve_source_name 脚本识别：假名/谚文/西里尔等不应标成 English
    #[test]
    fn resolve_source_name_labels_scripts_correctly() {
        assert_eq!(resolve_source_name("こんにちは世界", "auto"), "Japanese");
        assert_eq!(resolve_source_name("안녕하세요", "auto"), "Korean");
        assert_eq!(resolve_source_name("Привет мир", "auto"), "Russian");
        assert_eq!(resolve_source_name("สวัสดี", "auto"), "Thai");
        assert_eq!(resolve_source_name("مرحبا", "auto"), "Arabic");
        assert_eq!(resolve_source_name("你好，世界", "auto"), "Chinese");
        assert_eq!(resolve_source_name("Hello world", "auto"), "English");
        assert_eq!(resolve_source_name("12345", "auto"), "English");
        assert_eq!(resolve_source_name("", "auto"), "English");
        // 显式 source 优先于启发式
        assert_eq!(resolve_source_name("こんにちは", "zh"), "Chinese");
        assert_eq!(resolve_source_name("Hello", "ja"), "Japanese");
    }

    /// 真实模型回归：验证强任务限定提示词确实「只翻译且完整」。
    /// 用法：LINGGO_TEST_MODEL=<gguf 路径> cargo test --release --lib model_tests -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_model_translates_in_full() {
        let path = std::env::var("LINGGO_TEST_MODEL").expect("set LINGGO_TEST_MODEL");
        let model = llama_cpp_2::model::LlamaModel::load_from_file(
            backend(),
            &strip_unc(&path),
            &llama_cpp_2::model::params::LlamaModelParams::default().with_n_gpu_layers(0),
        )
        .expect("load model");
        let cases = [
            ("想必这就是新婚家具吧", "English"),
            ("他昨晚熬夜看球赛，今天一直打哈欠。", "English"),
            ("这个软件安装后会自动开机自启。", "English"),
            ("Please make sure to bring your passport to the airport.", "Chinese"),
            ("天气好的时候我们经常去公园散步。", "English"),
            ("Knowledge is the new oil.", "Chinese"),
            ("嗡嗡嗡", "English"),
            ("随便", "English"),
            ("随便", "Chinese"),
            ("好的", "English"),
            ("你好", "English"),
            ("再见", "English"),
            ("谢谢", "English"),
            ("翻译", "English"),
            ("翻", "English"),
            ("好", "English"),
            ("谢", "English"),
            ("嗡", "English"),
            ("你是谁", "English"),
            ("你是谁？", "English"),
            ("你是谁呀", "English"),
            ("你叫什么名字", "English"),
        ];
        for (text, tgt) in &cases {
            let src = resolve_source_name(text, "auto");
            let prompt = crate::prompt::translate_prompt(&src, tgt, text);
            let budget = crate::segmenter::translate_budget(text);
            let raw = generate(&model, &prompt, budget).expect("generate");
            let mut got = crate::prompt::clean_translation(&raw);
            if is_bad_output(&got, text) {
                let retry = crate::prompt::translate_prompt_retry(&src, tgt, text);
                let raw2 = generate(&model, &retry, budget).expect("generate");
                let got2 = crate::prompt::clean_translation(&raw2);
                println!("  retry for {text:?}: raw2={raw2:?}");
                if !is_bad_output(&got2, text) {
                    got = got2;
                }
            }
            println!("in={text:?} raw={raw:?} got={got:?}");
            assert!(
                got.len() >= text.trim().len() / 3,
                "译文过短：{got:?} (raw={raw:?})"
            );
        }
    }

    /// 实验：短词（含撞指令词「翻译」）+ 长句稳定性。对比「避 Translate 动词」的措辞。
    /// 结论依据：system 含 "Translate the ..." 时「翻译」撞词回声；避动词 + 对齐 user 应双稳。
    #[test]
    #[ignore]
    fn probe_short_word_stability() {
        let path = std::env::var("LINGGO_TEST_MODEL").expect("set LINGGO_TEST_MODEL");
        let model = llama_cpp_2::model::LlamaModel::load_from_file(
            backend(),
            &strip_unc(&path),
            &llama_cpp_2::model::params::LlamaModelParams::default().with_n_gpu_layers(0),
        )
        .expect("load model");
        let cat = |sys: &str, usr: &str| -> String {
            format!(
                "<|im_start|>system\n{sys}<|im_end|>\n<|im_start|>user\n{usr}<|im_end|>\n<|im_start|>assistant\n"
            )
        };
        let sys_tran = |src: &str, tgt: &str| -> String {
            format!(
                "You are a professional translator. Convert the {src} text to {tgt}. Output only the {tgt} text."
            )
        };
        let build = |sys: &str, src: &str, src_name: &str, tgt_name: &str, w: &str| {
            cat(&sys, &format!("{src_name}: {w}\n{tgt_name}:"))
        };

        let words = [
            ("翻译", "Chinese", "English"),
            ("好", "Chinese", "English"),
            ("谢", "Chinese", "English"),
            ("嗡", "Chinese", "English"),
            ("随便", "Chinese", "English"),
            ("你好", "Chinese", "English"),
            ("再见", "Chinese", "English"),
            ("谢谢", "Chinese", "English"),
        ];
        let sentences = [
            ("他昨晚熬夜看球赛，今天一直打哈欠。", "Chinese", "English"),
            ("这个软件安装后会自动开机自启。", "Chinese", "English"),
            ("天气好的时候我们经常去公园散步。", "Chinese", "English"),
            ("Please make sure to bring your passport to the airport.", "English", "Chinese"),
            ("Knowledge is the new oil.", "English", "Chinese"),
        ];

        let styles: Vec<(&str, Box<dyn Fn(&str, &str) -> String>)> = vec![
            ("zh__sys（用户方案）", Box::new(|_src: &str, _tgt: &str| "你是一名专业的翻译器。只输出译文，不要多余文字。不要使用 Markdown，不要加粗，不要多余换行。".to_string())),
            ("tran（英文 Convert 式）", Box::new(sys_tran)),
        ];
        for (name, sysf) in &styles {
            println!("===== style {name} ===== WORDS");
            for (w, src, tgt) in &words {
                let p = build(&sysf(src, tgt), src, src, tgt, w);
                let raw = generate(&model, &p, 64).expect("generate");
                println!(
                    "{w:?} raw={raw:?} got={got:?}",
                    got = crate::prompt::clean_translation(&raw)
                );
            }
            println!("===== style {name} ===== SENT");
            for (s, src, tgt) in &sentences {
                let p = build(&sysf(src, tgt), src, src, tgt, s);
                let raw = generate(&model, &p, 384).expect("generate");
                println!(
                    "{s:?} raw={raw:?} got={got:?}",
                    got = crate::prompt::clean_translation(&raw)
                );
            }
        }
    }
}