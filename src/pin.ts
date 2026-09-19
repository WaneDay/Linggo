// Linggo 贴图置顶窗口（Snipaste 风格）：
// - 透明无边框窗，CSS drop-shadow 用系统强调色发光描边
// - 可选「原位置贴图」：按 place 在截图选区原位置放置，先显示原图再增量覆盖译文
// - 左键点击空白处切换 原图 ↔ 覆盖图（默认覆盖图，设置可改）
// - 滚轮缩放（以窗口几何中心为锚，窗口随缩放变大/变小）、拖拽移动、右键/Esc 关闭。
//
// 坐标模型（关键）：
//   zoom = 用户缩放倍率，1 = 图像 1 物理像素对应屏幕 1 物理像素（原始大小）。
//   画布使用 CSS 坐标（已 setTransform(dpr)），因此绘制尺寸 = imgW * zoom / dpr。
//   窗口物理尺寸 = 图像物理尺寸 + 两侧 PAD（CSS px，随 dpr 换算）→ 发光阴影始终可见。

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, PhysicalPosition, PhysicalSize } from "@tauri-apps/api/window";
import type { PinLine, PinOpenPayload, PinUpdatePayload, PinTextRow } from "./shared";
import { applySystemAccent, watchSystemAccent, initTheme } from "./shared";

const win = getCurrentWindow();
const canvas = document.getElementById("pinCanvas") as HTMLCanvasElement;
const ctx = canvas.getContext("2d", { willReadFrequently: true })!;
const tip = document.getElementById("pinTip") as HTMLDivElement;

const PAD = 16; // 发光阴影留白（CSS px），需明显大于最外层 drop-shadow 扩散半径，避免阴影被窗口边缘截断成硬边
const ZOOM_MIN = 0.04;
const ZOOM_MAX = 16;
const TIP_TEXT = "左键切换原图/译文 · 拖拽移动 · 滚轮缩放 · 右键/Esc 关闭 · Ctrl+C 复制译文/原文";
const TIP_GAP = 3; // 提示词下方与贴图右上角边缘的间隙（CSS px）

let img: HTMLImageElement | null = null;
let imgW = 0;
let imgH = 0;
let zoom = 1; // 用户缩放倍率，1 = 原始物理 1:1
let ds = 1; // 当前绘制比例（CSS px / 图像像素）= zoom / dpr
let offX = 0;
let offY = 0;
let topPad = PAD; // 图像顶部留白（CSS px），需要给快捷键提示/贴图上方留位时增大
let overlayMode = true;
let lines: PinLine[] = [];
let rows: PinTextRow[] = [];
let accent = "#0078d4";
let showTip = true; // 贴图快捷键提示（设置 pin_show_tip 控制）

function dpr() { return window.devicePixelRatio || 1; }

