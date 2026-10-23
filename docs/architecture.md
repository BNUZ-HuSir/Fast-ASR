# Architecture 0.3.1

Native Win32 Rust overlay uses WS_EX_NOACTIVATE. Protected Unicode SendInput commits final recognition results; streaming hypotheses stay in the preview. Clipboard, focus binding, deferred output and resize/scroll behavior share the existing UI path. Local JSON settings migrate retired Whisper Small choices to X-ASR without discarding other preferences.

The capture coordinator drains 16 kHz mono WinMM audio every ~8 ms. A separate inference thread owns exactly one recognizer. X-ASR accepts frames continuously using sherpa-onnx 1.13.8 / ONNX Runtime CPU, greedy decoding, 480 ms model chunks, built-in endpoint detection, configurable trailing silence and a 30 s utterance bound. Stop appends two seconds of synthetic zeros and signals input-finished immediately to flush model lookahead; these are not recorded or played. Whisper Turbo uses Silero VAD, 512-sample frames, 320 ms pre-roll, hysteresis, minimum speech duration, configurable trailing silence and a 12 s segment bound.

Capture and inference exchange generation-tagged tasks; cancel invalidates queued output, and a new session resets recognizer state. Queue accounting tracks sample counts, stops capture on overload (10 s online / 90 s offline), and flushes remaining work. Model switching stops and drains the old session before destroying its decoder, then loads the replacement. C++ bridges dynamically resolve local DLL functions; only the selected engine is instantiated. Model files are SHA-256 verified before loading. CPU inference uses up to six threads; no GPU context is created.

Portable layout: runtime/sherpa, runtime/whisper, models/xasr-480ms, models/ggml-large-v3-turbo-q5_0.bin, models/vad, samples, licenses. Full release bundles both models for offline use. Synthetic WAV replay exercises the same coordinator, partial/final delivery, stop, cancel/restart, and model-switch paths without opening a microphone.

The overlay measures all retained text and grows up to 720 logical pixels or monitor work-area bounds, then scrolls. A focus epoch prevents delayed text from entering an unrelated native edit control. Several custom/web editors may share a native HWND, so compatibility with user applications remains a user acceptance test.


The title-bar pin toggles HWND_TOPMOST / HWND_NOTOPMOST with SWP_NOACTIVATE, SWP_NOMOVE and SWP_NOSIZE. Its state is persisted as Settings.topmost; old settings default to pinned. The UI regression clicks both states, checks the actual WS_EX_TOPMOST flag, original input focus and settings roundtrip.

During cancellation the UI ignores partial/final events until the worker acknowledges Stopped, so already-forwarded events cannot repopulate cleared text. Cancel remains enabled during decoding. Generation IDs invalidate pending inference tasks; if a decoder error occurs while switching, the coordinator still advances to the requested model.

Release packaging uses a list populated only by deliberate copies, never a recursive inventory of an existing output directory. Every required diagnostic has to pass and match both the package version and executable SHA-256. Archive integrity, exact inventory and separate release SHA256SUMS are verified after packaging.
