// Linggo 主窗口逻辑（步骤 8）：双栏翻译、33 语种、状态栏、设置抽屉、历史抽屉。

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { applySystemAccent, watchSystemAccent } from "./shared";

// ---------------------------------------------------------------------------
// 类型（与 Rust serde(rename_all=camelCase) 对应）
// ---------------------------------------------------------------------------

interface Hotkeys {
  f1: string;
  f2: string;
  f3: string;
  f4: string;
  f5: string;
}

// 翻译引擎元信息（与 Rust constants::ENGINES / ENGINE_TAGS 一一对应）
const ENGINE_META: Record<string, string> = { opus: "极快", nllb: "快速", gguf: "质量" };
const ENGINE_LABEL = (code: string): string => `${code.toUpperCase()} · ${ENGINE_META[code] ?? ""}`;
const DEFAULT_ENGINE_ORDER = ["opus", "nllb", "gguf"];

interface Settings {
  theme: string;
  hotkeys: Hotkeys;
  idleTimeoutSecs: number;
  defaultSource: string;
  defaultTarget: string;
  modelPath: string;
  ocrLang: string;
  gameModeBlock: boolean;
  historyLimit: number;
  autostart: boolean;
  pinDefaultOverlay: boolean;
  pinShowTip: boolean;
  nmtEnabled: boolean;
  engineOrder: string[];
  f5Engine: string;
  nmtOpusZhEn: string;
  nmtOpusEnZh: string;
  nmtNllbDir: string;
  nmtLlmFallback: boolean;
  packageIndexUrl: string;
  checkUpdatesEnabled: boolean;
  preferredLang: string;
  secondaryLang: string;
}

// 更新检查结果（与 Rust updater::UpdateInfo 对应，camelCase）
interface UpdateInfo {
  hasUpdate: boolean;
  current: string;
  latest: string;
  url: string;
  note: string | null;
  error: string | null;
}

interface MtStatus {
  enabled: boolean;
  priority: string;
  opusZhEn: string;
  opusEnZh: string;
  nllb: string;
  active: string;
  note: string;
}

interface PkgEntry {
  id: string;
  kind: string;
  name: string;
  fromCode: string;
  toCode: string;
  baseUrl: string;
  files: string[];
  sizeBytes: number;
  version: string;
}

interface InstalledPkg {
  id: string;
  name: string;
  kind: string;
  dir: string;
  sizeBytes: number;
}

interface PkgList {
  available: PkgEntry[];
  installed: InstalledPkg[];
  source: string;
}

interface PkgProgress {
  id: string;
  file: string;
  status: string;
  index: number;
  total: number;
  bytes: number;
  bytesTotal: number;
}

interface ModelStatus {
  state: string;
  path: string;
  error: string | null;
}

interface UiStatus {
  gameMode: boolean;
}

interface HistoryItem {
  ts: number;
  src: string;
  tgt: string;
  text: string;
  translated: string;
}

// ---------------------------------------------------------------------------
// 33 语种（与 constants.rs LANGS33 一致）
// ---------------------------------------------------------------------------

const LANGS: { code: string; zh: string; en: string }[] = [
  { code: "en", zh: "英语", en: "English" },
  { code: "zh", zh: "中文（简体）", en: "Chinese" },
  { code: "ja", zh: "日语", en: "Japanese" },
  { code: "ko", zh: "韩语", en: "Korean" },
  { code: "fr", zh: "法语", en: "French" },
  { code: "de", zh: "德语", en: "German" },
  { code: "es", zh: "西班牙语", en: "Spanish" },
  { code: "it", zh: "意大利语", en: "Italian" },
  { code: "pt", zh: "葡萄牙语", en: "Portuguese" },
  { code: "ru", zh: "俄语", en: "Russian" },
  { code: "ar", zh: "阿拉伯语", en: "Arabic" },
  { code: "hi", zh: "印地语", en: "Hindi" },
  { code: "vi", zh: "越南语", en: "Vietnamese" },
  { code: "th", zh: "泰语", en: "Thai" },
  { code: "id", zh: "印尼语", en: "Indonesian" },
  { code: "ms", zh: "马来语", en: "Malay" },
  { code: "tr", zh: "土耳其语", en: "Turkish" },
  { code: "nl", zh: "荷兰语", en: "Dutch" },
  { code: "pl", zh: "波兰语", en: "Polish" },
  { code: "uk", zh: "乌克兰语", en: "Ukrainian" },
  { code: "sv", zh: "瑞典语", en: "Swedish" },
  { code: "da", zh: "丹麦语", en: "Danish" },
  { code: "fi", zh: "芬兰语", en: "Finnish" },
  { code: "no", zh: "挪威语", en: "Norwegian" },
  { code: "cs", zh: "捷克语", en: "Czech" },
  { code: "hu", zh: "匈牙利语", en: "Hungarian" },
  { code: "ro", zh: "罗马尼亚语", en: "Romanian" },
  { code: "bg", zh: "保加利亚语", en: "Bulgarian" },
  { code: "hr", zh: "克罗地亚语", en: "Croatian" },
  { code: "sk", zh: "斯洛伐克语", en: "Slovak" },
  { code: "sl", zh: "斯洛文尼亚语", en: "Slovenian" },
  { code: "he", zh: "希伯来语", en: "Hebrew" },
  { code: "el", zh: "希腊语", en: "Greek" },
];

