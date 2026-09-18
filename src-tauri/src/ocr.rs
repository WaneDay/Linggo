// Linggo OCR 模块：Windows.Media.Ocr 本地文字识别（免 Python / RapidOCR），
// 含深色反相/自动放大/贴边留白等预处理；并提供 F3 全链路命令
// capture_screen / ocr_snip / snip_crop_png。

use crate::state::{AppState, ScreenFrame};
use serde::Serialize;
use std::io::Cursor;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrWord {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLine {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub words: Vec<OcrWord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrResult {
    pub text: String,
    pub lines: Vec<OcrLine>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenCaptureData {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub data_url: String,
}

fn rgba_to_png_b64(rgba: &[u8], w: u32, h: u32) -> Result<String, String> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or("像素数据无效")?;
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    use base64::Engine as _;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(buf)
    ))
}

/// 从整屏帧裁剪一块（越界自动夹紧；越界过多视为无效区域）
fn crop_rgba(frame: &ScreenFrame, x: i32, y: i32, w: i32, h: i32) -> Result<(Vec<u8>, u32, u32), String> {
    let fw = frame.width as i32;
    let fh = frame.height as i32;
    let x0 = x.max(0).min(fw - 1);
    let y0 = y.max(0).min(fh - 1);
    let cw = w.min(fw - x0).max(1) as usize;
    let ch = h.min(fh - y0).max(1) as usize;
    let mut out = Vec::with_capacity(cw * ch * 4);
    for row in 0..ch as u32 {
        let src_off = (((y0 as u32 + row) * frame.width) + x0 as u32) as usize * 4;
        let len = cw * 4;
        let end = src_off + len;
        if end > frame.rgba.len() {
            return Err("裁剪区域超出截图范围".to_string());
        }
        out.extend_from_slice(&frame.rgba[src_off..end]);
    }
    Ok((out, cw as u32, ch as u32))
}

// ---------------------------------------------------------------------------
// Windows OCR 核心
// ---------------------------------------------------------------------------

fn init_mta() {
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
}

fn lang_prefs(source_lang: &str) -> Vec<&'static str> {
    match source_lang.trim() {
        "" | "auto" => vec!["zh-Hans", "en"],
        "zh" => vec!["zh-Hans", "en"],
        "zh-Hant" => vec!["zh-Hant", "zh-Hans"],
        "en" => vec!["zh-Hans", "en"],
        "ja" => vec!["ja"],
        "ko" => vec!["ko"],
        "fr" => vec!["fr"],
        "de" => vec!["de"],
        "es" => vec!["es"],
        "it" => vec!["it"],
        "pt" => vec!["pt"],
        "ru" => vec!["ru"],
        "ar" => vec!["ar"],
        "hi" => vec!["hi"],
        "vi" => vec!["vi"],
        "th" => vec!["th"],
        "tr" => vec!["tr"],
        "nl" => vec!["nl"],
        "pl" => vec!["pl"],
        "el" => vec!["el"],
        "sv" => vec!["sv"],
        _ => vec!["en"],
    }
}

fn pick_language(prefs: &[&str]) -> Result<windows::Globalization::Language, String> {
    use windows::Media::Ocr::OcrEngine;
    let avail = OcrEngine::AvailableRecognizerLanguages().map_err(|e| e.to_string())?;
    let list: Vec<windows::Globalization::Language> = avail.into_iter().collect();
    if list.is_empty() {
        return Err(
            "系统未安装 OCR 语言包。请到 设置→时间和语言→语言和区域→语言选项，安装“光学字符识别 (OCR)”功能"
                .to_string(),
        );
    }
    let tags: Vec<String> = list
        .iter()
        .map(|l| l.LanguageTag().map(|s| s.to_string()).unwrap_or_default().to_lowercase())
        .collect();
    for p in prefs {
        let pl = p.to_lowercase();
        for (i, t) in tags.iter().enumerate() {
            if t == &pl || t.starts_with(&pl) || pl.starts_with(t.as_str()) {
                return Ok(list[i].clone());
            }
        }
    }
    Ok(list[0].clone())
}