// 强调色 → rgba（多层阴影用不同透明度，边缘更柔和）
function accentRgba(alpha: number): string {
  let r = 0, g = 120, b = 212; // 默认 #0078d4
  const s = (accent || "").trim();
  const m6 = s.match(/^#?([0-9a-fA-F]{6})$/);
  const m3 = s.match(/^#?([0-9a-fA-F]{3})$/);
  const mRgb = s.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i);
  if (m6) {
    r = parseInt(m6[1].slice(0, 2), 16);
    g = parseInt(m6[1].slice(2, 4), 16);
    b = parseInt(m6[1].slice(4, 6), 16);
  } else if (m3) {
    r = parseInt(m3[1][0] + m3[1][0], 16);
    g = parseInt(m3[1][1] + m3[1][1], 16);
    b = parseInt(m3[1][2] + m3[1][2], 16);
  } else if (mRgb) {
    r = +mRgb[1]; g = +mRgb[2]; b = +mRgb[3];
  }
  return `rgba(${r},${g},${b},${alpha})`;
}

// 柔和发光阴影：更窄的模糊半径（厚度小）+ 低透明度，边缘过渡干净不锐利
function glowFilter(): string {
  return [
    `drop-shadow(0 1px 2px rgba(0,0,0,0.30))`,
    `drop-shadow(0 0 2px ${accentRgba(0.5)})`,
    `drop-shadow(0 0 5px ${accentRgba(0.18)})`,
  ].join(" ");
}

function resize() {
  const d = dpr();
  canvas.width = Math.round(canvas.clientWidth * d);
  canvas.height = Math.round(canvas.clientHeight * d);
  ctx.setTransform(d, 0, 0, d, 0, 0);
}

// 图像物理尺寸 + 上下留白 → 窗口物理尺寸
function plannedSize() {
  const d = dpr();
  const top = (showTip ? topPad : PAD) * d;
  const bot = PAD * d;
  return {
    w: Math.max(48, Math.round(imgW * zoom + (top + bot))),
    h: Math.max(32, Math.round(imgH * zoom + (top + bot))),
  };
}

function render() {
  const cw = canvas.clientWidth;
  const ch = canvas.clientHeight;
  ctx.clearRect(0, 0, cw, ch);
  if (!img) return;
  ds = zoom / dpr();
  const dw = imgW * ds;
  const dh = imgH * ds;
  offX = (cw - dw) / 2;
  offY = topPad; // 图像顶部留白（可能被快捷键提示占据），底部居中通过窗口尺寸约束
  ctx.drawImage(img, offX, offY, dw, dh);
  if (overlayMode) renderOverlay();
  positionTip(cw, ch, dw, dh);
}

// 快捷键提示锚定在贴图右上角上方：右缘与图像右缘对齐，底缘贴着图像上缘（TIP_GAP 小间隙）。
// 顶部空间由 ensureTipRoom() 在贴图时预留，正常不会触发钳制/压到图上。
function positionTip(cw: number, ch: number, dw: number, dh: number) {
  const th = tip.offsetHeight || 16;
  const tw = tip.offsetWidth || 0;
  const ix = offX + dw; // 图像右缘
  const iy = offY;      // 图像上缘
  let left = ix - tw;
  if (left < 2) left = 2;
  if (left + tw > cw - 2) left = Math.max(2, cw - tw - 2);
  let top = iy - th - TIP_GAP;
  if (top < 2) top = 2;
  tip.style.left = `${left}px`;
  tip.style.top = `${top}px`;
  applyTipVisibility();
}

// 给提示词在贴图上方腾出空间：需要时把图像顶部留白加大，并同步上移/增高窗口（贴图图像保持原位）
async function ensureTipRoom() {
  if (!showTip || !img) return;
  const th = tip.offsetHeight || 20;
  const tw = tip.offsetWidth || 0;
  const need = TIP_GAP + th;
  const old = topPad;
  let grow = 0;
  if (need > old) grow = need - old;
  const right = offX + imgW * ds;
  if (tw > 2 && offX + tw > right + 2 && grow <= 0) {
    // 右侧无扩展需求，只处理顶部空间
  }
  if (grow > 0 && old > 0) {
    const d = dpr();
    try {
      const pos = await win.outerPosition();
      const size = await win.outerSize();
      await win.setPosition(new PhysicalPosition(pos.x, Math.round(pos.y - grow * d)));
      await win.setSize(new PhysicalSize(
        size.width,
        Math.round(size.height + grow * d),
      ));
    } catch { /* 瞬时窗口操作失败可忽略 */ }
    topPad += grow;
    resize();
  }
  render();
}

function applyTipVisibility() {
  tip.style.display = showTip ? "" : "none";
}

const FONT_FAMILY = '"Microsoft YaHei", "Segoe UI", "Noto Sans SC", sans-serif';

function isCJK(ch: string): boolean {
  const u = ch.codePointAt(0) ?? 0;
  return (u >= 0x2e80 && u <= 0x9fff) || (u >= 0xf900 && u <= 0xfaff) ||
    (u >= 0x3040 && u <= 0x30ff) || (u >= 0xac00 && u <= 0xd7a3) ||
    (u >= 0xff00 && u <= 0xffef);
}

// 中文逐字断行，拉丁文按词断行
function tokenizeText(text: string): string[] {
  const units: string[] = [];
  let buf = "";
  const flush = () => { if (buf) { units.push(buf); buf = ""; } };
  for (const ch of text) {
    if (/\s/.test(ch)) flush();
    else if (isCJK(ch)) { flush(); units.push(ch); }
    else buf += ch;
  }
  flush();
  return units;
}

function wrapText(text: string, maxW: number): string[] {
  const units = tokenizeText(text);
  const out: string[] = [];
  let cur = "";
  let prevCJK = false;
  for (const u of units) {
    const cjk = isCJK(u[0]);
    const sep = cur && !prevCJK && !cjk ? " " : "";
    const test = cur + sep + u;
    if (cur && ctx.measureText(test).width > maxW) {
      out.push(cur);
      cur = u;
    } else {
      cur = test;
    }
    prevCJK = cjk;
  }
  if (cur) out.push(cur);
  return out.length ? out : [text];
}

// 在段落框内求可读字号并换行：
// - 优先取「源单行高度 × 0.82」的字号（与原文大小接近），避免被压得过小；
// - 允许有限纵向溢出（最多 2.2 倍框高）来容纳更长的译文。
function layoutParagraph(text: string, W: number, H: number, srcLines: number) {
  const lineHeight = 1.16;
  const sh = H / Math.max(1, srcLines); // 源文本单行高度
  const preferred = Math.max(11, sh * 0.82);
  const maxH = H * 2.2;
  let best: { f: number; lines: string[]; lh: number } | null = null;
  for (let f = Math.floor(preferred); f >= 11; f--) {
    ctx.font = `${f}px ${FONT_FAMILY}`;
    const wrapped = wrapText(text, W);
    const lh = f * lineHeight;
    if (wrapped.length * lh <= maxH) {
      best = { f, lines: wrapped, lh };
      break;
    }
  }
  if (!best) {
    const f = 11;
    ctx.font = `${f}px ${FONT_FAMILY}`;
    best = { f, lines: wrapText(text, W), lh: f * lineHeight };
  }
  ctx.font = `${best.f}px ${FONT_FAMILY}`;
  return best;
}

function renderOverlay() {
  for (const line of lines) {
    const tr = (line.translated ?? "").trim();
    if (!tr) continue;
    const x = offX + line.x * ds;
    const y = offY + line.y * ds;
    const w = line.w * ds;
    const h = line.h * ds;
    if (w < 2 || h < 2) continue;
    // 背景色 + 排版（先算排版，背景按实际占用高度覆盖，可有限溢出）
    const bg = sampleBgColor(line.x, line.y, line.w, line.h);
    const { lines: wrapped, lh } = layoutParagraph(tr, w, h, line.n ?? 1);
    const needH = wrapped.length * lh;
    const fillH = Math.max(h, Math.min(needH, h * 2.2));
    ctx.fillStyle = bg;
    ctx.fillRect(x - 1, y - 1, w + 2, fillH + 2);
    const brightness = colorBrightness(bg);
    ctx.fillStyle = brightness > 120 ? "#1a1a1a" : "#f0f0f0";
    ctx.textBaseline = "middle";
    ctx.textAlign = "center";
    let cy = y + Math.max(0, (fillH - needH) / 2) + lh / 2;
    for (const ln of wrapped) {
      ctx.fillText(ln, x + w / 2, cy, w);
      cy += lh;
    }
    ctx.textAlign = "left";
  }
}

function sampleBgColor(imgX: number, imgY: number, imgW: number, imgH: number): string {
  // 采样行框左侧和右侧 4px 条带的像素均值
  if (!img) return "rgba(200,200,200,1)";
  try {
    const cw = img.naturalWidth;
    const ch = img.naturalHeight;
    const x0 = Math.max(0, Math.round(imgX - 4));
    const y0 = Math.max(0, Math.round(imgY));
    const x1 = Math.min(cw, Math.round(imgX + imgW + 4));
    const y1 = Math.min(ch, Math.round(imgY + imgH));
    const sw = x1 - x0;
    const sh = y1 - y0;
    if (sw <= 0 || sh <= 0) return "rgba(200,200,200,1)";
    // 临时 canvas 读取像素
    const tc = document.createElement("canvas");
    tc.width = sw; tc.height = sh;
    const tx = tc.getContext("2d")!;
    tx.drawImage(img, x0, y0, sw, sh, 0, 0, sw, sh);
    const data = tx.getImageData(0, 0, sw, sh).data;
    let r = 0, g = 0, b = 0, n = 0;
    // 只采样上下边缘行（代表背景色）
    for (let row = 0; row < sh; row++) {
      for (let col = 0; col < sw; col++) {
        const px = (row * sw + col) * 4;
        const dy = row / sh;
        // 仅在上下 30% 区域采样（背景色集中在文字行的上下空白）
        if (dy < 0.3 || dy > 0.7) {
          r += data[px]; g += data[px+1]; b += data[px+2]; n++;
        }
      }
    }
    if (n === 0) { r = g = b = 200; n = 1; }
    return `rgb(${(r/n)|0},${(g/n)|0},${(b/n)|0})`;
  } catch { return "rgba(200,200,200,1)"; }
}

function colorBrightness(rgba: string): number {
  const m = rgba.match(/\d+/g);
  if (!m || m.length < 3) return 120;
  return (+m[0] * 299 + +m[1] * 587 + +m[2] * 114) / 1000;
}

// 复制：显示翻译覆盖图 → 复制全部译文；未覆盖 → 复制截图 OCR 原文
function copyAllText(): string {
  if (overlayMode) {
    const trs = lines.map((l) => (l.translated ?? "").trim()).filter(Boolean);
    if (trs.length) return trs.join("\n");
  }
  if (rows.length) return rows.map((r) => r.text).join("\n");
  const srcs = lines.map((l) => l.text).filter(Boolean);
  return srcs.join("\n");
}

let tipTimer: ReturnType<typeof setTimeout> | null = null;
function flashTip(msg: string, ms = 1200) {
  if (tipTimer) clearTimeout(tipTimer);
  tip.textContent = msg;
  tipTimer = setTimeout(() => {
    tip.textContent = TIP_TEXT;
    tipTimer = null;
  }, ms);
}

async function onPinOpen(p: PinOpenPayload) {
  overlayMode = p.overlay !== false;
  lines = p.lines ?? [];
  rows = [];
  topPad = PAD;
  // 读取「贴图显示快捷键提示」设置（默认开启）
  try {
    const s = await invoke<{ pinShowTip?: boolean }>("settings_get");
    showTip = s.pinShowTip !== false;
  } catch {
    showTip = true;
  }
  // 行框坐标与裁剪图同坐标系（snip_crop_png 返回值与 ocr_snip 行框对齐）
  img = new Image();
  img.onload = async () => {
    imgW = img!.naturalWidth;
    imgH = img!.naturalHeight;
    const d = dpr();
    if (p.place) {
      // 原位置贴图：1:1 原始大小，窗口=图像+上下留白，窗口置于选区外扩留白处
      zoom = 1;
      const { w, h } = plannedSize();
      try {
        await win.setPosition(new PhysicalPosition(
          Math.round(p.place.x - topPad * d),
          Math.round(p.place.y - topPad * d),
        ));
        await win.setSize(new PhysicalSize(w, h));
      } catch { /* 尺寸异常不致命 */ }
    } else {
      // 其余入口：适配 90% 屏幕居中显示
      let z = 1;
      try {
        const availW = (window.screen.availWidth || imgW) - PAD * 2;
        const availH = (window.screen.availHeight || imgH) - PAD * 2;
        z = Math.min(1, availW / (imgW / d), availH / (imgH / d));
      } catch { /* 默认 1:1 */ }
      zoom = Math.max(ZOOM_MIN, z);
      const { w, h } = plannedSize();
      try {
        await win.setSize(new PhysicalSize(w, h));
        await win.center();
      } catch { /* 尺寸异常不致命 */ }
    }
    resize();
    render();
    tip.textContent = TIP_TEXT;
    await win.show();
    await win.setFocus();
    await ensureTipRoom();
  };
  img.src = p.dataUrl;
}

// 交互：单击 → 切换原图/覆盖图；按住拖拽 → 移动窗口；右键/Esc → 关闭；Ctrl+C → 复制译文/原文
let downPos: { x: number; y: number } | null = null;
let moved = false;

canvas.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  downPos = { x: e.clientX, y: e.clientY };
  moved = false;
});

