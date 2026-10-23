# Contributing

感谢帮助改进 Fast ASR。当前主要支持 Windows 10/11 x64 与中英文语音输入。

## 模型下载

模型权重不提交 Git。`python scripts/download_models.py` 默认下载并校验 X-ASR；`--model turbo` 准备 Turbo 与 VAD，`--model all` 准备两套，`--verify-only` 只检查文件。该入口不编译运行库。完整源码构建仍使用下方命令。

## 构建

安装 Rust MSVC（本机验证 1.96.1）、Visual Studio 2022 C++ 桌面开发组件及 Windows SDK、CMake，以及 Python 3.9+。运行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build.ps1 -PrepareAssets
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1 -IncludeUI
python scripts/package.py --require-ui
python scripts/validate_package.py
```

首次准备依赖需要联网并下载模型，之后正常语音输入在本地完成。依赖版本和哈希固定在 assets/ 与准备脚本中。所有构建输出位于 .build/，应用包位于 dist/。不需要手动提交 vendor/runtime、模型、缓存或编译结果。

IncludeUI 会短暂创建自测输入框、模拟键盘输入并覆盖剪贴板，请在没有其他输入操作时运行。自测输入仅写入它创建的测试控件，不打开麦克风。CI 使用不含 UI 的 WAV 回放检查，实际桌面测试仍需本机执行。

## 提交问题

提供版本、Windows 版本、CPU、当前模型、重现步骤，以及属于识别错误还是输入框兼容问题。日志请去除个人内容；无需上传真实对话录音。若希望提交语音样本，请确认有权公开，并提供对应文本和采样率。

代码改动应保持原生轻量界面、手动启用录音、焦点保护和取消语义。涉及音频线程或模型切换的改动需要重新运行 WAV 管线；涉及界面或输入的改动需要运行桌面回归。提交前运行格式检查、测试与 Clippy。
