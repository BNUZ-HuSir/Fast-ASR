#define NOMINMAX
#include <windows.h>
#include "whisper.h"
#include <algorithm>
#include <cstdio>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

static thread_local std::string error_message;
static std::wstring wide(const char* s) {
    int n = MultiByteToWideChar(CP_UTF8, 0, s, -1, nullptr, 0);
    if (!n) throw std::runtime_error("Invalid UTF-8 path");
    std::wstring out(n, 0);
    MultiByteToWideChar(CP_UTF8, 0, s, -1, out.data(), n);
    out.resize(n-1); return out;
}
static void quiet(ggml_log_level, const char*, void*) {}
#define API_LIST(X) \
 X(whisper_context_default_params) X(whisper_init_with_params) X(whisper_free) \
 X(whisper_full_default_params) X(whisper_full) X(whisper_full_n_segments) \
 X(whisper_full_get_segment_text) X(whisper_full_get_segment_no_speech_prob) \
 X(whisper_vad_default_context_params) X(whisper_vad_init_with_params) \
 X(whisper_vad_detect_speech_no_reset) X(whisper_vad_n_probs) X(whisper_vad_probs) \
 X(whisper_vad_reset_state) X(whisper_vad_free) X(whisper_log_set)
struct Api {
    std::vector<HMODULE> modules;
#define DECL(name) decltype(&::name) name = nullptr;
    API_LIST(DECL)
#undef DECL
    explicit Api(const char* runtime) {
        try {
            auto base=wide(runtime);
            for (auto name : {L"ggml-base.dll", L"ggml-cpu.dll", L"ggml.dll", L"whisper.dll"}) {
                auto module=LoadLibraryExW((base+L"\\"+name).c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
                if (!module) throw std::runtime_error("Cannot load Whisper runtime, Windows error " + std::to_string(GetLastError()));
                modules.push_back(module);
            }
#define LOAD(name) name=reinterpret_cast<decltype(name)>(GetProcAddress(modules.back(),#name)); if (!name) throw std::runtime_error("Missing runtime API: " #name);
            API_LIST(LOAD)
#undef LOAD
            whisper_log_set(quiet,nullptr);
        } catch (...) { release(); throw; }
    }
    void release() { for (auto it=modules.rbegin();it!=modules.rend();++it) FreeLibrary(*it); modules.clear(); }
    ~Api() { release(); }
};
struct ModelFile {
    FILE* file;
    whisper_model_loader loader;
    explicit ModelFile(const char* path) : file(_wfopen(wide(path).c_str(),L"rb")) {
        if (!file) throw std::runtime_error("Cannot open model file");
        loader.context=file;
        loader.read=[](void* c, void* out,size_t n) { return fread(out,1,n,static_cast<FILE*>(c)); };
        loader.eof=[](void* c) { return feof(static_cast<FILE*>(c))!=0; };
        loader.close=[](void*) {};
    }
    ~ModelFile() { fclose(file); }
};
struct Decoder {
    Api api;
    whisper_context* context=nullptr;
    int threads;
    Decoder(const char* runtime,const char* model,int n) : api(runtime),threads(n) {
        auto params=api.whisper_context_default_params(); params.use_gpu=false; params.flash_attn=true;
        ModelFile file(model);
        context=api.whisper_init_with_params(&file.loader,params);
        if (!context) throw std::runtime_error("Whisper model initialization failed");
    }
    ~Decoder() { if (context) api.whisper_free(context); }
};
struct Vad {
    Api api;
    whisper_vad_context* context=nullptr;
    Vad(const char* runtime,const char* model) : api(runtime) {
        auto params=api.whisper_vad_default_context_params(); params.use_gpu=false; params.n_threads=1;
        ModelFile file(model);
        context=api.whisper_vad_init_with_params(&file.loader,params);
        if (!context) throw std::runtime_error("Silero VAD initialization failed");
    }
    ~Vad() { if (context) api.whisper_vad_free(context); }
};
extern "C" {
const char* fa_error() { return error_message.c_str(); }
void* fa_model_create(const char* runtime,const char* model,int threads) {
    try { return new Decoder(runtime,model,threads); } catch (const std::exception& e) { error_message=e.what(); return nullptr; }
}
void fa_model_destroy(void* ptr) { delete static_cast<Decoder*>(ptr); }
int fa_transcribe(void* ptr,const float* samples,int n,char* out,int cap) {
    try {
        auto& d=*static_cast<Decoder*>(ptr);
        auto p=d.api.whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
        p.n_threads=d.threads; p.no_context=true; p.no_timestamps=true;
        p.print_special=false; p.print_progress=false; p.print_realtime=false; p.print_timestamps=false;
        // Keep the upstream acoustic context: aggressive shortening hurts mixed-language accuracy.
        p.language="auto"; p.translate=false; p.detect_language=false;
        p.initial_prompt="简体中文，English。";
        p.suppress_blank=true; p.suppress_nst=true; p.temperature_inc=0; p.greedy.best_of=1;
        if (d.api.whisper_full(d.context,p,samples,n)!=0) throw std::runtime_error("Whisper inference failed");
        std::string text;
        for (int i=0;i<d.api.whisper_full_n_segments(d.context);++i) {
            if (d.api.whisper_full_get_segment_no_speech_prob(d.context,i)>0.8f) continue;
            text+=d.api.whisper_full_get_segment_text(d.context,i);
        }
        if (text.size()+1>static_cast<size_t>(cap)) throw std::runtime_error("Recognition output exceeds buffer");
        memcpy(out,text.c_str(),text.size()+1); return 1;
    } catch (const std::exception& e) { error_message=e.what(); return 0; }
}
void* fa_vad_create(const char* runtime,const char* model) {
    try { return new Vad(runtime,model); } catch (const std::exception& e) { error_message=e.what(); return nullptr; }
}
void fa_vad_destroy(void* ptr) { delete static_cast<Vad*>(ptr); }
void fa_vad_reset(void* ptr) { auto& v=*static_cast<Vad*>(ptr); v.api.whisper_vad_reset_state(v.context); }
float fa_vad_probability(void* ptr,const float* samples,int n) {
    auto& v=*static_cast<Vad*>(ptr);
    if (!v.api.whisper_vad_detect_speech_no_reset(v.context,samples,n)) { error_message="VAD inference failed"; return -1; }
    int count=v.api.whisper_vad_n_probs(v.context);
    return count>0 ? v.api.whisper_vad_probs(v.context)[count-1] : 0;
}
}