canvas.addEventListener("mousemove", (e) => {
  if (!downPos) return;
  const dx = Math.abs(e.clientX - downPos.x);
  const dy = Math.abs(e.clientY - downPos.y);
  if (dx + dy > 6) {
    moved = true;
    void win.startDragging();
    downPos = null;
  }
});

window.addEventListener("mouseup", () => {
  if (downPos && !moved) {
    overlayMode = !overlayMode;
    render();
    tip.textContent = overlayMode ? "覆盖图模式 · 点击切回原图" : "原图模式 · 点击切换译文覆盖";
  }
  downPos = null;
  moved = false;
});

// 关闭：右键贴图 /（鼠标悬停在贴图上时）Esc；左键多次点击不再误关
canvas.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  void win.hide();
});

canvas.addEventListener("mouseenter", () => void win.setFocus());

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    void win.hide();
    return;
  }
  if ((e.ctrlKey || e.metaKey) && (e.key === "c" || e.key === "C")) {
    const txt = copyAllText();
    if (txt) {
      e.preventDefault();
      const n = txt.length;
      void invoke("copy_text", { text: txt })
        .then(() => flashTip(`已复制 ${n} 字${overlayMode ? "（译文）" : "（原文）"}`))
        .catch(() => flashTip("复制失败"));
    }
  }
});

