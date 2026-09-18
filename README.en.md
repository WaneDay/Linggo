<p align="center">
  <img src="src-tauri/icons/icon.png" alt="Linggo logo" width="128" height="128" />
</p>

# Linggo · Offline Selection Translator

> A **fully offline** translate-anywhere tool for Windows: translate selected text, typed text, or a screen region via OCR. Built with Tauri 2 + Rust + native WebView2, running NMT and LLM inference locally. No text ever leaves your machine.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%2F11%20x64-0078D4.svg)](#requirements)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB.svg)](https://tauri.app/)
[![Version](https://img.shields.io/badge/version-0.1.0-blue.svg)](../../releases)

---

## Highlights

- **Fully offline** — translation and OCR run entirely on-device. Only the update check contacts GitHub, and it never auto-downloads.
- **Three engines** — OPUS (fast FR↔EN), NLLB (multilingual), and GGUF LLM (quality-first), with configurable fallback order.
- **Gaming friendly** — global hotkeys are automatically suspended while a fullscreen game is foreground.
- **Tiny installer** (~5 MB); models are downloaded on demand through a built-in model market.

## Hotkeys

| Key | Action | Notes |
| --- | --- | --- |
| `F1` | Translate selection | Select text anywhere, press F1 |
| `F2` | Type & paste back | Press F2, type, `Enter` pastes translation into the original window |
| `F3` | Screenshot OCR translate | Press F3, drag a region; result can be pinned on top |
| `F4` | New folder | Type a name; `Enter` = translated name, `Esc` = raw name, `Delete` = cancel |
| `F5` | Main window | Dual-pane translation + all settings |
| `Ctrl + Enter` | Translate | Inside the main window |

All hotkeys are customizable in **Settings → Global Hotkeys**.

## Quick start

**Install:** grab `Linggo-Setup-x.y.z.exe` from Releases. The wizard lets you pick a **preferred / secondary language** and preset several preferences; then open the main window (`F5`) → **Settings → Model Market** to download a model, or pick a local `.gguf`.

**Build from source:**

```powershell
npm install
npm run build
cd src-tauri
cargo build --release --features custom-protocol
# output: src-tauri\target\release\linggo.exe
```

> The `custom-protocol` feature is **required**; without it the frontend assets will not load (blank window). See [docs/BUILD.md](docs/BUILD.md).

## Engines

| Engine | Type | Best for | Size |
| --- | --- | --- | --- |
| `OPUS` | NMT (CTranslate2) | zh ↔ en | ~80–160 MB / direction |
| `NLLB` | NMT (CTranslate2) | many languages | ~1.4 GB |
| `GGUF` | LLM (llama.cpp) | any, quality-first | ~1.1 GB |

Models are not bundled — download them in-app or drop files into `models/` next to the executable. See [docs/MODELS.md](docs/MODELS.md).

## Documentation

[Usage](docs/USAGE.md) · [Build](docs/BUILD.md) · [Models](docs/MODELS.md) · [Architecture](docs/ARCHITECTURE.md) · [FAQ](docs/FAQ.md) · [Contributing](docs/CONTRIBUTING.md) · [Changelog](docs/CHANGELOG.md) · [中文说明](README.md)

## Requirements

Windows 10 1809+ / Windows 11 (x64). WebView2 Runtime is included with Windows 11 and normally installed on Windows 10 via Microsoft Edge.

## License

[MIT](LICENSE) © 2026 WaneDay

## Credits

[Tauri](https://tauri.app/) · [llama.cpp](https://github.com/ggml-org/llama.cpp) · [CTranslate2](https://github.com/OpenNMT/CTranslate2) · [OPUS-MT](https://github.com/Helsinki-NLP) · [NLLB](https://github.com/facebookresearch/fairseq/tree/nllb) · [Hy-MT2](https://huggingface.co/tencent)