const IDLE_CHOICES = [5, 15, 30, 0];
const HISTORY_CHOICES = [0, 20, 50, 100, 200];
const OCR_LANGS = [
  { code: "auto", zh: "自动（中英混排优先）" },
  { code: "zh", zh: "中文" },
  { code: "en", zh: "英语" },
  { code: "ja", zh: "日语" },
  { code: "ko", zh: "韩语" },
  { code: "de", zh: "德语" },
  { code: "fr", zh: "法语" },
  { code: "es", zh: "西班牙语" },
  { code: "it", zh: "意大利语" },
  { code: "pt", zh: "葡萄牙语" },
  { code: "ru", zh: "俄语" },
];

function langZh(code: string): string {
  const l = LANGS.find((x) => x.code === code);
  return l ? l.zh : code;
}

// ---------------------------------------------------------------------------
// DOM 快捷访问
// ---------------------------------------------------------------------------

function $<T extends HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error("missing element: " + id);
  return el as T;
}

const inputArea = $<HTMLTextAreaElement>("inputArea");
const outputArea = $<HTMLTextAreaElement>("outputArea");
const sourceSel = $<HTMLSelectElement>("sourceSelect");
const targetSel = $<HTMLSelectElement>("targetSelect");
const translateBtn = $<HTMLButtonElement>("translateBtn");
const inputHint = $<HTMLSpanElement>("inputHint");

// ---------------------------------------------------------------------------
// 状态
// ---------------------------------------------------------------------------

let settings: Settings | null = null;
let history: HistoryItem[] = [];
let translating = false;
let gameMode = false;
let toastTimer: ReturnType<typeof setTimeout> | undefined;
/** 新版本入口状态（有更新时显示右下角按钮） */
let updateFabTarget: UpdateInfo | null = null;

const memory = {
  get input() {
    return localStorage.getItem("mt2.input") ?? "";
  },
  set input(v: string) {
    localStorage.setItem("mt2.input", v);
  },
  get output() {
    return localStorage.getItem("mt2.output") ?? "";
  },
  set output(v: string) {
    localStorage.setItem("mt2.output", v);
  },
  get src() {
    return localStorage.getItem("mt2.src") ?? "";
  },
  set src(v: string) {
    localStorage.setItem("mt2.src", v);
  },
  get tgt() {
    return localStorage.getItem("mt2.tgt") ?? "";
  },
  set tgt(v: string) {
    localStorage.setItem("mt2.tgt", v);
  },
};

// ---------------------------------------------------------------------------
// 提示 / 状态栏
// ---------------------------------------------------------------------------

function toast(msg: string, isErr = false) {
  const t = $<HTMLDivElement>("toast");
  t.textContent = msg;
  t.className = "toast" + (isErr ? " err" : "");
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.className = "toast hidden"), 3200);
}

const MODEL_LABEL: Record<string, string> = {
  unloaded: "模型未加载",
  loading: "模型加载中…",
  ready: "模型就绪",
  translating: "翻译中…",
  unloading: "闲置释放中…",
};

function renderModelStatus(st: ModelStatus) {
  const el = $<HTMLSpanElement>("modelState");
  el.textContent = MODEL_LABEL[st.state] ?? st.state;
  const line = $<HTMLSpanElement>("modelStatusLine");
  if (st.state === "ready") {
    line.className = "model-status-line ok";
    line.textContent = `已加载：${st.path || "（未显示路径）"}`;
  } else if (st.error) {
    line.className = "model-status-line err";
    line.textContent = st.error;
  } else {
    line.className = "model-status-line";
    line.textContent = st.state === "loading" ? "正在加载模型…" : "未加载";
  }
}

function renderGame(u: UiStatus) {
  gameMode = u.gameMode;
  $<HTMLSpanElement>("gameState").classList.toggle("hidden", !gameMode);
}

// ---------------------------------------------------------------------------
// 软件更新：右下角圆形入口 + 设置页「检查更新」
// ---------------------------------------------------------------------------

function updateStatusLine(msg: string, isOk = true) {
  const line = $<HTMLDivElement>("updateStatusLine");
  line.className = "model-status-line" + (isOk ? " ok" : " err");
  line.textContent = msg;
}

/** 有新版时显示右下角圆形按钮；无则隐藏 */
function showUpdateFab(info: UpdateInfo) {
  updateFabTarget = info;
  const fab = $<HTMLButtonElement>("updateFab");
  fab.classList.toggle("hidden", !info.hasUpdate);
  if (info.hasUpdate) {
    fab.title = `发现新版本 v${info.latest}（当前 v${info.current}）—— 点击打开发布页`;
  }
}

function bindUpdateControls() {
  $<HTMLButtonElement>("updateFab").addEventListener("click", () => {
    if (updateFabTarget) void openUpdatePage(updateFabTarget);
  });
}

/** 打开发布页（右上角按钮 / 手动检查命中的引导一致） */
async function openUpdatePage(info: UpdateInfo) {
  try {
    await invoke("open_url", { url: info.url });
  } catch (e) {
    toast(String(e), true);
  }
}

