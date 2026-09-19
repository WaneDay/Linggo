// Linggo 前端共享类型（多窗口事件负载；与 Rust serde camelCase 对应）

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** 把主题（auto/light/dark）写入 html[data-theme]；auto 删除属性回到「跟随系统」 */
export function applyTheme(theme: string): void {
  const root = document.documentElement;
  if (theme === "auto") delete root.dataset.theme;
  else root.dataset.theme = theme;
}

/**
 * 所有窗口统一主题入口：读当前设置并应用主题，同时监听 settings-changed 跟随主窗切换。
 * popup/snip/pin 等窗口在 init 时调用一次即可（窗口关闭即销毁，无需手动取消监听）。
 */
export async function initTheme(): Promise<void> {
  try {
    const s = await invoke<{ theme?: string }>("settings_get");
    applyTheme(s.theme || "auto");
  } catch {
    applyTheme("auto");
  }
  void listen<{ theme?: string }>("settings-changed", (e) => {
    applyTheme(e.payload.theme || "auto");
  }).catch(() => undefined);
}

/** F3 截图投递给 snip 窗口（透明压盖铺满虚拟屏幕） */
export interface SnipOpenPayload {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** OCR 行框/段落框（裁剪图坐标）：{x,y,w,h,text,translated?,n?}；n=段落包含的源文本行数 */
export interface PinLine {
  text: string;
  x: number;
  y: number;
  w: number;
  h: number;
  translated?: string;
  n?: number;
}

/** 贴图置顶窗口数据（F3 完整版：原图 + 段落框 + 译文 + 原位置） */
export interface PinOpenPayload {
  dataUrl: string;
  /** OCR 段落框（裁剪图坐标） */
  lines?: PinLine[];
  /** 默认显示覆盖图（true=贴图默认翻译覆盖，设置可改） */
  overlay?: boolean;
  /** 贴图摆放位置（虚拟桌面物理坐标）：true=贴图在原截图位置，否则默认居中 */
  place?: { x: number; y: number; w: number; h: number };
}

/** OCR 单行文本行框（裁剪图坐标）：贴图内文字拖选/高亮的最小单元，每行一个半透明矩形 */
export interface PinTextRow {
  text: string;
  x: number;
  y: number;
  w: number;
  h: number;
  /** 在源 OCR 行序列里的序号（复制时按阅读顺序拼接） */
  li: number;
}

/** 贴图增量更新（翻译完成后把段落译文覆盖到已打开的贴图上） */
export interface PinUpdatePayload {
  lines: PinLine[];
  overlay: boolean;
  /** 行级框（拖选高亮用） */
  rows?: PinTextRow[];
}

/** 弹窗打开负载（F1/F2/F3/loading/warning） */
export interface PopupPayload {
  mode: string;
  message?: string;
  source?: string;
  target?: string;
  text?: string;
  translated?: string;
  cropDataUrl?: string;
  base?: string;
}

/** 设置（前端只读需要的最小字段） */
export interface SettingsView {
  theme: string;
  hotkeys: { f1: string; f2: string; f3: string; f4: string; f5: string };
  idleTimeoutSecs: number;
  defaultSource: string;
  defaultTarget: string;
  modelPath: string;
  ocrLang: string;
  gameModeBlock: boolean;
  historyLimit: number;
  autostart: boolean;
  pinDefaultOverlay: boolean;
  /** 贴图时是否显示快捷键提示 */
  pinShowTip: boolean;
  engineOrder?: string[];
  f5Engine?: string;
  /** 启动时自动检查新版本 */
  checkUpdatesEnabled?: boolean;
  /** F1–F4 首选语言 */
  preferredLang?: string;
  /** F1–F4 次选语言 */
  secondaryLang?: string;
}

/** 模型状态（model-status 事件 / model_status 命令） */
export interface ModelStatusView {
  state: string;
  path: string;
  error: string | null;
}

// ---------------------------------------------------------------------------
// 系统强调色：让各窗口的 --accent 跟随 Windows「个性化 → 颜色」的强调色
// ---------------------------------------------------------------------------

/** 依据强调色亮度选择可读前景（深色强调→白字，浅色强调→黑字） */
export function contrastText(hex: string): string {
  const m = /^#?([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!m) return "#ffffff";
  const n = parseInt(m[1], 16);
  const lin = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  const lum =
    0.2126 * lin((n >> 16) & 255) +
    0.7152 * lin((n >> 8) & 255) +
    0.0722 * lin(n & 255);
  return lum > 0.5 ? "#000000" : "#ffffff";
}

/**
 * 读取 Windows 个性化强调色并写入 `--accent` / `--accent-fg`。
 * 返回 `#RRGGBB`；读取失败返回 null（保持主题默认蓝色）。
 */
export async function applySystemAccent(): Promise<string | null> {
  try {
    const hex = await invoke<string | null>("system_accent_color");
    if (!hex) return null;
    const root = document.documentElement;
    root.style.setProperty("--accent", hex);
    root.style.setProperty("--accent-fg", contrastText(hex));
    return hex;
  } catch {
    return null;
  }
}

/** 系统改色后无需重启：窗口每次获得焦点时重新读取强调色 */
export function watchSystemAccent(): void {
  window.addEventListener("focus", () => void applySystemAccent());
}