// 滚轮：以窗口几何中心为锚缩放，窗口随图像一起变大/变小
let zoomChain: Promise<void> = Promise.resolve();
canvas.addEventListener("wheel", (e) => {
  e.preventDefault();
  const f = e.deltaY < 0 ? 1.12 : 0.89;
  const nz = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, zoom * f));
  if (nz === zoom) return;
  zoomChain = zoomChain.then(async () => {
    try {
      const pos = await win.outerPosition();
      const size = await win.outerSize();
      const cx = pos.x + size.width / 2;
      const cy = pos.y + size.height / 2;
      zoom = nz;
      const { w, h } = plannedSize();
      await win.setSize(new PhysicalSize(w, h));
      await win.setPosition(new PhysicalPosition(Math.round(cx - w / 2), Math.round(cy - h / 2)));
    } catch { /* 忽略瞬时窗口操作失败 */ }
  });
}, { passive: false });

async function init() {
  void initTheme();
  watchSystemAccent();
  const c = await applySystemAccent();
  if (c) accent = c;
  canvas.style.filter = glowFilter();
  const un = await listen<PinOpenPayload>("pin-open", (e) => void onPinOpen(e.payload));
  const un2 = await listen<PinUpdatePayload>("pin-update", (e) => {
    const p = e.payload;
    lines = p.lines ?? [];
    rows = p.rows ?? [];
    overlayMode = p.overlay !== false;
    render();
  });
  const un3 = await listen<{ pinShowTip?: boolean }>("settings-changed", (e) => {
    if (typeof e.payload.pinShowTip === "boolean" && e.payload.pinShowTip !== showTip) {
      showTip = e.payload.pinShowTip;
      applyTipVisibility();
      void ensureTipRoom();
    }
  });
  window.addEventListener("beforeunload", () => { un(); un2(); un3(); });
  window.addEventListener("resize", () => { resize(); render(); });
}

void init().catch(() => undefined);
