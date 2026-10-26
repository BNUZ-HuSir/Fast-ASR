#define NOMINMAX
#include <windows.h>
#include <cstring>
#include <string>
#include <vector>
#include <stdexcept>
#include "sherpa-onnx/c-api/c-api.h"
#define ONLINE_APIS(X) \
 X(SherpaOnnxCreateOnlineRecognizer) X(SherpaOnnxDestroyOnlineRecognizer) \
 X(SherpaOnnxCreateOnlineStream) X(SherpaOnnxDestroyOnlineStream) \
 X(SherpaOnnxOnlineStreamAcceptWaveform) X(SherpaOnnxIsOnlineStreamReady) \
 X(SherpaOnnxDecodeOnlineStream) X(SherpaOnnxGetOnlineStreamResult) \
 X(SherpaOnnxDestroyOnlineRecognizerResult) X(SherpaOnnxOnlineStreamInputFinished) \
 X(SherpaOnnxOnlineStreamReset) X(SherpaOnnxOnlineStreamIsEndpoint)
static thread_local std::string xasr_error;
static std::string json_string(const std::string& text) {
    std::string result="\"";
    for(unsigned char ch:text) {
        if(ch=='"'||ch=='\\') {result+='\\';result+=ch;}
        else if(ch=='\n')result+="\\n";else if(ch=='\r')result+="\\r";else if(ch=='\t')result+="\\t";
        else if(ch>=32)result+=ch;
    }
    return result+'"';
}
struct Online {
    HMODULE library=nullptr;
    const SherpaOnnxOnlineRecognizer* recognizer=nullptr;
    const SherpaOnnxOnlineStream* stream=nullptr;
    std::string encoder,decoder,joiner,tokens;
    int threads=4,silence_ms=0;
    #define DECLARE(name) decltype(&name) name=nullptr;
    ONLINE_APIS(DECLARE)
    #undef DECLARE
    Online(const char* runtime,const char* model,int count):threads(count) {
        try {
            std::string filename=std::string(runtime)+"\\sherpa-onnx-c-api.dll";
            int n=MultiByteToWideChar(CP_UTF8,0,filename.c_str(),-1,nullptr,0);
            std::wstring path(n,L'\0');MultiByteToWideChar(CP_UTF8,0,filename.c_str(),-1,path.data(),n);
            library=LoadLibraryExW(path.c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
            if(!library)throw std::runtime_error("Cannot load X-ASR runtime, Windows error "+std::to_string(GetLastError()));
            #define LOAD(name) name=reinterpret_cast<decltype(name)>(GetProcAddress(library,#name));if(!name)throw std::runtime_error("Missing runtime function " #name);
            ONLINE_APIS(LOAD)
            #undef LOAD
            encoder=std::string(model)+"/encoder.int8.onnx";decoder=std::string(model)+"/decoder.onnx";
            joiner=std::string(model)+"/joiner.int8.onnx";tokens=std::string(model)+"/tokens.txt";
            load();reset(silence_ms);
        }catch(...){release();throw;}
    }
    void release() {
        if(stream){SherpaOnnxDestroyOnlineStream(stream);stream=nullptr;}
        if(recognizer){SherpaOnnxDestroyOnlineRecognizer(recognizer);recognizer=nullptr;}
        if(library){FreeLibrary(library);library=nullptr;}
    }
    ~Online(){release();}
    void load() {
        SherpaOnnxOnlineRecognizerConfig c{};
        c.feat_config.sample_rate=16000;c.feat_config.feature_dim=80;
        c.model_config.transducer.encoder=encoder.c_str();c.model_config.transducer.decoder=decoder.c_str();
        c.model_config.transducer.joiner=joiner.c_str();c.model_config.tokens=tokens.c_str();
        c.model_config.provider="cpu";c.model_config.num_threads=threads;
        c.decoding_method="greedy_search";c.enable_endpoint=silence_ms>0?1:0;
        c.rule1_min_trailing_silence=2.4f;c.rule2_min_trailing_silence=silence_ms/1000.f;c.rule3_min_utterance_length=30.f;
        recognizer=SherpaOnnxCreateOnlineRecognizer(&c);
        if(!recognizer)throw std::runtime_error("Cannot load X-ASR online recognizer");
    }
    void reset(int silence) {
        if(stream){SherpaOnnxDestroyOnlineStream(stream);stream=nullptr;}
        if(silence!=silence_ms){
            if(recognizer)SherpaOnnxDestroyOnlineRecognizer(recognizer);recognizer=nullptr;silence_ms=silence;
        }
        if(!recognizer)load();
        stream=SherpaOnnxCreateOnlineStream(recognizer);
        if(!stream)throw std::runtime_error("Cannot create X-ASR online stream");
    }
    std::string text() {
        if(!recognizer||!stream)throw std::runtime_error("X-ASR stream is not ready; start recording again");
        const auto* result=SherpaOnnxGetOnlineStreamResult(recognizer,stream);
        if(!result)throw std::runtime_error("Cannot read X-ASR result");
        std::string value=result->text?result->text:"";SherpaOnnxDestroyOnlineRecognizerResult(result);return value;
    }
    void decode(bool endpoints,std::vector<std::string>& finals) {
        while(SherpaOnnxIsOnlineStreamReady(recognizer,stream)) {
            SherpaOnnxDecodeOnlineStream(recognizer,stream);
            if(endpoints&&SherpaOnnxOnlineStreamIsEndpoint(recognizer,stream)) {
                auto value=text();if(!value.empty())finals.push_back(value);
                SherpaOnnxOnlineStreamReset(recognizer,stream);
            }
        }
    }
    std::string feed(const float* samples,int n,bool finish) {
        if(!recognizer||!stream)throw std::runtime_error("X-ASR stream is not ready; start recording again");
        std::vector<std::string> finals;
        if(n){SherpaOnnxOnlineStreamAcceptWaveform(stream,16000,samples,n);decode(true,finals);}
        if(finish){
            std::vector<float> tail(32000,0.f);
            SherpaOnnxOnlineStreamAcceptWaveform(stream,16000,tail.data(),(int)tail.size());decode(false,finals);
            SherpaOnnxOnlineStreamInputFinished(stream);decode(false,finals);
            auto value=text();if(!value.empty())finals.push_back(value);
        }
        std::string json="{\"finals\":[";
        for(size_t i=0;i<finals.size();++i){if(i)json+=',';json+=json_string(finals[i]);}
        json+="],\"partial\":"+json_string(finish?"":text())+"}";return json;
    }
};
extern "C" {
const char* fax_error(){return xasr_error.c_str();}
void* fax_create(const char* runtime,const char* model,int threads){
    try{return new Online(runtime,model,threads);}catch(const std::exception& e){xasr_error=e.what();return nullptr;}
}
void fax_destroy(void* p){delete static_cast<Online*>(p);}
int fax_reset(void* p,int silence_ms){
    try{static_cast<Online*>(p)->reset(silence_ms);return 1;}catch(const std::exception& e){xasr_error=e.what();return 0;}
}
int fax_feed(void* p,const float* samples,int n,int finish,char* out,int capacity){
    try {
        auto json=static_cast<Online*>(p)->feed(samples,n,finish!=0);
        if(json.size()+1>static_cast<size_t>(capacity))throw std::runtime_error("X-ASR result buffer exceeded");
        memcpy(out,json.c_str(),json.size()+1);return 1;
    }catch(const std::exception& e){xasr_error=e.what();return 0;}
}
}
