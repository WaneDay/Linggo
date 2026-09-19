// Linggo 截图框选窗口（透明压盖模式）：
// - 不冻结桌面、不显示截图：transparent 窗口 + 画布淡灰遮罩，实时桌面透出。
// - 移动鼠标自动识别鼠标所在「顶层窗口矩形」（windows_at，参考 ShareX）→ 强调色实线预选框。
// - 左键单击：直接选中预选框；左键拖动：自定义矩形；右键 / Esc：退出。
// - 回车：先贴图到原位置（Snipaste 式发光阴影）→ 退出截图 → 逐行翻译覆盖到贴图。

import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, PhysicalPosition, PhysicalSize } from "@tauri-apps/api/window";
import type { SnipOpenPayload } from "./shared";
import { applySystemAccent, watchSystemAccent, initTheme } from "./shared";

const win = getCurrentWindow();
const canvas = document.getElementById("snipCanvas") as HTMLCanvasElement;
const ctx = canvas.getContext("2d")!;
const hint = document.getElementById("snipHint") as HTMLDivElement;

const HINT_TEXT = "移动预选框 · 左键拖动框选 / 单击选中窗口 / 再单击扩角 · 回车贴图翻译 · 右键或 Esc 退出";

// 虚拟屏幕物理原点（负数多屏也支持）
let winLeft = 0;
let winTop = 0;
let winW = 0;
let winH = 0;

function dpr() { return window.devicePixelRatio || 1; }

function resize() {
  const d = dpr();
  canvas.width = Math.round(canvas.clientWidth * d);
  canvas.height = Math.round(canvas.clientHeight * d);
  ctx.setTransform(d, 0, 0, d, 0, 0);
}

// 物理屏幕坐标（绝对，windows_at 用）
function toAbs(e: MouseEvent) {
  const d = dpr();
  return {
    x: Math.round(winLeft + e.clientX * d),
    y: Math.round(winTop + e.clientY * d),
  };
}

// 物理屏幕坐标（相对虚拟屏幕原点，crop/ocr 用）
function toRel(e: MouseEvent) {
  const d = dpr();
  return {
    x: Math.max(0, Math.round(e.clientX * d)),
    y: Math.max(0, Math.round(e.clientY * d)),
  };
}

// 窗口矩形（绝对物理坐标）→ Canvas CSS 坐标
function absToCssRect(r: { left: number; top: number; right: number; bottom: number } | null) {
  if (!r) return null;
  const d = dpr();
  return {
    x: (r.left - winLeft) / d,
    y: (r.top - winTop) / d,
    w: (r.right - r.left) / d,
    h: (r.bottom - r.top) / d,
  };
}

// 状态
let accent = "#0067c0";
let hoverBox: { left: number; top: number; right: number; bottom: number } | null = null;
let downRel: { x: number; y: number } | null = null;
let curRel: { x: number; y: number } | null = null;
let pendingHover: { left: number; top: number; right: number; bottom: number } | null = null;
let sel: { x: number; y: number; w: number; h: number } | null = null;
let dragging = false;
let busy = false;
let detecting = false;
let detectDirty = false;

function normRect(a: { x: number; y: number }, b: { x: number; y: number }) {
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  const w = Math.abs(a.x - b.x);
  const h = Math.abs(a.y - b.y);
  return { x, y, w, h };
}

// 单击扩容：把选区距离点击点最近的角移动到点击点（其余三角固定）
// 仅当点击点在选区外时生效（实现「扩大」语义，且不影响双击回车确认）
function expandNearestCorner(p: { x: number; y: number }) {
  if (!sel) return;
  const inside =
    p.x >= sel.x && p.x <= sel.x + sel.w &&
    p.y >= sel.y && p.y <= sel.y + sel.h;
  if (inside) return;
  const corners = [
    { x: sel.x, y: sel.y },
    { x: sel.x + sel.w, y: sel.y },
    { x: sel.x, y: sel.y + sel.h },
    { x: sel.x + sel.w, y: sel.y + sel.h },
  ];
  let best = 0;
  let bd = Infinity;
  for (let i = 0; i < corners.length; i++) {
    const dx = corners[i].x - p.x;
    const dy = corners[i].y - p.y;
    const d = dx * dx + dy * dy;
    if (d < bd) { bd = d; best = i; }
  }
  const xs = corners.map((c, i) => (i === best ? p.x : c.x));
  const ys = corners.map((c, i) => (i === best ? p.y : c.y));
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  const w = Math.max(...xs) - x;
  const h = Math.max(...ys) - y;
  if (w >= 4 && h >= 4) sel = { x, y, w, h };
}

