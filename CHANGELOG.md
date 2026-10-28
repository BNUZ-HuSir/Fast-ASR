# Changelog

## 0.3.6 Preview — 2026-10-28

- 默认录音手势改为单独左 Alt 长按 0.5 秒，长按期间流式预览，松开填写。右 Alt / AltGr 与 Ctrl 不触发。
- 短按 Alt 恢复原生按键，长按录音不激活菜单；Alt 组合键恢复原组合并取消尚未提交的草稿。
- 新增 Alt+鼠标按下和滚轮转发，保留修饰键与事件顺序；单纯移动鼠标不打断。其他键或鼠标按钮已按住时不触发长按。
- 录音已触发时 Alt+Esc 用于取消，不执行原生切窗；触发前保留原组合。说明兼容边界。
- 键鼠钩子与输入重放移到独立线程，退出等待设限；移除诊断中的跨应用输入队列绑定。
- 新增私有桌面的真实钩子安装/卸载和原生进程退出检查；本次交付标记为 Preview，真实桌面快捷键、自动填写和 UI 回归仍待完成。

## 0.3.5 — 2026-10-27

- Only X-ASR-zh-en 480 ms INT8 is active and bundled; remove Whisper and Silero runtime dependencies from the application and default setup.
- Feed audio directly into X-ASR with native endpoint detection; retain streaming previews during held recording and commit on release.
- Enable Ctrl hold (300 ms) for first use, retaining explicit existing shortcut preferences and migrating retired model IDs.
- Simplify settings to microphone and shortcut, add developer credit Elza, and give the pin icon a rounded, accent-selected button.
- Verify multi-utterance endpoints, silence, release/cancel/restart, focus, clipboard recovery and simplified settings; shrink the offline package.


## 0.3.4 — 2026-10-27

- 长按录音改为单独按住左/右 Ctrl 0.3 秒触发，松开后填写；短按与 Ctrl 组合键转发，组合键或 Esc 取消本次草稿。
- 修正长按模式先缓存整段、松开才识别的行为：X-ASR 在长按期间实时经过 VAD 与流式 ASR，预览与填写时机分离。
- 录音时显示 VAD 等待/检测状态、实时语音概率及配置阈值；点击和长按共用 VAD，未添加后台常驻监听。
- 增加长按草稿预览/提交/取消界面回归，以及原生 Silero 阈值、最短语音和停顿时间的音频对照测试。

## 0.3.3 — 2026-10-26

- 按住录音改为单独长按 Alt 0.5 秒后触发，松开 Alt 再做 VAD / ASR 并填写。
- 短按 Alt 转发为正常菜单按键；Alt 组合键转发原组合，已开始的长按录音取消，避免误提交。
- 保留 Ctrl+Alt+空格的按一次切换模式；快捷键仍默认关闭。
- 新增 6 项 Alt 状态测试和真实 Windows 键盘钩子诊断，验证触发阈值、单次启动、松开、组合键转发及 Esc 取消。

## 0.3.2 — 2026-10-26

- 置顶按钮改为图钉图标，保留开关状态和不抢输入焦点的行为。
- 就绪时麦克风按钮为绿色，录音时切换为红色停止图标，处理时显示等待图标。
- 两种模型统一使用 Silero VAD，设置开放语音阈值（0.2–0.8）、最短语音（64–512 ms）及停顿时间。
- 快捷键增加“按住录音，松开识别”：按住 Ctrl+Alt+空格录音，松开任一键才提交识别并填写，单次最长 60 秒；Esc 取消。
- 保留鼠标启动实时听写及快捷键按一次切换；旧版已启用的快捷键仍使用切换模式。
- 保持共享 VAD 运行库驻留，避免模型切换时 ggml 重复初始化崩溃；清除切换前的长按状态。
- 模型下载脚本为 X-ASR 和 Turbo 都准备共享 VAD（小于 1 MB），并继续保留上游授权材料。

## 0.3.1 — 2026-10-23

- 标题栏增加“置顶 / 已置顶”按钮，切换不抢输入焦点，重启保留选择。
- 修复识别收尾阶段按 Esc 或清空无法取消，以及取消前已排队的文字可能重新出现的问题。
- 增强模型切换期间发生识别错误后的恢复。
- 增加许可证、贡献说明、问题模板和 Windows 构建验证工作流。
- 打包使用明确的文件清单，测试报告绑定程序 SHA-256，提供压缩包 SHA256SUMS。

## 0.3.0 — 2026-10-22

- 默认使用 X-ASR-zh-en INT8 480 ms 流式识别，候选实时预览、停顿后填写。
- 可切换 Whisper Large-v3-Turbo Q5_0，随完整离线包提供。
- 旧 Whisper Small 退出模型列表，旧设置迁移为 X-ASR。
- 保留焦点保护、复制后继续填写和长文本增高/滚动。
