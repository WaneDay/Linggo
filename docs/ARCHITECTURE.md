# 架构说明

Linggo 是一个 Tauri 2 应用：前端（WebView2 + TypeScript）负责界面，Rust 后端负责 Windows 原生能力与模型推理。两者通过 Tauri command / event 通信。

---

## 1. 进程与窗口模型

单个进程，4 个窗口（`src-tauri/tauri.conf.json`）：

| 窗口 label | 入口 | 用途 |
| --- | --- | --- |
| `main` | `index.html` | 主窗口：双栏翻译 + 设置 + 历史 |
| `popup` | `popup.html` | F1–F4 结果 / 输入弹窗（无边框、置顶） |
| `snip` | `snip.html` | F3 截图框选压盖（透明、全屏、置顶） |
| `pin` | `pin.html` | F3 结果贴图（透明、置顶、可拖动缩放） |

应用常驻系统托盘；关闭主窗口只隐藏，托盘菜单「退出」才结束进程。单实例运行。

---

## 2. 前端（`src/`）

| 文件 | 职责 |
| --- | --- |
| `main.ts` | 主窗口：翻译、语言选择、引擎设置、模型市场、历史、主题、更新提示 |
| `popup.ts` | 弹窗：F1/F2 文本翻译与回贴、F3 结果、F4 文件夹命名（Enter/Esc/Delete），含并发与竞态守卫 |
| `snip.ts` | 截图压盖：框选、坐标换算、遮罩绘制 |
| `pin.ts` | 结果贴图：拖动、缩放、翻译覆盖图、快捷键提示 |
| `shared.ts` | 跨窗口共享：主题、系统强调色、倒计时环、通用 DOM 工具 |
| `theme.css` | 全局样式（浅色 / 深色） |

---

## 3. 后端（`src-tauri/src/`）

| 模块 | 职责 |
| --- | --- |
| `main.rs` | 进程入口，调用 `run()` |
| `lib.rs` | 应用装配：注册 command、创建托盘与菜单、管理窗口显示/隐藏、单实例 |
| `constants.rs` | 版本、默认值、语言表（`APP_VERSION`、默认热键、选项列表、语言代码） |
| `settings.rs` | 设置读写与默认值合并，持久化到 `%APPDATA%\Linggo\settings.json` |
| `state.rs` | 全局共享状态（加载中的模型、锁等） |
| `hotkeys.rs` | 全局热键注册 / 更新 / 注销，F1–F5 分发 |
| `clipboard.rs` | 向原窗口发送复制、读取剪贴板、写回 / 粘贴 |
| `explorer.rs` | F4 文件夹创建与资源管理器刷新 |
| `fullscreen.rs` | 前台全屏检测（游戏模式） |
| `ocr.rs` | 调用 `Windows.Media.Ocr` 做本地 OCR |
| `mt_engine.rs` | OPUS / NLLB 推理（CTranslate2），语言检测、引擎选择与缓存 |
| `llama_backend.rs` | GGUF 推理（llama.cpp），模型加载/卸载、闲置回收、worker 线程 |
| `prompt.rs` | 为大模型构造翻译提示词与输出后处理 |
| `segmenter.rs` | 文本分句 / 分段，控制单次推理长度 |
| `pkg_index.rs` | 模型市场：索引获取 / 解析、下载、安装、卸载、路径安全校验 |
| `updater.rs` | 检查 GitHub Releases 版本（仅提示不下载） |
| `win32.rs` | Win32 窗口 / 前台窗口 / 坐标相关封装 |
| `wininet.rs` | 基于 WinINet 的最小 HTTP GET（避免引入额外网络依赖） |
| `winutil.rs` | 杂项 Windows 工具函数 |

---

## 4. 典型调用链

### F1 划词翻译
1. `hotkeys.rs` 捕获 F1。
2. `clipboard.rs` 向当前前台窗口发送 `Ctrl+C` 并读取剪贴板。
3. `mt_engine.rs` / `llama_backend.rs` 按引擎排序翻译。
4. 结果通过 event 广播给 `popup` 窗口展示；写回历史。

### F2 打字翻译并回贴
1. F2 → `popup` 显示输入框。
2. 前端把文本发给后端翻译，返回译文。
3. Enter 时后端把译文写剪贴板并向前台窗口发送粘贴 + 回车。

### F3 截图 OCR 翻译
1. F3 → 抓取全屏并显示 `snip` 窗口。
2. 用户框选 → 前端把区域坐标传回 → 后端裁剪并调用 `ocr.rs` 识别。
3. 识别文本走翻译管线 → `pin` 窗口显示结果，可叠加译文。

### F4 新建文件夹
1. F4 → `popup` 显示命名框。
2. Enter → 翻译名字后 `explorer.rs` 创建目录；Esc 用原文名；Delete 取消。
3. 创建后刷新资源管理器。

---

## 5. 设计要点

- **离线优先**：仅 `updater.rs` 联网，且通过 `wininet.rs` 自实现，减少依赖与体积。
- **内存友好**：大模型 / NLLB 闲置自动卸载；OPUS 常驻。worker 线程每条命令后重新武装闲置计时器。
- **安全**：CSP 严格（见 `tauri.conf.json`）；模型下载对文件名做路径穿越校验（`pkg_index.rs` 的 `safe_file_name`）。
- **静态链接**：`.cargo/config.toml` 使用 `+crt-static`，配合 `vendor/` 中的 llama.cpp 补丁，统一 MSVC 运行库。