function draw() {
  const cw = canvas.clientWidth;
  const ch = canvas.clientHeight;
  ctx.clearRect(0, 0, cw, ch);
  // 淡灰遮罩（透明窗口，透出实时桌面）
  ctx.fillStyle = "rgba(60,60,60,0.35)";
  ctx.fillRect(0, 0, cw, ch);

  const box = dragging && downRel && curRel ? absToCssRect(absOf(normRect(downRel, curRel))) : null;
  const cur = box ?? (sel ? absToCssRect(absOf(sel)) : null);

  let display = cur;
  const showHover = !dragging && !sel && hoverBox;
  if (showHover) display = absToCssRect(hoverBox);

  if (display) {
    // 选中区域擦除遮罩（露出实时桌面）
    ctx.clearRect(display.x, display.y, display.w, display.h);
    // 实线描边预选框（强调色）
    ctx.strokeStyle = accent;
    ctx.lineWidth = 2;
    ctx.strokeRect(display.x + 1, display.y + 1, display.w - 2, display.h - 2);
  }

  // 尺寸提示（拖动中或已选中都实时显示，物理像素）
  const sizeSrc = dragging && downRel && curRel ? normRect(downRel, curRel) : sel;
  const sizeCss = sizeSrc ? absToCssRect(absOf(sizeSrc)) : null;
  if (sizeSrc && sizeCss && sizeSrc.w > 0 && sizeSrc.h > 0) {
    const x = Math.max(sizeCss.x, 2);
    const y = Math.max(sizeCss.y - 26, 2);
    const label = `${sizeSrc.w} × ${sizeSrc.h}`;
    ctx.font = "12px Segoe UI";
    ctx.fillStyle = "rgba(0,0,0,0.65)";
    const tw = ctx.measureText(label).width;
    ctx.fillRect(x, y, tw + 14, 22);
    ctx.fillStyle = "#fff";
    ctx.fillText(label, x + 7, y + 15);
  }
}

function absOf(r: { x: number; y: number; w: number; h: number }) {
  return { left: winLeft + r.x, top: winTop + r.y, right: winLeft + r.x + r.w, bottom: winTop + r.y + r.h };
}

// 探测鼠标所在窗口（节流：单飞 + 脏标记）
function detectAtAbs(x: number, y: number) {
  detectDirty = false;
  if (detecting) {
    detectDirty = true;
    return;
  }
  detecting = true;
  void (async () => {
    const r = await invoke<{ left: number; top: number; right: number; bottom: number } | null>("windows_at", { x, y });
    if (!dragging) hoverBox = r;
    detecting = false;
    if (detectDirty) {
      const pos = cursorAbs;
      if (pos) detectAtAbs(pos.x, pos.y);
    }
    draw();
  })().catch(() => {
    detecting = false;
  });
}

let cursorAbs: { x: number; y: number } | null = null;
let mouseMoveTimer: ReturnType<typeof setTimeout> | undefined;

async function onSnipOpen(p: SnipOpenPayload) {
  busy = false;
  winLeft = p.left;
  winTop = p.top;
  winW = p.width;
  winH = p.height;
  sel = null;
  downRel = null;
  curRel = null;
  hoverBox = null;
  try {
    await win.setPosition(new PhysicalPosition(p.left, p.top));
    await win.setSize(new PhysicalSize(p.width, p.height));
  } catch { /* 尺寸异常不影响 */ }
  await win.show();
  await win.setFocus();
  resize();
  draw();
  hint.textContent = HINT_TEXT;
}

// ---------------------------------------------------------------------------
// 交互
// ---------------------------------------------------------------------------

canvas.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  // 左键一律先进入「待拖动」状态：移动超阈值即自定义框选，不移动则视为单击
  downRel = toRel(e);
  curRel = downRel;
  pendingHover = hoverBox;
  dragging = false;
});

canvas.addEventListener("mousemove", (e) => {
  cursorAbs = toAbs(e);
  if (downRel) {
    const r = toRel(e);
    if (!dragging && (Math.abs(r.x - downRel.x) > 3 || Math.abs(r.y - downRel.y) > 3)) {
      dragging = true;
      pendingHover = null;
    }
    if (dragging) {
      curRel = r;
      hoverBox = null;
      draw();
    }
    return;
  }
  // 非拖拽时节流探测（防高频 invoke）
  if (mouseMoveTimer) clearTimeout(mouseMoveTimer);
  mouseMoveTimer = setTimeout(() => detectAtAbs(cursorAbs!.x, cursorAbs!.y), 25);
});

window.addEventListener("mouseup", (e) => {
  if (e.button !== 0) return;
  if (dragging && downRel && curRel) {
    const r = normRect(downRel, curRel);
    sel = r.w > 2 && r.h > 2 ? r : null;
  } else if (downRel && !dragging) {
    // 单击：已选区 → 最近的角扩到点击点；无选区 → 选中悬停窗口矩形
    if (sel) {
      expandNearestCorner(downRel);
    } else if (pendingHover) {
      sel = {
        x: pendingHover.left - winLeft,
        y: pendingHover.top - winTop,
        w: pendingHover.right - pendingHover.left,
        h: pendingHover.bottom - pendingHover.top,
      };
    }
  }
  downRel = null;
  curRel = null;
  dragging = false;
  pendingHover = null;
  draw();
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") void win.hide();
  else if (e.key === "Enter") void confirmSel();
});
canvas.addEventListener("dblclick", () => void confirmSel());
canvas.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  void win.hide();
});

