# Third-party notices

Fast ASR 0.3.1 bundles X-ASR-zh-en 480 ms punct INT8 model files (Apache-2.0), sherpa-onnx 1.13.8 (Apache-2.0), ONNX Runtime (MIT), whisper.cpp 1.9.5 / ggml (MIT), OpenAI Whisper Large-v3-Turbo Q5_0 weights (MIT), and Silero VAD v5.1.2 (MIT). Full available notices and ONNX Runtime third-party notices are included under licenses/. Whisper runtime DLLs are built from pinned upstream source with static MSVC CRT and AVX2 CPU support.

Model revisions, exact sizes and SHA-256 hashes are pinned in assets/xasr-manifest.json and assets/whisper-manifest.json (also distributed under models/). Silero weights use ggml-org/whisper-vad revision 9ffd54a1e1ee413ddf265af9913beaf518d1639b, SHA-256 29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf.

Diagnostic Chinese/English/mixed WAVs originate from k2-fsa/sherpa-onnx (Apache-2.0); the English JFK sample originates from whisper.cpp (MIT). These are diagnostic assets, not accuracy references.

Rust dependencies and declared licenses are enumerated in licenses/rust-dependencies.json, with available upstream notices under licenses/rust/. Every distributed immutable file is recorded in build-info.json. Old Whisper Small and the 2023 Zipformer models are excluded from this release.

Application source is offered under the MIT License (LICENSE); upstream model and runtime licenses remain separate.
