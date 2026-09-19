// Linggo 弹窗窗口（步骤 9）：F1 原文/译文视图、F2 输入+回车回贴、F3 OCR 结果、加载/警告视图。

import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { applySystemAccent, watchSystemAccent, initTheme } from "./shared";

interface PopupPayload {
  mode: string;
  message?: string;
  source?: string;
  target?: string;
  text?: string;
  translated?: string;
  cropDataUrl?: string;
  base?: string;
}

const LANGS = [
  "en", "zh", "ja", "ko", "fr", "de", "es", "it", "pt", "ru", "ar", "hi",
  "vi", "th", "id", "ms", "tr", "nl", "pl", "uk", "sv", "da", "fi", "no",
  "cs", "hu", "ro", "bg", "hr", "sk", "sl", "he", "el",
];

function $<T extends Element>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error("missing element: " + id);
  return el as unknown as T;
}

const win = getCurrentWindow();
const views = ["loadingView", "warnView", "translationView", "inputView", "ocrView", "folderView"];

let ringTimer: ReturnType<typeof setInterval> | undefined;
let remainMs = 0;
let ringTotal = 0;
let paused = false;
let cropDataUrl = "";
let debounce: ReturnType<typeof setTimeout> | undefined;

// 翻译进行中标志与请求序号：防止「翻译中…」占位被回贴/建夹，也防旧请求覆盖新结果
let liveBusy = false;
let liveSeq = 0;
let folderBusy = false;
let folderSeq = 0;

// F4 文件夹视图状态
let folderBase = "";
let folderTarget = "auto";
let folderTranslated = "";

// 环 r=9 → 周长 2πr≈56.55；JS 每 100ms 拨动 stroke-dashoffset。
const RING_C = 56.55;

function showView(name: string, height: number) {
  for (const v of views) $<HTMLElement>(v).classList.add("hidden");
  $<HTMLElement>(name).classList.remove("hidden");
  void win.setSize(new LogicalSize(430, height)).then(() => void invoke("popup_reposition"));
}

function toast(msg: string) {
  $<HTMLSpanElement>("warnMsg").textContent = msg;
}

function stopCountdown() {
  if (ringTimer) clearInterval(ringTimer);
  ringTimer = undefined;
  $<HTMLDivElement>("ringWrap").classList.remove("armed");
}

function armAutoClose(secs: number) {
  stopCountdown();
  remainMs = secs * 1000;
  ringTotal = remainMs;
  $<SVGCircleElement>("ringFg").style.strokeDashoffset = "0";
  $<HTMLDivElement>("ringWrap").classList.add("armed");
  ringTimer = setInterval(() => {
    if (paused) return; // 悬停暂停，继续阅读
    remainMs -= 100;
    $<SVGCircleElement>("ringFg").style.strokeDashoffset = String(
      RING_C * (1 - Math.max(0, remainMs) / ringTotal),
    );
    if (remainMs <= 0) {
      stopCountdown();
      void closePop();
    }
  }, 100);
}

function cancelAutoClose() {
  stopCountdown();
}

async function closePop() {
  stopCountdown();
  // 先把焦点还给划词/回贴来源窗口，再隐藏弹窗；
  // 否则前台会落到 Linggo 主窗口，下次 F1 划词会把 Linggo 自己当复制来源 → 未捕获到选中文本。
  try {
    await invoke("restore_target_focus");
  } catch {
    /* 无源窗口时忽略 */
  }
  await win.hide();
}

// ---------------------------------------------------------------------------
// 视图渲染
// ---------------------------------------------------------------------------

function renderLoading(message: string) {
  $<HTMLSpanElement>("loadingMsg").textContent = message || "处理中…";
  showView("loadingView", 120);
}

function renderWarning(message: string) {
  $<HTMLSpanElement>("warnMsg").textContent = message;
  showView("warnView", 120);
  armAutoClose(message.length > 60 ? 8 : 6);
}

function renderTranslation(p: PopupPayload) {
  $<HTMLDivElement>("trSrc").textContent = p.text ?? "";
  $<HTMLDivElement>("trDst").textContent = p.translated ?? "";
  showView("translationView", 330);
  armAutoClose(14);
}

async function renderInput() {
  const sel = $<HTMLSelectElement>("inTarget");
  if (sel.options.length === 0) {
    const autoOpt = document.createElement("option");
    autoOpt.value = "auto";
    autoOpt.textContent = "自动（首选/次选）";
    sel.appendChild(autoOpt);
    for (const code of LANGS) {
      const opt = document.createElement("option");
      opt.value = code;
      opt.textContent = code.toUpperCase();
      sel.appendChild(opt);
    }
    sel.value = "auto";
  }
  $<HTMLInputElement>("inText").value = "";
  $<HTMLDivElement>("inResult").textContent = "";
  liveBusy = false;
  liveSeq = 0;
  showView("inputView", 210);
  // 首次 F2 开窗时窗口尚未 show，直接 focus 会被随后的 show/setFocus 抢走；
  // 统一由 onPopupOpen 在窗口显示后再触发聚焦（见下方 focusInputAfterShown）。
}