/** 设置页「检查更新」：不受自动检测开关限制 */
function manualUpdateCheck() {
  const btn = $<HTMLButtonElement>("updateCheckBtn");
  btn.disabled = true;
  btn.textContent = "检查中…";
  void invoke<UpdateInfo>("check_update")
    .then((info) => {
      if (info.hasUpdate) {
        updateStatusLine(`发现新版本 v${info.latest}（当前 v${info.current}），请在主窗打开入口`, true);
        showUpdateFab(info);
        toast(`发现新版本 v${info.latest}`);
      } else {
        updateStatusLine(`已是最新版本（v${info.current}）`, true);
        toast("已是最新版本");
      }
    })
    .catch((e) => {
      updateStatusLine(String(e), false);
      toast(String(e), true);
    })
    .finally(() => {
      btn.disabled = false;
      btn.textContent = "检查更新";
    });
}

function toggleUpdateControls(enable: boolean) {
  const btn = $<HTMLButtonElement>("updateCheckBtn");
  if (enable) btn.disabled = false;
}

// ---------------------------------------------------------------------------
// 设置面板读 / 写 / 应用
// ---------------------------------------------------------------------------

function currentSettings(): Settings {
  if (!settings) throw new Error("settings not loaded");
  return settings;
}

async function pushSettings(partial: Partial<Settings>) {
  const next: Settings = { ...currentSettings(), ...partial };
  settings = next;
  try {
    await invoke("settings_set", { value: next });
  } catch (e) {
    toast(String(e), true);
  }
}

function applyTheme() {
  const t = currentSettings().theme || "auto";
  const root = document.documentElement;
  if (t === "auto") delete root.dataset.theme;
  else root.dataset.theme = t;
  // 主题切换后重申系统强调色（内联 --accent 始终覆盖主题默认值）
  void applySystemAccent();
}

function fillSelect(el: HTMLSelectElement, items: { code: string; zh: string }[], extra0?: { code: string; zh: string }) {
  el.textContent = "";
  const opts: { code: string; zh: string }[] = extra0 ? [extra0, ...items] : items;
  for (const o of opts) {
    const opt = document.createElement("option");
    opt.value = o.code;
    opt.textContent = `${o.zh} (${o.code})`;
    el.appendChild(opt);
  }
}

/** 填充引擎选择框（OPUS·极快 / NLLB·快速 / GGUF·质量） */
function fillEngineOptions(el: HTMLSelectElement) {
  el.textContent = "";
  for (const code of ["opus", "nllb", "gguf"]) {
    const opt = document.createElement("option");
    opt.value = code;
    opt.textContent = ENGINE_LABEL(code);
    el.appendChild(opt);
  }
}

function applySettingsToUi() {
  const s = currentSettings();
  applyTheme();

  sourceSel.value = s.defaultSource;
  targetSel.value = s.defaultTarget;
  $<HTMLInputElement>("modelPathInput").value = s.modelPath;
  $<HTMLSelectElement>("idleSelect").value = String(s.idleTimeoutSecs);
  $<HTMLSelectElement>("historySelect").value = String(s.historyLimit);
  $<HTMLSelectElement>("themeSelect").value = s.theme;
  $<HTMLSelectElement>("ocrSelect").value = s.ocrLang;
  $<HTMLInputElement>("autostartCheck").checked = s.autostart;
  $<HTMLInputElement>("gameBlockCheck").checked = s.gameModeBlock;
  $<HTMLInputElement>("pinOverlayCheck").checked = s.pinDefaultOverlay;
  $<HTMLInputElement>("pinHintCheck").checked = s.pinShowTip !== false;
  $<HTMLInputElement>("nmtEnabledCheck").checked = s.nmtEnabled;
  const order =
    Array.isArray(s.engineOrder) && s.engineOrder.length === 3 ? s.engineOrder : DEFAULT_ENGINE_ORDER;
  $<HTMLSelectElement>("engineOrder0").value = order[0];
  $<HTMLSelectElement>("engineOrder1").value = order[1];
  $<HTMLSelectElement>("engineOrder2").value = order[2];
  $<HTMLSelectElement>("f5EngineSelect").value = s.f5Engine || "gguf";
  $<HTMLInputElement>("nmtLlmFallbackCheck").checked = s.nmtLlmFallback;
  $<HTMLInputElement>("pkgIndexUrl").value = s.packageIndexUrl || "";
  $<HTMLInputElement>("updateCheckCheck").checked = s.checkUpdatesEnabled !== false;
  $<HTMLSelectElement>("preferredLangSelect").value = s.preferredLang || "zh";
  $<HTMLSelectElement>("secondaryLangSelect").value = s.secondaryLang || "en";
  $<HTMLInputElement>("hkF1").value = s.hotkeys.f1;
  $<HTMLInputElement>("hkF2").value = s.hotkeys.f2;
  $<HTMLInputElement>("hkF3").value = s.hotkeys.f3;
  $<HTMLInputElement>("hkF4").value = s.hotkeys.f4;
  $<HTMLInputElement>("hkF5").value = s.hotkeys.f5;
}

function readHotkeys(): Hotkeys {
  return {
    f1: $<HTMLInputElement>("hkF1").value.trim(),
    f2: $<HTMLInputElement>("hkF2").value.trim(),
    f3: $<HTMLInputElement>("hkF3").value.trim(),
    f4: $<HTMLInputElement>("hkF4").value.trim(),
    f5: $<HTMLInputElement>("hkF5").value.trim(),
  };
}

