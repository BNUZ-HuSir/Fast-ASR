# 0.3.1 发布前评审

评审日期：2026-10-23。目标账号：BNUZ-HuSir。建议仓库名：Fast-ASR。范围包括 Rust/Win32 UI、C++ FFI、录音和识别线程、输入焦点、模型分发、许可证、源码仓库与离线包。

## 发现并修复

| 优先级 | 问题 | 处理与验证 |
| --- | --- | --- |
| P1 | stop() 在 busy 时直接返回，识别收尾阶段按 Esc 或清空不能取消，文字可能重新出现 | 允许处理阶段取消；UI 在 Stopped 确认前丢弃候选/最终事件，worker 用 generation 丢弃旧任务；新增 busy 状态桌面回归，WAV 管线另覆盖候选后取消 |
| P1 | 首次提交规则没有排除 target/、第三方源码、旧桥接中间文件 | 精确的 .gitignore 与源代码包清单，排除缓存/模型/用户设置/日志；审核实际待提交文件 |
| P1 | 旧打包脚本依赖机器上的旧报告，递归收集输出目录可能误收其他文件 | 测试报告绑定程序版本和 SHA-256，打包使用明确复制清单，压缩包逐文件校验 |
| P2 | 模型切换过程中识别失败后，待切换状态可能残留，后续开始请求被拒绝 | 错误分支继续执行已请求的模型切换，保留错误提示 |
| 发布准备 | 项目没有顶层许可证、贡献说明和自动构建配置 | 新增 MIT 许可证草案、上游声明、贡献/隐私说明、问题模板和只读权限 Windows 工作流 |

用户要求的置顶按钮通过 SetWindowPos 切换实际置顶标志并保存设置。已有版本仍默认置顶，新增按钮可以取消，操作不抢焦点。

## 验收依据

运行 scripts/build.ps1、scripts/verify.ps1 -IncludeUI、scripts/package.py --require-ui 与 scripts/validate_package.py。验证命令检查报告是否来自当前程序，不能复用其他版本结果。WAV 自检覆盖六段中文/英文/混说与静音；管线覆盖候选、最终结果、取消重启和 X-ASR → Turbo → X-ASR。桌面回归覆盖置顶开关、焦点保持、设置保存、复制后填写、换框保护和长文本；模拟 UI busy 状态检查取消和清空。推理错误时继续模型切换的恢复分支尚未专门故障注入。

原始功能报告位于工作区 reports/ 与离线包 docs/test-results/；构建与交付检查记录见 docs/validation.md 和对应 reports/ JSON。源码包另从独立目录构建，用固定的模型/运行库归档作为缓存，检查所需文件能否补齐。

本轮结果：Rust 格式检查、两个单元测试、Clippy、五份当前程序功能报告全部通过；独立源码目录重新生成 Whisper 运行库与 Rust/C++ 主程序，X-ASR 六段 WAV 与静音自检通过。源码提交清单约 2 MB，与源代码压缩包使用同一明确清单，未包含模型、构建缓存或用户设置。

便携包 CRC、逐文件哈希、源码清单、两套模型和 SHA256SUMS 检查通过；中文路径运行的识别管线与 Unicode 输入通过，UI 回归在第三次执行通过。前两次桌面检查存在焦点/恢复绑定失败，已在验证记录保留，尚不足以证明交互式自测在活跃桌面上的稳定性。

## 发布判断与已知限制

适合先作为 Windows Preview 小范围发布。现有证据支持功能流程通过，不能承诺所有编辑器兼容或识别准确率。没有人工参考集 CER/WER、跨麦克风/口音评测或长期运行统计。Turbo 在本机 CPU 上明显慢于实时，默认保持 X-ASR。

程序目前未签名；高权限程序可能拒绝模拟输入；网页/自定义控件共享 HWND 时不能精确区分输入字段，非标准密码框也可能无法被识别。已在使用和隐私文档写明。麦克风真实讲话由用户主动开启后验收，本轮自动测试不打开麦克风。

## GitHub 发布状态

首次评审时，账号公开资料已核对为 BNUZ-HuSir，本地仓库尚无提交或 remote，gh auth status 显示该账号凭据无效。未创建公开仓库、未推送、未上传 Release；本地材料按 Fast-ASR / MIT / v0.3.1 Preview 准备，待维护者确认公开发布并恢复登录后执行。GitHub 工作流已经编写，但尚未在 GitHub runner 上执行，不能宣称远端 CI 已通过。

## 技术核对来源

- Microsoft SetWindowPos： https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos
- X-ASR 上游与许可： https://github.com/Gilgamesh-J/X-ASR
- Whisper Turbo 模型卡： https://huggingface.co/openai/whisper-large-v3-turbo
- sherpa-onnx 固定运行库： https://github.com/k2-fsa/sherpa-onnx/releases/tag/v1.13.8
- GitHub Actions checkout / upload-artifact 官方仓库，工作流使用查证后的完整提交 SHA。

## 源码上传准备补充

新增 `scripts/download_models.py`，默认仅准备 X-ASR，支持选择 Turbo / 全部、便携目录和只校验；复用固定来源、断点续传和 SHA-256 校验。权重、运行库、下载归档与模型临时文件均由 Git 忽略规则排除。CI 只上传源码压缩包和验证报告，不上传包含模型的便携包。模型脚本已经验证已有文件复用、缓存归档首次安装、损坏小文件拒绝/保留/恢复和两模型便携布局；此次未重新从网络下载完整权重。
