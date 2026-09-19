// Linggo 常量：版本号、默认热键、闲置超时选项、Hy-MT2 支持的 33 种语言。
// 33 语种为可调数据表：改这里即可增减语言（需求固定为 Hy-MT2 支持的 33 种）。

pub const APP_VERSION: &str = "0.1.1";

/// 默认全局热键（与需求一致）
pub const DEFAULT_HOTKEYS: [(&str, &str); 5] = [
    ("f1", "F1"),
    ("f2", "F2"),
    ("f3", "F3"),
    ("f4", "F4"),
    ("f5", "F5"),
];

/// 闲置超时可选值（秒）；0 = 永久常驻
pub const IDLE_CHOICES: [u64; 4] = [5, 15, 30, 0];
pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 15;

/// 历史记录数可选值；0 = 不保留
pub const HISTORY_CHOICES: [u32; 5] = [0, 20, 50, 100, 200];
pub const DEFAULT_HISTORY_LIMIT: u32 = 50;

/// 首选语言（默认中文）：F1–F4 识别到该语言时译为次选语言
pub const DEFAULT_PREFERRED_LANG: &str = "zh";
/// 次选语言（默认英语）：F1–F4 识别到非首选语言时译为首选语言
pub const DEFAULT_SECONDARY_LANG: &str = "en";

// ---------------------------------------------------------------------------
// 翻译引擎（OPUS/NLLB 为 NMT 快速引擎，GGUF 为大模型）
// ---------------------------------------------------------------------------

/// 引擎标识（与配置 engine_order / f5_engine 一致）
pub const ENGINES: [&str; 3] = ["opus", "nllb", "gguf"];

/// 各引擎二字特色描述（与 ENGINES 一一对应）
pub const ENGINE_TAGS: [&str; 3] = ["极快", "快速", "质量"];

/// 默认引擎排序（F1–F4 快速功能按此顺序优先命中）
pub const DEFAULT_ENGINE_ORDER: [&str; 3] = ["opus", "nllb", "gguf"];

/// F5 主窗口默认引擎：最高质量大模型（可在设置单独改）
pub const DEFAULT_F5_ENGINE: &str = "gguf";

pub fn is_engine(s: &str) -> bool {
    ENGINES.contains(&s)
}

/// 单条语言定义：code(ISO639-1 短码) / 中文名 / 英文名（喂给大模型的语种名称）
#[derive(Debug, Clone, Copy)]
pub struct Lang {
    pub code: &'static str,
    pub zh: &'static str,
    pub en: &'static str,
}

/// Hy-MT2 支持的 33 种语言（互相翻译）
pub const LANGS33: [Lang; 33] = [
    Lang { code: "en", zh: "英语", en: "English" },
    Lang { code: "zh", zh: "中文（简体）", en: "Chinese" },
    Lang { code: "ja", zh: "日语", en: "Japanese" },
    Lang { code: "ko", zh: "韩语", en: "Korean" },
    Lang { code: "fr", zh: "法语", en: "French" },
    Lang { code: "de", zh: "德语", en: "German" },
    Lang { code: "es", zh: "西班牙语", en: "Spanish" },
    Lang { code: "it", zh: "意大利语", en: "Italian" },
    Lang { code: "pt", zh: "葡萄牙语", en: "Portuguese" },
    Lang { code: "ru", zh: "俄语", en: "Russian" },
    Lang { code: "ar", zh: "阿拉伯语", en: "Arabic" },
    Lang { code: "hi", zh: "印地语", en: "Hindi" },
    Lang { code: "vi", zh: "越南语", en: "Vietnamese" },
    Lang { code: "th", zh: "泰语", en: "Thai" },
    Lang { code: "id", zh: "印尼语", en: "Indonesian" },
    Lang { code: "ms", zh: "马来语", en: "Malay" },
    Lang { code: "tr", zh: "土耳其语", en: "Turkish" },
    Lang { code: "nl", zh: "荷兰语", en: "Dutch" },
    Lang { code: "pl", zh: "波兰语", en: "Polish" },
    Lang { code: "uk", zh: "乌克兰语", en: "Ukrainian" },
    Lang { code: "sv", zh: "瑞典语", en: "Swedish" },
    Lang { code: "da", zh: "丹麦语", en: "Danish" },
    Lang { code: "fi", zh: "芬兰语", en: "Finnish" },
    Lang { code: "no", zh: "挪威语", en: "Norwegian" },
    Lang { code: "cs", zh: "捷克语", en: "Czech" },
    Lang { code: "hu", zh: "匈牙利语", en: "Hungarian" },
    Lang { code: "ro", zh: "罗马尼亚语", en: "Romanian" },
    Lang { code: "bg", zh: "保加利亚语", en: "Bulgarian" },
    Lang { code: "hr", zh: "克罗地亚语", en: "Croatian" },
    Lang { code: "sk", zh: "斯洛伐克语", en: "Slovak" },
    Lang { code: "sl", zh: "斯洛文尼亚语", en: "Slovenian" },
    Lang { code: "he", zh: "希伯来语", en: "Hebrew" },
    Lang { code: "el", zh: "希腊语", en: "Greek" },
];

pub fn lang_by_code(code: &str) -> Option<&'static Lang> {
    LANGS33.iter().find(|l| l.code == code)
}

/// 取某语言的中文名（未知原样返回）
pub fn lang_zh(code: &str) -> String {
    lang_by_code(code).map(|l| l.zh.to_string()).unwrap_or_else(|| code.to_string())
}

/// 取某语言的英文名（喂给大模型更可靠）
pub fn lang_en(code: &str) -> String {
    lang_by_code(code).map(|l| l.en.to_string()).unwrap_or_else(|| code.to_string())
}

/// 校验语言码："auto" 或 33 语种之一合法
pub fn is_valid_lang(code: &str) -> bool {
    code == "auto" || lang_by_code(code).is_some()
}