// 键盘事件 code → 全局热键可解析的按键名（与 Rust 侧 global-hotkey crate 的 parse_key 对齐）
const HK_CODE_RE =
  /^(F([1-9]|1[0-9]|2[0-4])|Numpad(0|1|2|3|4|5|6|7|8|9|Add|Decimal|Divide|Enter|Equal|Multiply|Subtract)|Space|Enter|Escape|Backspace|Delete|Insert|Home|End|PageUp|PageDown|ArrowUp|ArrowDown|ArrowLeft|ArrowRight|PrintScreen|ScrollLock|CapsLock|NumLock|Minus|Equal|Comma|Period|Quote|Semicolon|Slash|Backquote|Backslash|BracketLeft|BracketRight|AudioVolumeUp|AudioVolumeDown|AudioVolumeMute|MediaPlay|MediaPause|MediaStop|MediaPlayPause|MediaTrackNext|MediaTrackPrevious)$/;

/** e.code → 可存进 settings.hotkeys 的组合键片段；不支持/纯修饰键返回 null */
function hotkeyPartFromCode(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  return HK_CODE_RE.test(code) ? code : null;
}

/** 快捷键录入控件：点击聚焦进入录制态，按下组合键即写入设置（Delete/Backspace 清除，Esc/Tab 取消） */
function bindHotkeyRecorders() {
  const keys = ["f1", "f2", "f3", "f4", "f5"] as const;
  for (const key of keys) {
    const el = $<HTMLInputElement>("hk" + key.toUpperCase());
    const enterRecording = () => {
      el.dataset.prevHk = el.value;
      el.value = "请按新的快捷键…";
      el.classList.add("recording");
    };
    const exitRecording = (restore: boolean) => {
      el.classList.remove("recording");
      if (restore) el.value = el.dataset.prevHk ?? "";
      delete el.dataset.prevHk;
    };
    el.addEventListener("focus", (e) => {
      e.preventDefault();
      enterRecording();
    });
    el.addEventListener("blur", () => {
      if (el.classList.contains("recording")) exitRecording(true);
    });
    el.addEventListener("keydown", (e) => {
      if (!el.classList.contains("recording")) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape" || e.key === "Tab") {
        exitRecording(true);
        el.blur();
        return;
      }
      if (e.key === "Backspace" || e.key === "Delete") {
        exitRecording(false);
        el.value = "";
        void pushSettings({ hotkeys: { ...currentSettings().hotkeys, [key]: "" } });
        el.blur();
        return;
      }
      const part = hotkeyPartFromCode(e.code);
      if (!part) return; // 纯修饰键（Ctrl/Shift 等）不作为主键：继续等组合
      const mods: string[] = [];
      if (e.ctrlKey) mods.push("Ctrl");
      if (e.altKey) mods.push("Alt");
      if (e.shiftKey) mods.push("Shift");
      if (e.metaKey) mods.push("Super"); // Windows 键
      exitRecording(false);
      el.value = [...mods, part].join("+");
      void pushSettings({ hotkeys: { ...currentSettings().hotkeys, [key]: el.value } });
      el.blur();
    });
  }
}

