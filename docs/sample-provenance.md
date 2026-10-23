# 诊断 WAV 来源记录

记录日期：2026-10-23。以下文件用于 WAV 流程回归，不作为识别准确率基准，不用于模型训练或宣传。

这些录音由上游测试资源提供。上游代码项目分别公开 Apache-2.0 / MIT 许可，但本项目没有独立核验每段原始录音的全部内容、录音者或声音相关权利，不能仅凭代码许可证推定所有录音权益均已授权。再利用录音时应单独核实相应用途。

已逐一比对本地上游解压资源与当前测试 WAV 的内容哈希，文件内容一致。未将原录音内容改写为本项目作品。具体争议按 LEGAL.md 的权利反馈流程核查；本记录不提供对录音的额外授权。

| 本地文件 | SHA-256 | 上游位置 |
| --- | --- | --- |
| `vendor/samples/asr/0.wav` | `7d93384ca14702cc584a7a33fe2fed92e89e708549161cb12ea38c916882103b` | https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-small-bilingual-zh-en-2023-02-16.tar.bz2 (`test_wavs/0.wav`) |
| `vendor/samples/asr/1.wav` | `8bfb42c963e623ebab31b81ff4404867d07d3102507c87ac14577c4c61663b8c` | https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-small-bilingual-zh-en-2023-02-16.tar.bz2 (`test_wavs/1.wav`) |
| `vendor/samples/asr/2.wav` | `20805bcc696b9b65f3357a2508a419d4f33b81006d47e1312e219b0fad934d1d` | https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-small-bilingual-zh-en-2023-02-16.tar.bz2 (`test_wavs/2.wav`) |
| `vendor/samples/asr/3.wav` | `5851e31f3e8e5f741635da9466e19fccfbabd117ebcdc92db242c92483aee064` | https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-small-bilingual-zh-en-2023-02-16.tar.bz2 (`test_wavs/3.wav`) |
| `vendor/samples/asr/46.wav` | `be53e027a0c562b882c8c10806b1861c2cda0dc417a786d15a0d9d8de3d85dda` | https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-small-bilingual-zh-en-2023-02-16.tar.bz2 (`test_wavs/46.wav`) |
| `vendor/samples/asr/english-jfk.wav` | `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e` | https://github.com/ggml-org/whisper.cpp/blob/d1be6fde11ac6e0407606b4e42fe72d34add8037/samples/jfk.wav (`samples/jfk.wav`) |
