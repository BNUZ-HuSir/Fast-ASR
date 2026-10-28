# Architecture 0.3.6

Fast ASR is a native Rust / Win32 overlay using WS_EX_NOACTIVATE. Protected Unicode SendInput commits final results; hypotheses stay in the preview. The overlay retains clipboard recovery, focus binding, deferred output, adaptive height and scrolling. Elza is credited in the minimal settings window.

## Recognition pipeline

Only X-ASR-zh-en INT8 480 ms is built and packaged. Its C++ bridge dynamically loads sherpa-onnx 1.13.8 / ONNX Runtime CPU. One inference thread owns one online recognizer. WinMM captures 16 kHz mono audio; the coordinator drains 512-sample buffers roughly every 8 ms and forwards them directly to inference. There is no separate VAD, Whisper decoder or ggml dependency in the executable or portable runtime.

X-ASR uses greedy decoding and native endpoint detection: 600 ms trailing silence after speech, 2.4 s with no decoded content, and 30 s maximum utterance length. An endpoint publishes a final and resets the online stream. Stop adds two seconds of synthetic zeros to flush lookahead and signals input-finished; those zeros are not recorded or played. A new recording resets the stream, retaining the loaded model. These are model boundary rules, not a VAD probability or a 600 ms end-to-end guarantee.

The coordinator remains responsive while inference works. Generation IDs invalidate cancelled queued work and late replies. Queue accounting limits backlog to ten seconds of samples; overload stops capture and retains existing results. Capture failures are surfaced. Replay diagnostics feed bundled WAV audio at a real-time pace through the same worker, without opening a microphone.

## Recording and output

First-use settings enable the low-level left Alt hold hook. A lone left Alt held at least 500 ms starts capture; keyup stops and commits after finalization. Taps and combinations restore native Alt behavior. Esc or a combination during held capture cancels its unpublished text. Right Alt / AltGr and Ctrl bypass the hold gesture. During recording Alt+Esc is reserved for cancellation; before the threshold it forwards normally. A companion low-level mouse hook forwards Alt before an initiating click or wheel action and cancels the draft, preserving mouse chords; movement alone passes. Other held keys or mouse buttons prevent entry. Long recording holds suppress both Alt edges; short taps replay both in order. Native diagnostics observe forwarded system-key events without dispatching modal menu activation into the owned stock EDIT. Settings, busy states and another capture block new holds. The hold limit is 60 seconds; no audio is captured before activation.

Both mouse and hold modes stream to the same decoder. Mouse mode commits endpoint finals continuously. Hold mode accumulates finals separately from committed history and displays them with partials; release makes the draft available to protected input. It cannot rewrite the target while Alt remains down. Cancellation clears held draft, partials and stale inference before permitting a new start.

Low-level hooks run on a dedicated message thread; a second worker owns SendInput replay. Hook callbacks do not synchronously inject input. Physical releases and button/wheel events are queued behind outstanding replay batches so they cannot overtake delayed downs. Startup and shutdown are bounded, hook handles are explicitly unregistered, and foreground diagnostics never attach another application's input queue. A private inactive desktop probe verifies five install/uninstall cycles and native process termination without input injection. This is a lifecycle check, not native gesture compatibility verification.

## UI and persistence

The rounded pin button uses an accent outline and tinted fill while topmost, a neutral outline otherwise. SetWindowPos uses SWP_NOACTIVATE and stores the preference. Settings contain a fixed model label, microphone and shortcut selectors, concise instructions and developer credit. Missing or invalid settings default to left Alt hold; explicit prior shortcut choices survive upgrades. Retired model IDs deserialize as X-ASR; obsolete VAD fields are ignored and omitted when saving.

## Distribution

Only xasr_bridge.cpp is compiled. Pinned model and DLL preparation is Python stdlib code. The app validates X-ASR model sizes and hashes before loading and performs no downloads. The downloader and build scripts provide setup. The portable package contains X-ASR weights, three sherpa/ONNX DLLs, documentation, test WAVs, licenses and matching verification reports. Preview packages explicitly list pending desktop regressions and never reuse historical input/UI reports. Archived notices remain for provenance; they do not mean Whisper or Silero is shipped as a runtime. The source inventory excludes the retired Whisper bridge and setup script while retaining the historical manifest required by the notice checker.

Model, runtime and license provenance remains pinned. The repository and source ZIP exclude weights, native binaries, caches, credentials and user settings. Package checks enforce exact inventory, hashes and the absence of retired runtime/model binaries.