function bindSettingsControls() {
  sourceSel.addEventListener("change", () => {
    memory.src = sourceSel.value;
    void pushSettings({ defaultSource: sourceSel.value });
  });
  targetSel.addEventListener("change", () => {
    memory.tgt = targetSel.value;
    void pushSettings({ defaultTarget: targetSel.value });
  });
  $<HTMLSelectElement>("idleSelect").addEventListener("change", (e) =>
    void pushSettings({ idleTimeoutSecs: Number((e.target as HTMLSelectElement).value) }),
  );
  $<HTMLSelectElement>("historySelect").addEventListener("change", (e) => {
    void pushSettings({ historyLimit: Number((e.target as HTMLSelectElement).value) });
  });
  $<HTMLSelectElement>("themeSelect").addEventListener("change", (e) => {
    void pushSettings({ theme: (e.target as HTMLSelectElement).value });
  });
  $<HTMLSelectElement>("ocrSelect").addEventListener("change", (e) =>
    void pushSettings({ ocrLang: (e.target as HTMLSelectElement).value }),
  );
  $<HTMLInputElement>("autostartCheck").addEventListener("change", (e) => {
    void pushSettings({ autostart: (e.target as HTMLInputElement).checked });
  });
  $<HTMLInputElement>("gameBlockCheck").addEventListener("change", (e) => {
    void pushSettings({ gameModeBlock: (e.target as HTMLInputElement).checked });
  });
  $<HTMLInputElement>("pinOverlayCheck").addEventListener("change", (e) => {
    void pushSettings({ pinDefaultOverlay: (e.target as HTMLInputElement).checked });
  });
  $<HTMLInputElement>("pinHintCheck").addEventListener("change", (e) => {
    void pushSettings({ pinShowTip: (e.target as HTMLInputElement).checked });
  });
  $<HTMLInputElement>("nmtEnabledCheck").addEventListener("change", (e) => {
    void pushSettings({ nmtEnabled: (e.target as HTMLInputElement).checked });
  });
  const orderSelects = ["engineOrder0", "engineOrder1", "engineOrder2"] as const;
  orderSelects.forEach((id, i) => {
    $<HTMLSelectElement>(id).addEventListener("change", () => {
      const old =
        Array.isArray(currentSettings().engineOrder) && currentSettings().engineOrder.length === 3
          ? [...currentSettings().engineOrder]
          : [...DEFAULT_ENGINE_ORDER];
      const v = $<HTMLSelectElement>(id).value;
      const dup = old.indexOf(v);
      if (dup !== -1 && dup !== i) {
        old[dup] = old[i]; // 与其他槽位复用同一引擎时交换原值
      }
      old[i] = v;
      orderSelects.forEach((oid, k) => ($<HTMLSelectElement>(oid).value = old[k]));
      void pushSettings({ engineOrder: old });
    });
  });
  $<HTMLSelectElement>("f5EngineSelect").addEventListener("change", (e) => {
    void pushSettings({ f5Engine: (e.target as HTMLSelectElement).value });
  });
  const prefSel = $<HTMLSelectElement>("preferredLangSelect");
  const secSel = $<HTMLSelectElement>("secondaryLangSelect");
  const swapIfEqual = (changed: HTMLSelectElement, other: HTMLSelectElement) => {
    if (changed.value === other.value && other.value !== "auto") {
      other.value = changed.value === "en" ? "zh" : "en";
    }
  };
  prefSel.addEventListener("change", () => {
    swapIfEqual(prefSel, secSel);
    void pushSettings({ preferredLang: prefSel.value, secondaryLang: secSel.value });
  });
  secSel.addEventListener("change", () => {
    swapIfEqual(secSel, prefSel);
    void pushSettings({ preferredLang: prefSel.value, secondaryLang: secSel.value });
  });
  $<HTMLInputElement>("nmtLlmFallbackCheck").addEventListener("change", (e) => {
    void pushSettings({ nmtLlmFallback: (e.target as HTMLInputElement).checked });
  });
  $<HTMLInputElement>("updateCheckCheck").addEventListener("change", (e) => {
    void pushSettings({ checkUpdatesEnabled: (e.target as HTMLInputElement).checked });
  });
  $<HTMLButtonElement>("updateCheckBtn").addEventListener("click", () => void manualUpdateCheck());
  $<HTMLInputElement>("pkgIndexUrl").addEventListener("change", (e) => {
    void pushSettings({ packageIndexUrl: (e.target as HTMLInputElement).value.trim() });
  });
  $<HTMLButtonElement>("nmtUnloadBtn").addEventListener("click", async () => {
    try {
      const r = await invoke<string>("nmt_unload");
      toast(r);
      void invoke<MtStatus>("nmt_status").then(renderNmt);
    } catch (e) {
      toast(String(e), true);
    }
  });
  for (const [id, key] of [
    ["nmtOpusZhEnPath", "nmtOpusZhEn"],
    ["nmtOpusEnZhPath", "nmtOpusEnZh"],
    ["nmtNllbPath", "nmtNllbDir"],
  ] as const) {
    $(id).addEventListener("change", (e) => {
      void pushSettings({ [key]: (e.target as HTMLInputElement).value.trim() } as Partial<Settings>);
    });
  }
  bindHotkeyRecorders();
}

function bindModelControls() {
  const pathInput = $<HTMLInputElement>("modelPathInput");
  $<HTMLButtonElement>("modelBrowseBtn").addEventListener("click", async () => {
    const p = await invoke<string | null>("pick_model");
    if (p) {
      pathInput.value = p;
      void pushSettings({ modelPath: p });
    }
  });
  pathInput.addEventListener("change", () => void pushSettings({ modelPath: pathInput.value.trim() }));
  $<HTMLButtonElement>("modelLoadBtn").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("modelLoadBtn");
    btn.disabled = true;
    try {
      await invoke("model_load", { path: currentSettings().modelPath });
      toast("模型已加载");
    } catch (e) {
      toast(String(e), true);
    } finally {
      btn.disabled = false;
    }
  });
  $<HTMLButtonElement>("modelUnloadBtn").addEventListener("click", async () => {
    try {
      const r = await invoke<string>("model_unload");
      toast(r);
    } catch (e) {
      toast(String(e), true);
    }
  });
  $<HTMLButtonElement>("modelClearBtn").addEventListener("click", () => {
    $<HTMLInputElement>("modelPathInput").value = "";
    void pushSettings({ modelPath: "" });
    toast("已清空模型路径，翻译将仅由 NMT 引擎（OPUS/NLLB）承担");
  });
}

function renderNmt(ms: MtStatus) {
  $<HTMLInputElement>("nmtEnabledCheck").checked = ms.enabled;
  $<HTMLInputElement>("nmtOpusZhEnPath").value = ms.opusZhEn || "";
  $<HTMLInputElement>("nmtOpusEnZhPath").value = ms.opusEnZh || "";
  $<HTMLInputElement>("nmtNllbPath").value = ms.nllb || "";
  const line = $<HTMLDivElement>("nmtStatusLine");
  const head = ms.active === "none" ? "停用" : ms.active === "opus" ? "OPUS 引擎生效" : "NLLB 引擎生效";
  line.className = "model-status-line" + (ms.active === "none" ? " err" : " ok");
  line.textContent = `${head} · ${ms.note}`;
}

function bindNmtControls() {
  const pick = async (kind: string, inputId: string) => {
    try {
      const p = await invoke<string | null>("nmt_pick_dir", { kind });
      if (p) ($<HTMLInputElement>(inputId).value = p);
    } catch (e) {
      toast(String(e), true);
    }
  };
  $<HTMLButtonElement>("nmtOpusZhEnBrowse").addEventListener("click", () => void pick("opus_zh_en", "nmtOpusZhEnPath"));
  $<HTMLButtonElement>("nmtOpusEnZhBrowse").addEventListener("click", () => void pick("opus_en_zh", "nmtOpusEnZhPath"));
  $<HTMLButtonElement>("nmtNllbBrowse").addEventListener("click", () => void pick("nllb", "nmtNllbPath"));
}