function renderOcr(p: PopupPayload) {
  cropDataUrl = p.cropDataUrl ?? "";
  $<HTMLDivElement>("ocrSrc").textContent = p.text ?? "";
  $<HTMLDivElement>("ocrDst").textContent = p.translated ?? "";
  $<HTMLButtonElement>("ocrCopyImgBtn").disabled = !cropDataUrl;
  $<HTMLButtonElement>("ocrPinBtn").disabled = !cropDataUrl;
  showView("ocrView", 380);
  armAutoClose(16);
}

// ---------------------------------------------------------------------------
// F4 新建文件夹视图：输入即译（复用 translate_text）+ 回车创建
// ---------------------------------------------------------------------------

async function renderFolder(p: PopupPayload) {
  folderBase = p.base ?? "";
  folderTarget = "auto";
  folderTranslated = "";
  folderBusy = false;
  folderSeq = 0;
  $<HTMLInputElement>("inFolderText").value = "";
  $<HTMLDivElement>("folderPreview").textContent = "";
  const baseEl = $<HTMLSpanElement>("folderBase");
  baseEl.textContent = folderBase;
  baseEl.title = folderBase;
  showView("folderView", 230);
}

async function liveFolderTranslate() {
  const text = $<HTMLInputElement>("inFolderText").value.trim();
  const mySeq = ++folderSeq;
  if (!text) {
    $<HTMLDivElement>("folderPreview").textContent = "";
    folderTranslated = "";
    folderBusy = false;
    return;
  }
  $<HTMLDivElement>("folderPreview").textContent = "翻译中…";
  folderBusy = true;
  try {
    const r = await invoke<string>("translate_text", { text, source: "auto", target: folderTarget });
    // 过期响应丢弃：输入变化后旧结果不应覆盖新文本的译文
    if (folderSeq !== mySeq) return;
    $<HTMLDivElement>("folderPreview").textContent = r;
    folderTranslated = r;
    folderBusy = false;
  } catch (e) {
    if (folderSeq !== mySeq) return;
    $<HTMLDivElement>("folderPreview").textContent = String(e);
    folderTranslated = "";
    folderBusy = false;
  }
}

async function doCreateFolder() {
  const raw = $<HTMLInputElement>("inFolderText").value.trim();
  if (!raw) {
    toast("请输入文件夹名字");
    return;
  }
  if (folderBusy) {
    toast("翻译中，请稍候再创建");
    return;
  }
  const name = (folderTranslated || raw).trim();
  try {
    await invoke<string>("create_folder", { base: folderBase, name });
  } catch (e) {
    $<HTMLDivElement>("folderPreview").textContent = String(e);
    toast(String(e));
    return;
  }
  // 原生体感：创建成功后立即隐藏弹窗，不等焦点归还（后者后台异步进行）。
  hideFolderAndRestore();
}

/// Esc：不翻译，直接用原文名字创建文件夹
async function doCreateFolderRaw() {
  const raw = $<HTMLInputElement>("inFolderText").value.trim();
  if (!raw) {
    // 未输入名字：等同取消本次创建
    cancelFolderCreate();
    return;
  }
  try {
    await invoke<string>("create_folder", { base: folderBase, name: raw });
  } catch (e) {
    $<HTMLDivElement>("folderPreview").textContent = String(e);
    toast(String(e));
    return;
  }
  hideFolderAndRestore();
}

/// Delete：取消本次创建并关闭 F4 弹窗
function cancelFolderCreate() {
  folderSeq++; // 作废进行中的翻译响应
  folderTranslated = "";
  folderBusy = false;
  void closePop();
}

function hideFolderAndRestore() {
  stopCountdown();
  void win.hide();
  void invoke("restore_target_focus").catch(() => {});
}

// ---------------------------------------------------------------------------
// 输入即译（450ms 防抖）
// ---------------------------------------------------------------------------

async function liveTranslate() {
  const text = $<HTMLInputElement>("inText").value.trim();
  const target = $<HTMLSelectElement>("inTarget").value;
  const mySeq = ++liveSeq;
  if (!text) {
    $<HTMLDivElement>("inResult").textContent = "";
    liveBusy = false;
    return;
  }
  $<HTMLDivElement>("inResult").textContent = "翻译中…";
  liveBusy = true;
  try {
    const r = await invoke<string>("translate_text", { text, source: "auto", target });
    // 过期响应丢弃：输入变化后旧结果不应覆盖新文本的译文
    if (liveSeq !== mySeq) return;
    $<HTMLDivElement>("inResult").textContent = r;
    liveBusy = false;
  } catch (e) {
    if (liveSeq !== mySeq) return;
    $<HTMLDivElement>("inResult").textContent = String(e);
    liveBusy = false;
  }
}

// ---------------------------------------------------------------------------
// 事件订阅
// ---------------------------------------------------------------------------

