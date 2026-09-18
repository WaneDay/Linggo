# 贡献指南

感谢你对 Linggo 感兴趣！欢迎提交 Issue 与 Pull Request。

---

## 开发环境

见 [BUILD.md](BUILD.md)。最小步骤：

```powershell
npm install
npm run build
cd src-tauri
cargo build --release --features custom-protocol
```

开发时可用：

```powershell
npm run tauri dev
```

---

## 提交前检查

1. **前端可构建**：`npm run build`
2. **Rust 测试通过**：`cd src-tauri; cargo test --lib`
3. **Release 可编译**：`cargo build --release --features custom-protocol`
4. 若改动了影响运行时的逻辑，建议实际运行 `linggo.exe` 做冒烟验证。

---

## 代码约定

- **语言与风格**：Rust 使用 `rustfmt` 默认风格；TypeScript 与现有文件保持一致（2 空格缩进、无分号或按现有风格）。
- **注释**：仅在必要处写注释，说明「为什么」而非「是什么」；不添加无意义注释。
- **前端**：复用 `shared.ts` 中的工具；新增窗口需同步更新 `tauri.conf.json` 与 `capabilities/`。
- **命令**：Tauri 命令参数上不要写 `///` 文档注释（会破坏宏解析）。
- **托盘事件**：使用 `app.on_menu_event` / `app.on_tray_icon_event`，不要依赖全局 handler。
- **安全**：任何源自网络的文件名 / 路径都必须做合法性校验（参考 `pkg_index.rs` 的 `safe_file_name`）。不要引入会外发的网络请求。

---

## 不要提交

- `node_modules/`、`dist/`、`src-tauri/target/`、`models/`、`*.gguf`、`install/`（已在 `.gitignore`）。
- 新增第三方依赖前请说明理由；本项目刻意保持体积与依赖精简。
- 大文件与模型一律走 Releases / 模型市场，不入库。

---

## 提交流程

1. Fork 本仓库，从默认分支拉出特性分支：`feat/xxx` 或 `fix/xxx`。
2. 保持提交信息简洁清晰（可用中文或英文），一个提交聚焦一件事。
3. 发起 PR，说明：**做了什么、为什么、如何验证**，并附上关键截图 / 日志。
4. 确保上述「提交前检查」全部通过。

---

## 报告问题

请在 Issue 中提供：

- Linggo 版本（设置 → 关于，或主窗口底部版本号）；
- Windows 版本、是否首次安装 / 源码构建；
- 复现步骤、期望结果与实际结果；
- 相关日志或截图（注意隐去隐私内容）。

---

## 许可证

向本仓库贡献代码即表示你同意以 [MIT License](../LICENSE) 授权你的贡献。