// ---------------------------------------------------------------------------
// 模型市场（Package Index）：可用语言包列表 + 已安装列表 + 下载 / 删除
// ---------------------------------------------------------------------------

function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function bindPkgManager() {
  const listEl = $<HTMLDivElement>("pkgList");
  const instEl = $<HTMLDivElement>("pkgInstalled");
  const srcHint = $<HTMLSpanElement>("pkgSourceHint");
  const refreshBtn = $<HTMLButtonElement>("pkgRefreshBtn");
  let installed: InstalledPkg[] = [];
  let confirming: string | null = null;
  let confirmTimer: ReturnType<typeof setTimeout> | undefined;
  let busy: Record<string, boolean> = {};

  const langLabel = (p: PkgEntry) =>
    p.fromCode && p.toCode
      ? `${p.fromCode} → ${p.toCode}`
      : p.kind === "nllb"
        ? "全语种"
        : p.kind === "gguf"
          ? "大模型"
          : p.kind;

  const renderInstalled = () => {
    instEl.textContent = "";
    if (installed.length === 0) {
      const d = document.createElement("div");
      d.className = "pkg-empty";
      d.textContent = "models 目录暂无已安装语言包";
      instEl.appendChild(d);
      return;
    }
    for (const p of installed) {
      const row = document.createElement("div");
      row.className = "pkg-row";
      const info = document.createElement("div");
      info.className = "pkg-info";
      const name = document.createElement("div");
      name.className = "pkg-name";
      name.textContent = p.name;
      const meta = document.createElement("div");
      meta.className = "pkg-meta";
      meta.textContent = `${p.id} · ${fmtSize(p.sizeBytes)}`;
      meta.title = p.dir;
      info.append(name, meta);
      const del = document.createElement("button");
      del.className = "pkg-btn";
      del.textContent = "删除";
      del.addEventListener("click", () => {
        if (confirming !== p.id) {
          confirming = p.id;
          del.textContent = "确认删除";
          if (confirmTimer) clearTimeout(confirmTimer);
          confirmTimer = setTimeout(() => {
            confirming = null;
            del.textContent = "删除";
          }, 4000);
          return;
        }
        confirming = null;
        del.textContent = "删除中…";
        del.disabled = true;
        void invoke<string>("pkg_delete", { id: p.id })
          .then((r) => {
            toast(r);
            void refresh(false);
          })
          .catch((e) => {
            toast(String(e), true);
            del.textContent = "删除";
            del.disabled = false;
          });
      });
      row.append(info, del);
      instEl.appendChild(row);
    }
  };

  const renderAvailable = (entries: PkgEntry[]) => {
    listEl.textContent = "";
    const has = new Set(installed.map((i) => i.id));
    if (entries.length === 0) {
      const d = document.createElement("div");
      d.className = "pkg-empty";
      d.textContent = "定制索引中暂无语言包条目";
      listEl.appendChild(d);
      return;
    }
    for (const p of entries) {
      const row = document.createElement("div");
      row.className = "pkg-row" + (busy[p.id] ? " dl" : "");
      row.dataset.id = p.id;
      const info = document.createElement("div");
      info.className = "pkg-info";
      const name = document.createElement("div");
      name.className = "pkg-name";
      name.textContent = `${p.name}（${langLabel(p)}）`;
      const meta = document.createElement("div");
      meta.className = "pkg-meta";
      meta.textContent = `${p.id} · ${p.files.length} 个文件 · ${fmtSize(p.sizeBytes)}${p.version ? ` · v${p.version}` : ""}`;
      info.append(name, meta);
      const btn = document.createElement("button");
      btn.className = "pkg-btn";
      if (has.has(p.id)) {
        btn.disabled = true;
        btn.textContent = "已安装 ✓";
      } else {
        btn.textContent = "下载";
        const startDownload = () => {
          if (busy[p.id]) {
            void invoke("pkg_cancel_download");
            return;
          }
          if (Object.values(busy).some(Boolean)) {
            toast("已有下载进行中，请先取消或等待完成");
            return;
          }
          busy[p.id] = true;
          btn.textContent = "取消下载";
          void invoke("pkg_download", { id: p.id }).catch((e) => {
            toast(String(e), true);
            busy[p.id] = false;
            btn.textContent = "下载";
          });
        };
        btn.addEventListener("click", startDownload);
      }
      const prog = document.createElement("div");
      prog.className = "pkg-progress";
      prog.dataset.prog = p.id;
      prog.style.display = "none";
      const fill = document.createElement("div");
      fill.className = "pkg-progress-fill";
      fill.style.width = "0%";
      prog.appendChild(fill);
      row.append(info, btn, prog);
      listEl.appendChild(row);
    }
  };

  const refresh = async (force: boolean) => {
    try {
      const r = await invoke<PkgList>("pkg_list", { force });
      installed = r.installed;
      srcHint.textContent = r.source;
      renderInstalled();
      renderAvailable(r.available);
      if (force) toast("模型列表已刷新");
    } catch (e) {
      srcHint.textContent = "";
      toast(String(e), true);
    }
  };

  refreshBtn.addEventListener("click", () => void refresh(true));
  const un = listen<PkgProgress>("pkg-download", (e) => {
    const { id, file, status, index, total, bytes, bytesTotal } = e.payload;
    const row = listEl.querySelector(`.pkg-row[data-id="${CSS.escape(id)}"]`) as HTMLElement | null;
    const prog = row?.querySelector(".pkg-progress") as HTMLElement | null;
    const fill = prog?.querySelector(".pkg-progress-fill") as HTMLElement | null;
    if (prog && fill) {
      prog.style.display = status === "done" && index === total ? "none" : "";
      const pct = bytesTotal > 0 ? Math.round((bytes / bytesTotal) * 100) : total ? Math.round((index / total) * 100) : 0;
      fill.style.width = `${Math.min(100, pct)}%`;
      fill.classList.toggle("err", status === "failed");
      fill.classList.toggle("ok", status === "done");
      prog.title = `${pct}% ${file}`;
    }
    if (status === "failed" || status === "cancelled") {
      const btn = row?.querySelector("button") as HTMLButtonElement | null;
      busy[id] = false;
      if (prog) prog.style.display = "none";
      if (btn) {
        btn.disabled = false;
        btn.textContent = "下载";
      }
      if (status === "cancelled") toast("已取消下载");
      if (status === "cancelled") void refresh(false);
    }
    if (status === "done" && index === total) {
      busy[id] = false;
      void invoke<PkgList>("pkg_list", { force: false }).then((r) => {
        installed = r.installed;
        renderInstalled();
        renderAvailable(r.available);
      });
      void invoke<MtStatus>("nmt_status").then(renderNmt);
    }
  });
  un.then((fn) => window.addEventListener("beforeunload", fn));
  void refresh(false);
}

