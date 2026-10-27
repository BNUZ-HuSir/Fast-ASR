# Third-party notices / 第三方资源声明

Fast ASR 是个人学习、开发实验与非营利技术分享项目。应用原创源码适用根目录 `LICENSE`（MIT）；这项用途说明不额外禁止商用，也不更改任何上游许可。项目用途、识别结果核对、责任边界及权利反馈见 `docs/LEGAL.md`。

0.3.5 实际运行与便携包只包含 X-ASR、sherpa-onnx 和 ONNX Runtime。下表 Whisper / ggml / Silero 条目及其材料为历史版本的来源留档，本版不加载或打包这些权重与运行库。留存历史授权材料不构成新增运行依赖。

## 模型与原生运行库

| 资源与作者/维护方 | 使用的版本或文件 | 公开许可证 | 项目或原始模型来源 |
| --- | --- | --- | --- |
| X-ASR 作者，GitHub: Gilgamesh-J / Hugging Face: GilgameshWind；ONNX INT8 包由 k2-fsa/sherpa-onnx 分发 | X-ASR-zh-en 480 ms punct INT8，包日期 2026-06-05 | Apache-2.0 | https://github.com/Gilgamesh-J/X-ASR / https://huggingface.co/GilgameshWind/X-ASR-zh-en |
| sherpa-onnx contributors（含 Xiaomi Corporation；原始逐文件署名以其源码为准） | sherpa-onnx 1.13.8 | Apache-2.0 | https://github.com/k2-fsa/sherpa-onnx |
| Microsoft Corporation 及其第三方组件作者 | ONNX Runtime 1.28.2，由上述 sherpa-onnx 运行库包携带 | MIT；内部组件各适用其原有条款 | https://github.com/microsoft/onnxruntime |
| The ggml authors | whisper.cpp / ggml 1.9.5，固定源码本地构建 AVX2 CPU DLL | MIT | https://github.com/ggml-org/whisper.cpp |
| OpenAI；ggml 转换文件由 ggerganov/whisper.cpp 分发 | Whisper Large-v3-Turbo Q5_0，固定 revision `5359861c739e955e79d9a303bcbc70fb988958b1` | MIT | https://huggingface.co/openai/whisper-large-v3-turbo / https://huggingface.co/ggerganov/whisper.cpp |
| Silero Team；ggml 转换文件由 ggml-org/whisper-vad 分发 | Silero VAD 5.1.2，固定 revision `9ffd54a1e1ee413ddf265af9913beaf518d1639b` | MIT | https://github.com/snakers4/silero-vad / https://huggingface.co/ggml-org/whisper-vad |

源码仓库不提交模型权重或运行库。实际模型下载 URL、文件大小和 SHA-256 在 `assets/xasr-manifest.json`、`assets/whisper-manifest.json` 与准备脚本中固定。上游 ONNX/ggml 量化和转换文件直接使用，不是本项目训练的模型；本项目没有改动清单中的权重内容。应用集成和本地构建运行库不表示上游为本项目提供官方支持或背书。

## 许可证、模型卡和通知文件

源码中的原文位于 `vendor/licenses/`，完整发布包中位于 `licenses/`。下载模型时也在目标目录保留 `licenses/`、本声明及 `docs/LEGAL.md`。

- `X-ASR-LICENSE`、`X-ASR-MODEL-CARD.txt` 与 `X-ASR-ARCHIVE-README.txt`：原始项目 Apache-2.0 许可、作者模型卡以及实际 INT8 归档附带说明。作者模型卡声明的是模型许可；不只以推理框架许可推断模型许可。
- `sherpa-onnx-LICENSE`：固定版本 Apache-2.0 原文。
- `onnxruntime-LICENSE`、`onnxruntime-ThirdPartyNotices.txt`：Microsoft MIT 许可及该版本完整第三方声明；后者不翻译或删改。
- `whisper-model-LICENSE`、`whisper-original-MODEL-CARD.txt` 与 `whisper-quantized-MODEL-CARD.txt`：OpenAI MIT 原文、原始模型卡与实际转换分发仓库的模型卡。
- `whisper.cpp-LICENSE`：固定构建源码的 MIT 原文，适用于所用 whisper.cpp / ggml 源码。
- `silero-vad-LICENSE` 与 `silero-converted-MODEL-CARD.txt`：Silero 5.1.2 MIT 原文与实际 ggml 分发仓库的模型卡。
- `SOURCES.json`：以上原文的来源、版本依据、哈希和核对范围。模型卡为上游快照，其中性能描述不代表本项目已验证。

Apache-2.0 通用文本附录中的占位符是上游原文，不擅自替换为本项目名字。该表补充资源来源，不冒充作者声明。2026-10-23 核对的 X-ASR 与 sherpa-onnx 根目录没有找到 `NOTICE`，固定下载归档也没有 LICENSE/NOTICE；因此额外保留作者授权声明及许可证。这项检查不表示所有上游源码子目录均无其他通知；新增资源或升级时需重新检查。

## Rust 依赖与诊断资源

完整包的 `licenses/rust-dependencies.json` 列出 Cargo 已解析依赖及其声明的许可证，`licenses/rust/` 携带可获取的上游原文。清单可包含非 Windows 目标的条件依赖，不表示每个条目都进入 Windows 可执行文件。每个发布包不可变文件的 SHA-256 记录在 `build-info.json`。这些清单是材料记录，不是完整知识产权审计结论。

诊断 WAV 的具体上游位置、文件哈希及核对限制见 `docs/sample-provenance.md`。原始录音的权利不能仅凭代码许可证推定；本项目没有独立核验其全部原始权利链，不承诺对录音的声音、文本或人物身份拥有独立授权。

本项目与上述作者或机构无官方关联，不使用其名义担保识别准确性或法律合规性。各资源仍适用各自的原始许可及适用法律；本项目 MIT 许可不替代它们。