fn dark_background(img: &image::RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return false;
    }
    let step = (w.max(h) / 240).max(1);
    let (mut sum, mut n) = (0u64, 0u64);
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let p = img.get_pixel(x, y);
            sum += (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) as u64;
            n += 1;
            x += step;
        }
        y += step;
    }
    n > 0 && sum / n < 120
}

fn is_cjkish(c: char) -> bool {
    let u = c as u32;
    (0x3400..=0x9FFF).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0x3040..=0x30FF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0x3000..=0x303F).contains(&u)
        || (0xFF00..=0xFFEF).contains(&u)
}

/// Windows OCR 会在 CJK「词」间插空格，两侧都是 CJK 的空格去掉，保留拉丁单词间隔。
fn collapse_cjk_spaces(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ' ' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let prev = out.chars().last();
            let next = chars.get(j).copied();
            let between_cjk =
                matches!((prev, next), (Some(p), Some(n)) if is_cjkish(p) && is_cjkish(n));
            if !between_cjk && prev.is_some() && next.is_some() {
                out.push(' ');
            }
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// 识别一张 RGBA8 图，返回逐行文本 + 行框（原图像素坐标）。
fn ocr_recognize(width: u32, height: u32, rgba: &[u8], source_lang: &str) -> Result<Vec<OcrLine>, String> {
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Security::Cryptography::CryptographicBuffer;

    if width == 0 || height == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
        return Err("截图数据无效".to_string());
    }
    init_mta();

    let mut base = image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or("截图数据无效")?;

    // 深色底（浅字深字）反相为深字浅底再识别，是 Windows OCR 命中率杀手，通常必须做
    if dark_background(&base) {
        for px in base.pixels_mut() {
            px[0] = 255 - px[0];
            px[1] = 255 - px[1];
            px[2] = 255 - px[2];
        }
    }

    // 小图/小字放大：长 <2000 放大到 2000，短 <200 追加考虑短边；上限 2x，超 9000 封顶
    let long = width.max(height) as f32;
    let short = width.min(height) as f32;
    let f_long = if long < 2000.0 { 2000.0 / long } else { 1.0 };
    let f_short = if short < 200.0 { 200.0 / short } else { 1.0 };
    let mut factor = f_long.max(f_short).min(2.0);
    if long * factor > 9000.0 {
        factor = 9000.0 / long;
    }
    let up = if factor > 1.01 {
        let nw = (((width as f32) * factor).round() as u32).max(1);
        let nh = (((height as f32) * factor).round() as u32).max(1);
        let scaled = image::imageops::resize(&base, nw, nh, image::imageops::FilterType::Lanczos3);
        image::imageops::unsharpen(&scaled, 1.2, 2)
    } else {
        base
    };

    // 四周留白（采样背景色）：避免贴边/孤立短文本漏字
    let (uw, uh) = up.dimensions();
    let pad = ((uw.max(uh) as f32) * 0.12).round().max(36.0) as u32;
    let bgpx = {
        let c = [
            up.get_pixel(0, 0),
            up.get_pixel(uw - 1, 0),
            up.get_pixel(0, uh - 1),
            up.get_pixel(uw - 1, uh - 1),
        ];
        let mut a = [0u32; 3];
        for p in c {
            a[0] += p[0] as u32;
            a[1] += p[1] as u32;
            a[2] += p[2] as u32;
        }
        image::Rgba([(a[0] / 4) as u8, (a[1] / 4) as u8, (a[2] / 4) as u8, 255])
    };
    let mut img = image::RgbaImage::from_pixel(uw + pad * 2, uh + pad * 2, bgpx);
    image::imageops::overlay(&mut img, &up, pad as i64, pad as i64);
    let (w, h) = img.dimensions();

    // RGBA→BGRA，alpha 强制不透明（透明像素会被当黑色导致识别失败）
    let mut bgra = img.into_raw();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }

    let buffer = CryptographicBuffer::CreateFromByteArray(&bgra).map_err(|e| e.to_string())?;
    let bitmap =
        SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w as i32, h as i32)
            .map_err(|e| e.to_string())?;

    let lang = pick_language(&lang_prefs(source_lang))?;
    let engine = OcrEngine::TryCreateFromLanguage(&lang).map_err(|e| e.to_string())?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| e.to_string())?
        .get()
        .map_err(|e| e.to_string())?;

    let lines = result.Lines().map_err(|e| e.to_string())?;
    let mut out: Vec<OcrLine> = Vec::new();
    for line in lines {
        let raw = line.Text().map(|s| s.to_string()).unwrap_or_default();
        let t = collapse_cjk_spaces(&raw);
        let t = t.trim().to_string();
        if t.is_empty() {
            continue;
        }
        let (mut l, mut tt, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut words: Vec<OcrWord> = Vec::new();
        let p = pad as f32;
        let unmap = |v: f32| (v - p) / factor;
        for word in line.Words().map_err(|e| e.to_string())? {
            if let Ok(rect) = word.BoundingRect() {
                l = l.min(rect.X);
                tt = tt.min(rect.Y);
                r = r.max(rect.X + rect.Width);
                b = b.max(rect.Y + rect.Height);
                let wt = word
                    .Text()
                    .map(|s| collapse_cjk_spaces(&s.to_string()))
                    .unwrap_or_default();
                let wt = wt.trim().to_string();
                if !wt.is_empty() {
                    words.push(OcrWord {
                        text: wt,
                        x: unmap(rect.X),
                        y: unmap(rect.Y),
                        w: rect.Width / factor,
                        h: rect.Height / factor,
                    });
                }
            }
        }
        let bbox = if l <= r && tt <= b {
            Some((unmap(l), unmap(tt), (r - l) / factor, (b - tt) / factor))
        } else {
            None
        };
        let (x, y, w, h) = bbox.unwrap_or((0.0, 0.0, 0.0, 0.0));
        out.push(OcrLine { text: t, x, y, w, h, words });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 命令面
// ---------------------------------------------------------------------------

/// 截取整个虚拟屏幕，缓存帧（供裁剪/OCR）并返回 PNG data URL（snip 窗口展示用）
#[tauri::command]
pub async fn capture_screen(app: AppHandle) -> Result<ScreenCaptureData, String> {
    let shot = tauri::async_runtime::spawn_blocking(crate::win32::capture_virtual_screen)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("截图失败：请检查屏幕是否被锁定或远程会话未就绪")?;
    let data_url = rgba_to_png_b64(&shot.rgba, shot.width as u32, shot.height as u32)?;
    *app.state::<AppState>().screen_capture.lock().unwrap() = Some(ScreenFrame {
        left: shot.left,
        top: shot.top,
        width: shot.width as u32,
        height: shot.height as u32,
        rgba: shot.rgba,
    });
    Ok(ScreenCaptureData {
        left: shot.left,
        top: shot.top,
        width: shot.width as u32,
        height: shot.height as u32,
        data_url,
    })
}

/// 对整屏帧的矩形区域做 OCR，返回识别文本与行框（区域原图坐标）
#[tauri::command]
pub async fn ocr_snip(app: AppHandle, x: i32, y: i32, w: i32, h: i32) -> Result<OcrResult, String> {
    let frame = app.state::<AppState>().screen_capture.lock().unwrap().clone().ok_or("请先截图")?;
    let lang = crate::settings::current(&app).ocr_lang;
    tauri::async_runtime::spawn_blocking(move || {
        let (crop, cw, ch) = crop_rgba(&frame, x, y, w, h)?;
        let lines = ocr_recognize(cw, ch, &crop, &lang)?;
        let text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        Ok(OcrResult { text, lines })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 从整屏帧裁一块并返回 PNG data URL（F3 的「复制截图 / 贴图置顶」用）
#[tauri::command]
pub async fn snip_crop_png(app: AppHandle, x: i32, y: i32, w: i32, h: i32) -> Result<String, String> {
    let frame = app.state::<AppState>().screen_capture.lock().unwrap().clone().ok_or("请先截图")?;
    tauri::async_runtime::spawn_blocking(move || {
        let (crop, cw, ch) = crop_rgba(&frame, x, y, w, h)?;
        rgba_to_png_b64(&crop, cw, ch)
    })
    .await
    .map_err(|e| e.to_string())?
}