// ---------------------------------------------------------------------------
// 翻译
// ---------------------------------------------------------------------------

async function doTranslate() {
  const text = inputArea.value.trim();
  if (!text) {
    toast("请输入要翻译的文本");
    return;
  }
  if (translating) return;
  translating = true;
  translateBtn.disabled = true;
  inputHint.textContent = "翻译中…";
  try {
    const out = await invoke<string>("translate_text", {
      text,
      source: sourceSel.value,
      target: targetSel.value,
      engine: currentSettings().f5Engine,
    });
    outputArea.value = out;
    memory.output = out;
    pushHistory(text, out);
  } catch (e) {
    toast(String(e), true);
  } finally {
    translating = false;
    translateBtn.disabled = false;
    inputHint.textContent = "";
  }
}

function pushHistory(text: string, translated: string) {
  history.unshift({
    ts: Math.floor(Date.now() / 1000),
    src: sourceSel.value,
    tgt: targetSel.value,
    text,
    translated,
  });
  if (!$<HTMLDivElement>("histList").querySelector(".hist-empty")) renderHistory();
}

function fmtTime(ts: number) {
  const d = new Date(ts * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function renderHistory() {
  const list = $<HTMLDivElement>("histList");
  list.textContent = "";
  if (currentSettings().historyLimit === 0) {
    const d = document.createElement("div");
    d.className = "hist-empty";
    d.textContent = "未开启历史记录（设置中可调整）";
    list.appendChild(d);
    return;
  }
  if (history.length === 0) {
    const d = document.createElement("div");
    d.className = "hist-empty";
    d.textContent = "暂无记录";
    list.appendChild(d);
    return;
  }
  for (const h of history) {
    const item = document.createElement("div");
    item.className = "hist-item";
    const meta = document.createElement("div");
    meta.className = "hist-meta";
    const langs = document.createElement("span");
    langs.textContent = `${langZh(h.src)} → ${langZh(h.tgt)}`;
    const time = document.createElement("span");
    time.textContent = fmtTime(h.ts);
    meta.append(langs, time);
    const src = document.createElement("div");
    src.className = "hist-src";
    src.textContent = h.text;
    src.title = h.text;
    const dst = document.createElement("div");
    dst.className = "hist-dst";
    dst.textContent = h.translated;
    dst.title = h.translated;
    item.append(meta, src, dst);
    item.addEventListener("click", () => {
      inputArea.value = h.text;
      sourceSel.value = h.src;
      targetSel.value = h.tgt;
      outputArea.value = h.translated;
    });
    list.appendChild(item);
  }
}

// ---------------------------------------------------------------------------
// 抽屉开关
// ---------------------------------------------------------------------------

function bindDrawers() {
  const settingsPanel = $<HTMLDivElement>("settingsPanel");
  const historyPanel = $<HTMLDivElement>("historyPanel");
  const settingsMask = $<HTMLDivElement>("settingsMask");
  const histMask = $<HTMLDivElement>("histMask");
  const open = (p: HTMLDivElement, m: HTMLDivElement) => {
    p.classList.remove("hidden");
    m.classList.remove("hidden");
  };
  const close = (p: HTMLDivElement, m: HTMLDivElement) => {
    p.classList.add("hidden");
    m.classList.add("hidden");
  };
  $<HTMLButtonElement>("settingsBtn").addEventListener("click", () => {
    close(historyPanel, histMask);
    open(settingsPanel, settingsMask);
  });
  $<HTMLButtonElement>("histBtn").addEventListener("click", () => {
    close(settingsPanel, settingsMask);
    renderHistory();
    open(historyPanel, histMask);
  });
  $<HTMLButtonElement>("histClear").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("histClear");
    btn.disabled = true;
    try {
      await invoke("history_clear");
      history = [];
      renderHistory();
      toast("历史已清空");
    } catch (e) {
      toast(String(e), true);
    } finally {
      btn.disabled = false;
    }
  });
  $<HTMLButtonElement>("settingsClose").addEventListener("click", () => close(settingsPanel, settingsMask));
  $<HTMLButtonElement>("histClose").addEventListener("click", () => close(historyPanel, histMask));
  settingsMask.addEventListener("click", () => close(settingsPanel, settingsMask));
  histMask.addEventListener("click", () => close(historyPanel, histMask));
}

