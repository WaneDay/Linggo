# Linggo · 离线划词翻译

> 一款 Windows 下**完全离线**的划词 / 打字 / 截图 OCR 翻译工具。基于 Tauri 2 + Rust + 原生 WebView2，本地跑 NMT 与大模型，正文翻译不联网、不上传。

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%2F11%20x64-0078D4.svg)](#系统要求)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB.svg)](https://tauri.app/)
[![Version](https://img.shields.io/badge/version-0.1.0-blue.svg)](../../releases)

---

## 简介

Linggo 把「翻译」变成一个随时可用的系统级能力：选中文本按一下键、输入框里敲一下回车、屏幕上框一块区域，译文立刻出现，并且可以直接回贴到原窗口或贴图置顶。
<p align="center">
  <img src="src-tauri/icons/icon.png" alt="Linggo logo" width="128" height="128" />
</p>
- **完全离线**：翻译、OCR 全部在本机完成；只有「检查更新」会访问 GitHub，且不自动下载。
- **三种引擎**：OPUS（中英极快）、NLLB（多语种快速）、GGUF 大模型（Hy-MT2-1.8B，质量优先），可自由排序与回退。
- **游戏友好**：全屏游戏时自动禁用热键，避免误触。

> 本项目为 Windows 10 简化高性能版实现，使用 Tauri 2 构建，安装包约 **11 MB**（模型按需下载）。

---

## 功能特性

| 功能 | 说明 |
| --- | --- |
| **划词翻译 (F1)** | 选中任意程序的文本，按 F1 弹出原文 / 译文 |
| **打字翻译回贴 (F2)** | 弹窗输入，Enter 把译文直接粘贴替换回原窗口 |
| **截图 OCR 翻译 (F3)** | 框选屏幕区域，本地 OCR 识别 + 翻译，可「贴图置顶」对照 |
| **新建文件夹 (F4)** | 在文件管理器 / 桌面用输入框建文件夹，支持「译名 / 原名 / 取消」 |
| **主窗口 (F5)** | 双栏翻译 + 全部设置 + 翻译历史 |
| **本地模型市场** | 内置语言包索引，一键下载 OPUS / NLLB，自动发现 `models` 目录 |
| **智能内存管理** | 闲置自动释放大模型（可设 5/15/30 秒或永久常驻） |
| **游戏模式** | 前台全屏游戏时自动屏蔽全局热键 |
| **跟随系统** | 外观跟随系统明暗，环形倒计时取系统强调色 |
| **托盘常驻** | 关闭主窗口 = 最小化到托盘；可设开机自启（静默） |
| **更新提示** | 启动静默检查 GitHub Releases，仅提示不强制升级 |

---

## 快捷键一览（简略使用教程）

| 快捷键 | 功能 | 用法提要 |
| --- | --- | --- |
| `F1` | 划词翻译 | 先选中文本 → 按 F1 → 查看 / 复制译文 |
| `F2` | 打字翻译并回贴 | 按 F2 → 输入 → `Enter` 回贴到原窗口；`Esc` 关闭 |
| `F3` | 截图 OCR 翻译 | 按 F3 → 拖拽框选 → 松开后自动识别翻译；可贴图置顶 |
| `F4` | 新建文件夹 | 鼠标移到资源管理器 / 桌面 → 按 F4 → 输入名字 |
| `F5` | 主窗口 | 打开双栏翻译与设置面板 |
| `Enter` | 确认 / 回贴 | F2 回贴译文；F4 用**译文名**建夹 |
| `Esc` | 关闭 / 原名 | 弹窗关闭；F4 中用**原文名**建夹 |
| `Delete` | 取消 | F4 中取消本次创建并关闭弹窗 |
| `Ctrl + Enter` | 翻译 | 主窗口（F5）内触发翻译 |

> 全部热键均可在「设置 → 全局热键」中自定义。详细的逐功能说明见 **[docs/USAGE.md](docs/USAGE.md)**。

---

## 快速开始

### 方式一：下载安装包（推荐）

1. 前往本仓库 **Releases** 页面下载 `Linggo-Setup-x.y.z.exe`。
2. 运行安装程序：向导中可选择**首选语言 / 次选语言**，并预设闲置释放、历史条数、外观主题、游戏模式等偏好（安装后可在软件「设置」中随时修改）。
3. 按向导完成安装（可选桌面快捷方式、开机自启）。
4. 首次启动后，打开主窗口（`F5`）→「设置 → 模型市场」下载模型，或「模型文件 → 浏览」选择本地 `.gguf`。

> 系统要求：Windows 10 1809+ / Windows 11（x64）。WebView2 运行时 Win11 自带，Win10 通常随 Edge 已安装。

### 方式二：从源码构建

```powershell
# 1. 前端依赖 + 打包
npm install
npm run build

# 2. 编译 Rust（务必带 custom-protocol，否则前端资源无法加载）
cd src-tauri
cargo build --release --features custom-protocol
# 产物：src-tauri\target\release\linggo.exe
```

- 编译后的 `linggo.exe` 可直接运行（首次需在设置里选择 / 下载模型）。
- 需要生成安装包时：把 `target\release\linggo.exe` 复制为 `target\release\Linggo.exe`，再用 Inno Setup 编译 `Linggo.iss`（默认输出到仓库上一级的 `发布包\`；向导中含首选/次选语言与偏好设置页）。
- **完整的环境准备、常见编译报错与离线依赖说明见 [docs/BUILD.md](docs/BUILD.md)。**

---

## 引擎与模型

Linggo 内置三级翻译引擎，F1–F4 按「设置 → 引擎排序」依次尝试，F5 默认使用质量最高的 GGUF：

| 引擎 | 类型 | 适用 | 体积（约） | 备注 |
| --- | --- | --- | --- | --- |
| `OPUS` | NMT (CTranslate2) | 中 ↔ 英 | ~80–160 MB / 方向 | 速度最快，常驻缓存 |
| `NLLB` | NMT (CTranslate2) | 多语种互译 | ~1.4 GB | 闲置自动回收 |
| `GGUF` | 大模型 (llama.cpp) | 任意语种，质量优先 | ~1.1 GB | Hy-MT2-1.8B，闲置卸载 |

- 模型文件不入库，**在软件内「模型市场」一键下载**，或手动放到可执行文件旁的 `models/` 目录。
- 模型来源与手动安装方法见 **[docs/MODELS.md](docs/MODELS.md)**。

---

## 目录结构

```
.
├─ index.html / popup.html / snip.html / pin.html   # 4 个窗口入口
├─ src/                    # 前端 TypeScript / CSS
│  ├─ main.ts              # 主窗口逻辑
│  ├─ popup.ts             # 弹窗（F1–F4 结果 / 输入）
│  ├─ snip.ts / pin.ts     # 截图压盖 / 贴图
│  ├─ shared.ts            # 主题 / 强调色等共享逻辑
│  └─ theme.css            # 全局样式
├─ src-tauri/              # Rust 后端
│  ├─ src/                 # 各功能模块（见 docs/ARCHITECTURE.md）
│  ├─ capabilities/        # 窗口权限声明
│  ├─ icons/               # 全套应用图标
│  ├─ vendor/crates/       # 本地补丁依赖（llama-cpp-2 / llama-cpp-sys-2）
│  ├─ Cargo.toml / Cargo.lock
│  ├─ tauri.conf.json
│  └─ pkg_index.json       # 内置语言包索引
├─ docs/                   # 文档（构建 / 使用 / 模型 / 架构 / FAQ ...）
├─ Linggo.iss              # Inno Setup 安装脚本（含语言/偏好选择页）
└─ LICENSE                 # MIT
```

> `src-tauri/vendor/` 是**编译必需**的本地补丁依赖（修复 MSVC 运行库链接），请勿删除。

---

## 文档索引

| 文档 | 内容 |
| --- | --- |
| [docs/USAGE.md](docs/USAGE.md) | 详细使用说明（逐功能 + 全部设置项） |
| [docs/BUILD.md](docs/BUILD.md) | 从源码构建、环境准备、打包安装包 |
| [docs/MODELS.md](docs/MODELS.md) | 引擎原理、模型下载与手动安装 |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 代码架构与模块职责 |
| [docs/FAQ.md](docs/FAQ.md) | 常见问题与排错 |
| [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) | 参与开发与贡献指南 |
| [docs/CHANGELOG.md](docs/CHANGELOG.md) | 版本更新记录 |
| [README.en.md](README.en.md) | English README |

---

## 常见问题（速览）

- **按 F1 没反应？** 先确认已选中文本；部分程序（如部分终端 / 游戏反作弊）会拦截 `Ctrl+C`，导致无法抓取选区。
- **提示未加载模型？** 到「设置 → 模型」选择或下载 `.gguf`；仅用 OPUS/NLLB 时请确认已在模型市场安装对应语言包。
- **热键和别的软件冲突？** 到「设置 → 全局热键」改成其他组合。
- **全屏游戏里热键误触？** 保持「游戏模式」勾选，或关闭后手动按需使用。
- 更多见 **[docs/FAQ.md](docs/FAQ.md)**。

---

## 声明

- 本项目仅供学习与个人使用，请遵守当地法律法规及第三方模型 / 素材的许可协议。
- 翻译质量取决于所选开源模型，仅供参考，不保证准确性。
- 软件除「检查更新」外不进行任何联网；模型由用户自行下载，版权归各自发布方所有。

## 许可证

本项目采用 [MIT License](LICENSE) 开源。

## 致谢

感谢 [Tauri](https://tauri.app/)、[llama.cpp](https://github.com/ggml-org/llama.cpp)、[CTranslate2](https://github.com/OpenNMT/CTranslate2)、[OPUS-MT](https://github.com/Helsinki-NLP)、[NLLB](https://github.com/facebookresearch/fairseq/tree/nllb)、[Hy-MT2](https://huggingface.co/tencent) 等开源项目。