async function onPopupOpen(p: PopupPayload) {
  cancelAutoClose();
  switch (p.mode) {
    case "loading":
      renderLoading(p.message ?? "");
      break;
    case "warning":
      renderWarning(p.message ?? "出错了");
      break;
    case "translation":
      renderTranslation(p);
      break;
    case "input":
      await renderInput();
      break;
    case "folder":
      await renderFolder(p);
      break;
    case "ocr":
      renderOcr(p);
      break;
    default:
      renderLoading("未知请求");
  }
  await win.show();
  await win.setFocus();
  // 首次开窗 WebView2 attach 可能把窗口挪到默认位置，显示后再对位一次
  void invoke("popup_reposition");
  // 窗口真正显示/聚焦后再把键盘焦点给输入框（首次开窗必做，否则焦点落在窗口本体上）
  if (p.mode === "input") focusAfterShown("inText");
  else if (p.mode === "folder") focusAfterShown("inFolderText");
}

let _focusTimer: number | undefined;
function focusAfterShown(id: string) {
  if (_focusTimer !== undefined) window.clearTimeout(_focusTimer);
  _focusTimer = window.setTimeout(() => {
    $<HTMLInputElement>(id).focus();
    _focusTimer = undefined;
  }, 60);
}

// ---------------------------------------------------------------------------
// 初始化
// ---------------------------------------------------------------------------

async function init() {
  // 主题跟随设置（浅色/深色/跟随系统），并监听设置变更
  void initTheme();
  // 用 Windows「个性化」强调色点缀环形倒计时（读失败用主题 --accent 兜底）
  watchSystemAccent();
  const accent = await applySystemAccent();
  if (accent) $<SVGCircleElement>("ringFg").style.stroke = accent;

  // 挂事件前先捕获几次由模型加载引起的 popup-open 之前的状态
  const un1: UnlistenFn = await listen<PopupPayload>("popup-open", (e) => void onPopupOpen(e.payload));
  const un2: UnlistenFn = await listen<{ message: string }>("warning", (e) => {
    cancelAutoClose();
    renderWarning(e.payload.message);
    void win.show().then(() => win.setFocus());
  });

  $<HTMLButtonElement>("closeBtn").addEventListener("click", () => void closePop());
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") void closePop();
  });

  // 悬停暂停倒计时（不重置进度，离开后从剩余时间续走）
  document.addEventListener("mouseenter", () => {
    paused = true;
  });
  document.addEventListener("mouseleave", () => {
    paused = false;
  });

  // F1 翻译视图
  $<HTMLButtonElement>("copyOrigBtn").addEventListener("click", async () => {
    await invoke("copy_text", { text: $<HTMLDivElement>("trSrc").textContent });
  });
  $<HTMLButtonElement>("copyTransBtn").addEventListener("click", async () => {
    await invoke("copy_text", { text: $<HTMLDivElement>("trDst").textContent });
  });

  // F2 输入视图
  const inText = $<HTMLInputElement>("inText");
  inText.addEventListener("input", () => {
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(() => void liveTranslate(), 450);
  });
  inText.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      void doPaste();
    }
  });
  $<HTMLSelectElement>("inTarget").addEventListener("change", () => void liveTranslate());
  $<HTMLButtonElement>("inPasteBtn").addEventListener("click", () => void doPaste());
  $<HTMLButtonElement>("inCopyBtn").addEventListener("click", async () => {
    await invoke("copy_text", { text: $<HTMLDivElement>("inResult").textContent });
  });

  // F4 文件夹视图
  const inFolderText = $<HTMLInputElement>("inFolderText");
  inFolderText.addEventListener("input", () => {
    folderTranslated = "";
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(() => void liveFolderTranslate(), 450);
  });
  inFolderText.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      void doCreateFolder();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      void doCreateFolderRaw();
    } else if (e.key === "Delete") {
      e.preventDefault();
      e.stopPropagation();
      cancelFolderCreate();
    }
  });
  $<HTMLButtonElement>("folderCreateBtn").addEventListener("click", () => void doCreateFolder());

  // F3 OCR 视图
  $<HTMLButtonElement>("ocrCopyTextBtn").addEventListener("click", async () => {
    await invoke("copy_text", { text: $<HTMLDivElement>("ocrDst").textContent });
  });
  $<HTMLButtonElement>("ocrCopyImgBtn").addEventListener("click", async () => {
    if (!cropDataUrl) return;
    try {
      await invoke("copy_image", { b64: cropDataUrl });
      toast("已复制截图");
    } catch (e) {
      toast(String(e));
    }
  });
  $<HTMLButtonElement>("ocrPinBtn").addEventListener("click", async () => {
    if (!cropDataUrl) return;
    await emitTo("pin", "pin-open", { dataUrl: cropDataUrl });
    await closePop();
  });

  window.addEventListener("beforeunload", () => {
    un1();
    un2();
  });
}

async function doPaste() {
  const text = $<HTMLDivElement>("inResult").textContent ?? "";
  // 翻译进行中：不把「翻译中…」占位文本贴进目标窗口
  if (liveBusy) {
    toast("还在翻译中，请稍候回车回贴");
    return;
  }
  if (!text.trim()) {
    toast("还没有译文可回贴");
    return;
  }
  try {
    await invoke("commit_paste", { text });
    await closePop();
  } catch (e) {
    toast(String(e));
  }
}

void init().catch((e) => {
  toast(String(e));
});