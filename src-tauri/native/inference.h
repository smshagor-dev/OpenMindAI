#pragma once

#include "rust/cxx.h"

#include <cstdint>
#include <memory>

namespace openmind {

struct ChatMessage;
struct GenerationConfig;
struct TokenSink;

class InferenceEngine final {
public:
    InferenceEngine(rust::Str model_path, std::int32_t n_gpu_layers);
    ~InferenceEngine();

    InferenceEngine(const InferenceEngine&) = delete;
    InferenceEngine& operator=(const InferenceEngine&) = delete;
    InferenceEngine(InferenceEngine&&) noexcept;
    InferenceEngine& operator=(InferenceEngine&&) noexcept;

    void generate(
        rust::Str prompt,
        rust::Str system_prompt,
        const GenerationConfig& config,
        TokenSink& sink);

    void generate_messages(
        rust::Slice<const ChatMessage> messages,
        const GenerationConfig& config,
        TokenSink& sink);

    void clear_kv_cache();

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

std::unique_ptr<InferenceEngine> create_engine(rust::Str model_path, std::int32_t n_gpu_layers);

} // namespace openmind