// ---------------------------------------------------------------------------
// 初始化
// ---------------------------------------------------------------------------

async function init() {
  watchSystemAccent();
  fillSelect(
    sourceSel,
    LANGS,
    { code: "auto", zh: "自动检测" },
  );
  fillSelect(targetSel, LANGS);
  fillSelect($<HTMLSelectElement>("idleSelect"), IDLE_CHOICES.map((v) => ({ code: String(v), zh: v === 0 ? "永久常驻" : `${v} 秒` })));
  fillSelect(
    $<HTMLSelectElement>("historySelect"),
    HISTORY_CHOICES.map((v) => ({ code: String(v), zh: v === 0 ? "不保留" : `${v} 条` })),
  );
  fillSelect($<HTMLSelectElement>("ocrSelect"), OCR_LANGS.slice(1).map((l) => l), OCR_LANGS[0]);
  fillSelect($<HTMLSelectElement>("preferredLangSelect"), LANGS);
  fillSelect($<HTMLSelectElement>("secondaryLangSelect"), LANGS);
  for (const id of ["engineOrder0", "engineOrder1", "engineOrder2", "f5EngineSelect"]) {
    fillEngineOptions($<HTMLSelectElement>(id));
  }

  settings = await invoke<Settings>("settings_get");
  if (!settings.hotkeys) {
    // 低版本配置兜底：热键缺失时补齐默认值
    settings.hotkeys = { f1: "F1", f2: "F2", f3: "F3", f4: "F4", f5: "F5" };
  }
  applySettingsToUi();

  inputArea.value = memory.input || "";
  outputArea.value = settings.modelPath ? memory.output : "";
  sourceSel.value = memory.src || settings.defaultSource;
  targetSel.value = memory.tgt || settings.defaultTarget;
  memory.src = sourceSel.value;
  memory.tgt = targetSel.value;

  history = await invoke<HistoryItem[]>("history_get");

  $<HTMLSpanElement>("version").textContent = `v${await invoke<string>("app_version")}`;
  $<HTMLSpanElement>("aboutVersion").textContent = await invoke<string>("app_version");
  $<HTMLSpanElement>("updateCurrentVer").textContent = await invoke<string>("app_version");

  bindSettingsControls();
  bindModelControls();
  bindNmtControls();
  bindPkgManager();
  bindDrawers();
  bindUpdateControls();

  inputArea.addEventListener("input", () => (memory.input = inputArea.value));
  inputArea.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void doTranslate();
    }
  });
  translateBtn.addEventListener("click", () => void doTranslate());
  $<HTMLButtonElement>("swapBtn").addEventListener("click", () => {
    const s = sourceSel.value;
    const t = targetSel.value;
    if (t !== "auto") {
      sourceSel.value = t;
      targetSel.value = s === "auto" ? "zh" : s;
    }
    memory.src = sourceSel.value;
    memory.tgt = targetSel.value;
    void pushSettings({ defaultSource: sourceSel.value, defaultTarget: targetSel.value });
  });
  $<HTMLButtonElement>("copyOutBtn").addEventListener("click", async () => {
    const text = outputArea.value.trim();
    if (!text) return;
    try {
      await invoke("copy_text", { text });
      toast("已复制");
    } catch (e) {
      toast(String(e), true);
    }
  });

  try {
    await invoke<ModelStatus>("model_status").then(renderModelStatus);
  } catch {
    /* 状态面板保持初始 */
  }

  try {
    await invoke<MtStatus>("nmt_status").then(renderNmt);
  } catch {
    /* 保持初始 */
  }

  const un1: UnlistenFn = await listen<ModelStatus>("model-status", (e) => renderModelStatus(e.payload));
  const un5: UnlistenFn = await listen<MtStatus>("nmt-status", (e) => renderNmt(e.payload));
  const un2: UnlistenFn = await listen<Settings>("settings-changed", (e) => {
    settings = e.payload;
    applySettingsToUi();
  });
  const un3: UnlistenFn = await listen<UiStatus>("ui-status", (e) => renderGame(e.payload));
  const unUpd: UnlistenFn = await listen<UpdateInfo>("update-available", (e) => {
    showUpdateFab(e.payload);
    if (e.payload.hasUpdate) {
      updateStatusLine(`发现新版本 v${e.payload.latest}，点击右下角圆形按钮查看`, true);
    }
  });
  const un4: UnlistenFn = await listen("history-changed", () => {
    void invoke<HistoryItem[]>("history_get").then((h) => {
      history = h;
      if (!$<HTMLDivElement>("historyPanel").classList.contains("hidden")) renderHistory();
    });
  });
  window.addEventListener("beforeunload", () => {
    un1();
    un2();
    un3();
    un4();
    un5();
    unUpd();
  });
}

void init().catch((e) => toast(String(e), true));