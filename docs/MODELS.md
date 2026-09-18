# 模型与引擎

Linggo 本体不含模型。所有模型都在本机运行，**不会上传任何内容**。本文说明三种引擎的差异、模型来源与安装方式。

---

## 1. 三种引擎

| 引擎 | 底层 | 适用场景 | 单包体积 | 内存行为 |
| --- | --- | --- | --- | --- |
| **OPUS** | CTranslate2 上的 OPUS-MT 神经网络翻译 | 中 ↔ 英（及中↔部分欧洲语言） | ~80–160 MB / 方向 | 加载后常驻缓存，速度最快 |
| **NLLB** | CTranslate2 上的 NLLB-200 distilled 1.3B | 多语种互译 | ~1.33 GB（`model.bin` + tokenizer） | 闲置自动回收（默认 15 秒，可调） |
| **GGUF** | llama.cpp 运行 Hy-MT2-1.8B Q4_K_M | 任意语种，强调译文质量 / 上下文 | ~1.08 GB | 闲置自动卸载 |

F1–F4 会按「设置 → 引擎排序」依次尝试（默认 OPUS → NLLB → GGUF），某个引擎不支持当前语对时自动跳到下一个；「NMT 不可用时回退大模型」决定是否最终交给 GGUF。F5 主窗口默认固定用 GGUF，可单独修改。

---

## 2. 通过模型市场安装（推荐）

主窗口 → **设置 → 模型市场**：

1. 「索引地址」留空使用**内置索引**（仓库中的 `src-tauri/pkg_index.json`），也可填入自定义索引地址后「刷新列表」。
2. 在「可用语言包」里找到需要的包，点「下载」，自动安装到可执行文件旁的 `models\` 目录。
3. 「已安装」区域可直接删除卸载。

内置索引包含：

- `hy-mt2-1.8b-q4-k-m-gguf` — Hy-MT2-1.8B Q4_K_M（GGUF）
- `nllb-200-ct2-1.3b` — NLLB-200 1.3B（CT2）
- `opus-mt-zh-en-ct2` / `opus-mt-en-zh-ct2` — 中 ↔ 英
- `opus-mt-zh-{de,fi,he,it,ms,nl,sv,uk,vi,bg}-ct2` 及对应 `*-zh` 反向语言包

> 下载地址使用 `hf-mirror.com` 镜像。若你所在网络可直接访问 Hugging Face，也可手动下载（见下）。

---

## 3. 手动安装

### GGUF 大模型
把 `.gguf` 文件放到可执行文件旁的 `models\` 目录，然后在
**设置 → 模型 → 模型文件**「浏览…」选中它，点「加载」。

### OPUS / NLLB
这两种引擎需要一个**包含模型的目录**，目录内应含对应文件：

| 引擎 | 目录内必需文件 |
| --- | --- |
| OPUS（CTranslate2） | `model.bin`、`config.json`、`source.spm`、`target.spm`（部分包另含 `shared_vocabulary.json`） |
| NLLB（CTranslate2） | `model.bin`、`tokenizer.json` |

在「设置 → 快速翻译引擎」中，把对应输入框指向这些目录：

- `OPUS zh→en` / `OPUS en→zh` — 指定各自语言包目录；
- `NLLB 模型目录` — 选择含 `model.bin` 的目录；
- 留空则自动在 `models\` 下发现。

### 手动下载来源（内置索引同源）

| 模型 | 链接 |
| --- | --- |
| Hy-MT2-1.8B GGUF | https://hf-mirror.com/tencent/Hy-MT2-1.8B-GGUF |
| NLLB-200 1.3B CT2 | https://hf-mirror.com/michaelfeil/ct2fast-nllb-200-distilled-1.3B |
| OPUS zh→en | https://hf-mirror.com/gaudi/opus-mt-zh-en-ctranslate2 |
| OPUS en→zh | https://hf-mirror.com/gaudi/opus-mt-en-zh-ctranslate2 |

> NLLB 若使用其它来源，需保证是 **CTranslate2 格式**（不是原始 Hugging Face Transformers 权重）。

---

## 4. 目录与内存

- 默认安装位置：可执行文件旁 `models\`（安装版即安装目录下）。
- **闲置释放**（设置 → 通用）：大模型和 NLLB 在无操作一段时间后自动卸载以释放内存，可选 `5 / 15 / 30 秒` 或「不释放」。默认 15 秒。
- 「释放 NMT 内存」按钮可立即回收 NLLB（约 1.3 GB）；OPUS 体积小，加载后常驻缓存。

---

## 5. 自定义模型市场索引

索引是一个 JSON 数组，字段如下：

```json
[
  {
    "id": "my-opus-zh-en",
    "kind": "opus",
    "name": "OPUS-MT 中→英（自建）",
    "fromCode": "zh",
    "toCode": "en",
    "baseUrl": "https://example.com/models/opus-zh-en",
    "files": ["config.json", "model.bin", "source.spm", "target.spm"],
    "sizeBytes": 156000000,
    "version": "1.0"
  }
]
```

- `kind`：`opus` / `nllb` / `gguf`。
- `baseUrl`：文件所在目录，下载时拼接文件名。
- `files`：需要下载的文件列表。
- 本地自建索引可直接填 `file:///` 或指向本地 HTTP 服务。

改完把索引地址填入设置即可。
