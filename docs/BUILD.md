# 从源码构建

Linggo 由 **前端（TypeScript + Vite）** 与 **Rust 后端（Tauri 2）** 组成。Rust 侧内嵌 `llama.cpp` / `CTranslate2`，因此首次编译较慢（视机器约 10–40 分钟）。

---

## 1. 环境要求

| 组件 | 版本 / 说明 |
| --- | --- |
| Windows | 10 1809+ / 11，**x64** |
| Rust | stable，工具链 `stable-x86_64-pc-windows-msvc` |
| Visual Studio | 2022 **生成工具**，勾选「使用 C++ 的桌面开发」（MSVC v143 + Windows SDK） |
| CMake | 3.20+，需加入 `PATH`（编译 llama.cpp / oneDNN 用） |
| Node.js | 18+（含 npm） |
| WebView2 运行时 | Win11 自带；Win10 通常随 Edge 安装。运行 exe 必需，编译不需要 |
| Inno Setup 6 | 仅打安装包时需要（`ISCC.exe`） |

安装 Rust（若未装）：

```powershell
winget install Rustlang.Rustup
rustup default stable-msvc
```

验证：

```powershell
rustc -V ; cargo -V ; cmake --version ; node -v ; npm -v
```

---

## 2. 编译步骤

```powershell
# 在仓库根目录
npm install
npm run build          # tsc + vite build → 生成 dist\

cd src-tauri
cargo build --release --features custom-protocol
```

产物：`src-tauri\target\release\linggo.exe`。

### 关键注意

1. **必须带 `--features custom-protocol`**。没有该特性时 Tauri 走 dev 服务器地址，独立运行会**白屏**。
2. **不要删除 `src-tauri\vendor\`**。`Cargo.toml` 通过 `[patch.crates-io]` 指向本地补丁：
   - `vendor/crates/llama-cpp-2`
   - `vendor/crates/llama-cpp-sys-2`（内嵌完整 llama.cpp 源码）

   该补丁修正了 MSVC 运行库链接（`/MT`），与 CTranslate2 静态运行库保持一致；缺少它会链接失败。
3. **不要删除 `src-tauri\.cargo\config.toml`**，其中设置 `-C target-feature=+crt-static` 静态链接 C 运行时。
4. `src-tauri\gen\` 由 Tauri 构建时自动生成（能力 schema），无需入库。

### 运行

```powershell
# 开发模式（热重载前端）
npm run tauri dev

# 直接运行 release 版
.\src-tauri\target\release\linggo.exe
```

首次运行仍需在设置中选择 / 下载模型（见 [MODELS.md](MODELS.md)）。

---

## 3. 打包安装包（Inno Setup）

`Linggo.iss` 使用 Inno Setup 6，读取 `src-tauri\target\release\Linggo.exe` 作为主程序，默认输出到**仓库上一级的 `发布包\` 目录**（不污染源码目录）。

```powershell
# 1. cargo 产物名为 linggo.exe，复制为 Linggo.exe（.iss 按此名字查找）
Copy-Item .\src-tauri\target\release\linggo.exe .\src-tauri\target\release\Linggo.exe -Force

# 2. 编译脚本（ISCC 在 Inno Setup 6 安装目录）
& "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" .\Linggo.iss
# 产物：..\发布包\Linggo-Setup-<版本>.exe
```

安装向导附带两个自定义页面：

- **翻译语言 / Translation languages**：选择首选语言与次选语言（校验两者不能相同）。
- **偏好设置 / Preferences**：闲置释放、历史条数、外观主题，以及游戏模式、快速引擎、回退大模型、贴图覆盖图、贴图提示等开关。

上述偏好写入 `%APPDATA%\Linggo\settings.json`；若该文件已存在且未勾选「重新安装时覆盖已有设置」，则保留原设置。安装包图标取自 `src-tauri\icons\icon.ico`。

> 也可改用 Tauri 自带打包：`npm run tauri build`（会调用 NSIS/WiX，需要联网下载打包器）。本项目默认使用 Inno Setup 方案。

---

## 4. 依赖与离线说明

- 正常联网环境下 `cargo build` 会自动拉取 crates.io 依赖；首次编译会下载并编译 `llama.cpp`、`oneDNN` 等，耗时较长。
- `src-tauri\vendor\` 只包含被 `[patch.crates-io]` 覆盖的两个 crate 的**源码**，其余依赖仍需从 crates.io 获取。
- 构建产物不包含模型；模型下载见 [MODELS.md](MODELS.md)。

---

## 5. 常见编译问题

| 现象 | 原因 / 解决 |
| --- | --- |
| 运行后白屏 | 漏了 `--features custom-protocol` |
| 链接错误（LNK / 运行库冲突） | 删了 `vendor/` 或 `.cargo/config.toml`，恢复即可 |
| `cmake not found` | 安装 CMake 并加入 `PATH` |
| `link.exe not found` / 缺 Windows SDK | 安装 VS 2022 生成工具并勾选 C++ 桌面开发 |
| 编译卡在 `llama.cpp` / oneDNN | 正常，首次编译最慢；保证磁盘与内存充足 |
| 前端资源 404 / 旧界面 | 先在根目录 `npm run build`，再重新 `cargo build`（`build.rs` 会重新内嵌 `dist`） |

更多问题见 [FAQ.md](FAQ.md)。
