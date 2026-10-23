# 中英文流式 ASR 调研与本机验证

历史调研记录：下文描述接入前的比较；X-ASR 已在 0.3.0 接入，当前使用方法见 README。

日期：2026-10-22。目标：Windows 本地桌面语音输入，中英文及混说，模型文件控制在约 500 MB 以内，优先准确率，同时持续输出。

## 结论

优先将 X-ASR-zh-en INT8 的 480 ms 流式版本作为下一轮集成候选。下载和 CPU WAV 测试已完成，尚未替换桌面程序的 Whisper 引擎。推荐代表适配目标较好，不代表已经证明其总体准确率最高。

X-ASR 官方项目定位为中英双语流式/离线一体模型，采用 Zipformer transducer，提供 160/480/960/1920 ms chunk。这里使用的是 2026-06-05 发布的 sherpa-onnx 转换权重，与项目最初的 2023 年双语 Zipformer 不是同一组权重。

备选：Streaming Paraformer 双语 INT8（官方文件表合计约 226 MiB，即约 237 MB），已有 sherpa-onnx 接口。Qwen3-ASR 0.6B 具有流式接口和社区量化实现，但需要另行验证 Windows CPU 部署、内存和延迟。SenseVoice 的现成接口采用离线识别，可配合 VAD 做分段识别；本次优先选择原生持续流式模型。

## 下载与完整性

- 480 ms 下载包：133,895,136 bytes，SHA256 fa5f63d618e5a01526e275a358bb7772e403f84808a4769fba52cffd8160bf74。
- 960 ms 下载包：133,895,831 bytes，SHA256 0a92b798bd6801c333c7ce8aebf5ba769bfe7f3f3511699a67837b2288428603。
- 以上均与 GitHub Release API 提供的 digest 对照通过。
- 480 ms ONNX 权重合计 169,169,147 bytes；960 ms 为 169,167,082 bytes。另有词表、BPE、测试音频。下载压缩包大小、权重大小与运行内存不是同一指标。

## 本机测试

运行时：现有 sherpa-onnx v1.13.8 Windows x64 CPU，4 线程。主机记录为 i5-12600KF、32 GB RAM。使用 vendor/samples/asr 的六段既有录音，包含中文、英文及中英夹杂。

| 配置 | ONNX 权重 MB（十进制） | 进程工作集峰值 MiB | 私有提交峰值 MiB | 加载秒数 | 单流报告 RTF 范围 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 480 ms greedy | 169.17 | 371.58 | 419.03 | 3.40 | 0.20–0.36 |
| 960 ms greedy | 169.17 | 395.87 | 418.60 | 3.33 | 0.14–0.25 |
| 960 ms beam，4 paths | 169.17 | 396.16 | 418.57 | 2.96 | 0.16–0.28 |

CLI 同时处理六条流，每流 RTF 含并发争用，不是桌面软件单麦克风实时延迟测试。RTF 低于 1 只说明该测试计算速度快于音频长度。上述内存只属于测试进程，不含桌面 GUI。未测试 GPU。

本次正式比较增加 --right-padding=2.0，以便 960 ms 模型在录音结束后完成尾部解码；CLI 默认 0.3 秒补零会导致部分样本尾字尚未解出。2 秒为文件测试合成补零，不是产品设定的收尾等待时间。初次不带 -padded 的 JSON 保留为诊断过程记录；比较应使用 -padded.json。

样本 0：Whisper F16 旧报告含 “Today is Li Ba Ah.”；X-ASR 的 480 ms 与 960 ms 均输出 “昨天是 Monday， today is 礼拜二， the day after tomorrow 是星期三”。这只是同一文件的输出对照，旧 Whisper 的 VAD 分段与新流式解码流程不同，不是严格控制实验。

样本 2：480 ms 输出包含“频繁的”，960 ms 输出包含“平凡的”。本批样本未证明 960 ms 或 beam search 稳定优于 480 ms。样本 46 含“扫脱下”等可疑输出，仍需人工听音确认。

没有独立人工参考转写，因此未计算 CER/WER，也不宣称准确率达标。官方不同项目的评测不直接拼成统一排名，量化版效果需要独立评估。

复现：python scripts/research_streaming_asr.py。报告：reports/x-asr-480ms-int8-greedy_search-padded.json、reports/x-asr-960ms-int8-greedy_search-padded.json、reports/x-asr-960ms-int8-modified_beam_search-padded.json。

## 接入建议

1. Rust 通过 sherpa-onnx C API 创建在线识别器，逐块送入音频；模型常驻，禁止每句话重新加载。
2. 将 partial 和 final 两种结果分开：候选文字实时显示，稳定文字再追加到目标输入框；如果要求输入框也显示候选文字，必须管理自己插入的临时范围并处理焦点变更，避免覆盖用户手动输入。
3. VAD/终点判断负责停顿收尾与噪声门控，不在短暂停顿时频繁重置识别上下文；结束时正确 flush，不丢末尾词。
4. 当前先保留现有程序作为回退，完成真实麦克风与中英夹杂评估后再确定默认模型。

## 一手来源

- X-ASR 项目与评测：https://github.com/Gilgamesh-J/X-ASR
- 中文模型说明：https://github.com/Gilgamesh-J/X-ASR/blob/main/README_zh.md
- sherpa-onnx 模型发布：https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models
- 发布资产与 SHA：https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/tags/asr-models
- Streaming Paraformer：https://k2-fsa.github.io/sherpa/onnx/pretrained_models/online-paraformer/paraformer-models.html
- Qwen3-ASR 官方：https://github.com/QwenLM/Qwen3-ASR
- Qwen 社区 CPU 实现：https://github.com/antirez/qwen-asr
- SenseVoice 部署：https://k2-fsa.github.io/sherpa/onnx/sense-voice/index.html
