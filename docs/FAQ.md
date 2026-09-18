# 常见问题（FAQ）

## 使用

### 按 F1 没有反应 / 抓不到文字？
- 先用鼠标**选中**文本再按 F1。
- Linggo 通过发送 `Ctrl+C` 抓取选区，目标程序必须支持复制。以下场景会失败，请改用 **F2**：
  - 终端（部分终端复制键不是 `Ctrl+C`）；
  - 远程桌面 / 虚拟机；
  - 带反作弊或屏蔽热键的程序（多为游戏）。
- 若热键被别的软件占用（如截图、录屏工具），到「设置 → 全局热键」改成其他组合。

### F1/F2 在游戏里没反应？
「设置 → 游戏模式」默认会在**前台为全屏游戏时禁用热键**，以防误触。需要时可取消勾选。

### 提示「模型未加载」？
打开 **主窗口 (F5) → 设置**：
- 用大模型：在「模型」里选择 `.gguf` 并「加载」；
- 用快速引擎：在「模型市场」下载 OPUS / NLLB 语言包。
参考 [MODELS.md](MODELS.md)。

### 翻译很慢 / 内存占用高？
- 中英互译优先走 OPUS（最快）；GGUF 质量好但更吃资源。
- 让「设置 → 通用 → 闲置释放」保持开启（默认 15 秒自动卸载大模型）。
- NLLB 约占用 1.3 GB，可点「释放 NMT 内存」。

### F4 为什么有时用译名、有时用原名？
这是设计行为：`Enter` 用译文名，`Esc` 用原始输入名，`Delete` 取消。见 [USAGE.md](USAGE.md#f4--新建文件夹)。

### 关闭主窗口后程序还在？
Linggo 常驻托盘。左键单击托盘图标恢复窗口，右键菜单「退出」彻底关闭。

### 如何恢复出厂设置？
删除 `%APPDATA%\Linggo` 目录（会丢失设置与历史）。

### 翻译内容会被上传吗？
不会。翻译与 OCR 全部在本机完成。只有「检查更新」会访问 GitHub，且不会下载，可在设置中关闭。

---

## 构建 / 开发

### 运行后白屏？
编译时漏了 `--features custom-protocol`：

```powershell
cargo build --release --features custom-protocol
```

该特性会把前端资源内嵌进可执行文件；不带则程序去连开发服务器地址，导致白屏。

### 链接错误（LNK / 运行库冲突）？
多半是删除了编译必需文件：
- `src-tauri/vendor/`（`llama-cpp-2` / `llama-cpp-sys-2` 本地补丁）
- `src-tauri/.cargo/config.toml`（`+crt-static`）

请恢复后再编译。详见 [BUILD.md](BUILD.md)。

### `cmake not found` / `link.exe not found`？
安装 CMake（加入 `PATH`）与 Visual Studio 2022 生成工具（勾选「使用 C++ 的桌面开发」）。

### 编译很久都卡在 llama.cpp / oneDNN？
首次编译需从源码构建这些 C/C++ 组件，属正常现象，视机器可能需要 10–40 分钟。请保证磁盘空间与内存充足。

### 改了前端代码但界面没变？
先在根目录执行 `npm run build` 生成 `dist/`，再重新 `cargo build`（`build.rs` 会重新内嵌资源）。

### 能否用 `npm run tauri build` 打包？
可以。项目同时提供 Inno Setup 方案（`Linggo.iss`）。注意 Tauri 自带打包可能需联网下载 NSIS/WiX。自定义方案见 [BUILD.md](BUILD.md#3-打包安装包inno-setup)。

---

## 安装 / 运行

### 提示缺少 WebView2？
Windows 11 自带；Windows 10 请安装 [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)（通常随 Edge 已存在）。

### 杀毒软件报毒 / SmartScreen 拦截？
未签名的自编译 exe 可能触发 SmartScreen，点「更多信息 → 仍要运行」即可。若安全软件误报，请加入信任区。正式分发建议自行代码签名。

### 支持哪些系统？
Windows 10 1809+ / Windows 11，**仅 x64**。不支持 32 位与 ARM（未构建对应产物）。

### 开机自启怎么关？
安装时可取消勾选，或安装后在「设置 → 通用 → 开机自启」取消。