// ---------------------------------------------------------------------------
// 确认：先贴图原图（原位置+发光阴影）→ 退出截图 → 分组翻译覆盖
// ---------------------------------------------------------------------------

type OcrLineBox = { text: string; x: number; y: number; w: number; h: number };

// 把相邻同栏的 OCR 行合并成「段落」，便于整段翻译 + 更大字号换行排版
function groupParagraphs(src: OcrLineBox[]): (OcrLineBox & { n: number })[] {
  const lines = src
    .filter((l) => l.text.trim() && l.w > 0 && l.h > 0)
    .slice()
    .sort((a, b) => a.y - b.y || a.x - b.x);
  type Group = OcrLineBox & { bottom: number; lastH: number; n: number };
  const groups: Group[] = [];
  for (const l of lines) {
    const g = groups[groups.length - 1];
    let sameBlock = false;
    if (g) {
      const gap = l.y - g.bottom;
      const hRef = g.lastH;
      const overlap = Math.min(g.x + g.w, l.x + l.w) - Math.max(g.x, l.x);
      const minW = Math.min(g.w, l.w);
      const ratio = l.h / g.lastH;
      sameBlock =
        gap <= hRef * 0.9 && gap >= -hRef * 0.6 &&
        overlap > Math.min(minW * 0.3, 24) &&
        ratio > 0.5 && ratio < 2;
    }
    if (g && sameBlock) {
      g.text = `${g.text} ${l.text}`;
      const x1 = Math.max(g.x + g.w, l.x + l.w);
      const y1 = Math.max(g.bottom, l.y + l.h);
      g.x = Math.min(g.x, l.x);
      g.y = Math.min(g.y, l.y);
      g.w = x1 - g.x;
      g.h = y1 - g.y;
      g.bottom = y1;
      g.lastH = l.h;
      g.n += 1;
    } else {
      groups.push({ text: l.text, x: l.x, y: l.y, w: l.w, h: l.h, bottom: l.y + l.h, lastH: l.h, n: 1 });
    }
  }
  return groups.map((g) => ({ text: g.text, x: g.x, y: g.y, w: g.w, h: g.h, n: g.n }));
}

async function confirmSel() {
  if (!sel || busy) return;
  busy = true;
  const { x, y, w, h } = sel;
  try {
    const crop = await invoke<string>("snip_crop_png", {
      x: Math.round(x),
      y: Math.round(y),
      w: Math.round(w),
      h: Math.round(h),
    });
    // 1) 立即在原位置贴图（Snipaste 式：无边框 + 强调色发光阴影），先显示原图
    await emitTo("pin", "pin-open", {
      dataUrl: crop,
      overlay: false,
      lines: [],
      place: { x: winLeft + x, y: winTop + y, w, h },
    });
    // 2) 先退出截图，再翻译
    await win.hide();

    const ocr = await invoke<{
      text: string;
      lines: { text: string; x: number; y: number; w: number; h: number }[];
    }>("ocr_snip", {
      x: Math.round(x),
      y: Math.round(y),
      w: Math.round(w),
      h: Math.round(h),
    });
    const rows = (ocr.lines ?? []).map((l, li) => ({
      text: l.text.trim(),
      x: l.x,
      y: l.y,
      w: l.w,
      h: l.h,
      li,
    }));
    // 合并成段落后再整段翻译（避免逐行碎片化导致字号被压得过小）
    const groups = groupParagraphs(ocr.lines ?? []);
    const texts = groups.map((g) => g.text);
    const settings = await invoke<{ pinDefaultOverlay: boolean }>("settings_get");
    let translated: string[] = [];
    if (texts.length > 0) {
      translated = await invoke<string[]>("translate_lines", {
        texts,
        source: "auto",
        target: "auto",
      });
    }
    // 3) 译文覆盖绘制到贴图（按段落框换行排版）
    const lines = groups.map((g, i) => ({
      text: g.text,
      x: g.x,
      y: g.y,
      w: g.w,
      h: g.h,
      n: g.n,
      translated: translated[i] ?? "",
    }));
    await emitTo("pin", "pin-update", {
      lines,
      overlay: settings.pinDefaultOverlay !== false,
      rows,
    });
  } catch (err) {
    busy = false;
    hint.textContent = HINT_TEXT;
    await win.hide();
    await emitTo("popup", "warning", { message: String(err) });
  }
}

// ---------------------------------------------------------------------------
// 初始化
// ---------------------------------------------------------------------------

async function init() {
  void initTheme();
  watchSystemAccent();
  const c = await applySystemAccent();
  if (c) accent = c;
  window.addEventListener("resize", () => { resize(); draw(); });
  const un: UnlistenFn = await listen<SnipOpenPayload>("snip-open", (e) => void onSnipOpen(e.payload));
  window.addEventListener("beforeunload", un);
}

void init().catch((e) => toast(e));

function toast(msg: string) {
  hint.textContent = msg;
}