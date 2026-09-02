#include "openmind/native/inference.h"
#include "openmind/src/native_bridge.rs.h"

#include "llama.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <limits>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace openmind {
namespace {

std::once_flag g_backend_once;

void ensure_backend_initialized() {
    std::call_once(g_backend_once, [] { llama_backend_init(); });
}

std::string to_string(rust::Str value) {
    return std::string(value.data(), value.size());
}

std::uint32_t round_context(std::uint64_t value) {
    constexpr std::uint64_t kBlock = 256;
    const auto rounded = ((value + kBlock - 1) / kBlock) * kBlock;
    if (rounded > std::numeric_limits<std::uint32_t>::max()) {
        throw std::runtime_error("requested context is too large");
    }
    return static_cast<std::uint32_t>(rounded);
}

std::vector<llama_token> tokenize(
    const llama_vocab* vocab,
    const std::string& text,
    bool add_special) {
    if (text.size() > static_cast<std::size_t>(std::numeric_limits<std::int32_t>::max())) {
        throw std::runtime_error("prompt is too large to tokenize");
    }

    const auto text_len = static_cast<std::int32_t>(text.size());
    std::int32_t count = llama_tokenize(
        vocab,
        text.data(),
        text_len,
        nullptr,
        0,
        add_special,
        true);

    if (count == 0) {
        return {};
    }
    if (count > 0) {
        throw std::runtime_error("llama_tokenize returned an unexpected positive sizing result");
    }

    std::vector<llama_token> tokens(static_cast<std::size_t>(-count));
    count = llama_tokenize(
        vocab,
        text.data(),
        text_len,
        tokens.data(),
        static_cast<std::int32_t>(tokens.size()),
        add_special,
        true);
    if (count < 0) {
        throw std::runtime_error("failed to tokenize prompt");
    }
    tokens.resize(static_cast<std::size_t>(count));
    return tokens;
}

std::string token_piece(const llama_vocab* vocab, llama_token token) {
    std::vector<char> buffer(256);
    std::int32_t written = llama_token_to_piece(
        vocab,
        token,
        buffer.data(),
        static_cast<std::int32_t>(buffer.size()),
        0,
        true);

    if (written < 0) {
        buffer.resize(static_cast<std::size_t>(-written));
        written = llama_token_to_piece(
            vocab,
            token,
            buffer.data(),
            static_cast<std::int32_t>(buffer.size()),
            0,
            true);
    }
    if (written < 0) {
        throw std::runtime_error("failed to convert generated token to text");
    }
    return std::string(buffer.data(), static_cast<std::size_t>(written));
}

struct ContextDeleter {
    void operator()(llama_context* value) const noexcept {
        if (value != nullptr) {
            llama_free(value);
        }
    }
};

struct SamplerDeleter {
    void operator()(llama_sampler* value) const noexcept {
        if (value != nullptr) {
            llama_sampler_free(value);
        }
    }
};

using ContextPtr = std::unique_ptr<llama_context, ContextDeleter>;
using SamplerPtr = std::unique_ptr<llama_sampler, SamplerDeleter>;

} // namespace

class InferenceEngine::Impl final {
public:
    Impl(rust::Str model_path, std::int32_t n_gpu_layers) {
        ensure_backend_initialized();

        auto params = llama_model_default_params();
        params.n_gpu_layers = n_gpu_layers;
        model_ = llama_model_load_from_file(to_string(model_path).c_str(), params);
        if (model_ == nullptr) {
            throw std::runtime_error("failed to load GGUF model");
        }
        vocab_ = llama_model_get_vocab(model_);
        if (vocab_ == nullptr) {
            llama_model_free(model_);
            model_ = nullptr;
            throw std::runtime_error("loaded model does not expose a vocabulary");
        }
    }

    ~Impl() {
        context_.reset();
        if (model_ != nullptr) {
            llama_model_free(model_);
        }
    }

    void generate(
        rust::Str prompt,
        rust::Str system_prompt,
        const GenerationConfig& config,
        TokenSink& sink) {
        if (config.max_tokens == 0) {
            return;
        }
        if (!std::isfinite(config.temperature) || config.temperature < 0.0F) {
            throw std::runtime_error("temperature must be finite and >= 0");
        }
        if (!std::isfinite(config.top_p) || config.top_p <= 0.0F || config.top_p > 1.0F) {
            throw std::runtime_error("top_p must be finite and in (0, 1]");
        }

        std::string merged;
        const auto system = to_string(system_prompt);
        if (!system.empty()) {
            merged.reserve(system.size() + prompt.size() + 32);
            merged.append("System:\n");
            merged.append(system);
            merged.append("\n\nUser:\n");
            merged.append(prompt.data(), prompt.size());
            merged.append("\n\nAssistant:\n");
        } else {
            merged = to_string(prompt);
        }

        auto prompt_tokens = tokenize(vocab_, merged, true);
        if (prompt_tokens.empty()) {
            throw std::runtime_error("prompt produced no tokens");
        }

        const std::uint64_t required =
            static_cast<std::uint64_t>(prompt_tokens.size()) + config.max_tokens + 8ULL;
        const std::uint32_t desired_context = round_context(
            std::max<std::uint64_t>(std::max<std::uint32_t>(config.n_ctx, 512U), required));
        const std::uint32_t desired_batch = std::max<std::uint32_t>(
            32U,
            std::min<std::uint32_t>(
                config.n_batch == 0 ? 512U : config.n_batch,
                desired_context));
        const std::int32_t desired_threads = std::max<std::int32_t>(config.n_threads, 1);

        ensure_context(desired_context, desired_batch, desired_threads);
        llama_memory_clear(llama_get_memory(context_.get()), true);

        decode_prompt(prompt_tokens, desired_batch);

        SamplerPtr sampler;
        if (config.temperature <= 0.0F) {
            sampler.reset(llama_sampler_init_greedy());
        } else {
            auto chain_params = llama_sampler_chain_default_params();
            auto* chain = llama_sampler_chain_init(chain_params);
            if (chain == nullptr) {
                throw std::runtime_error("failed to create sampler chain");
            }
            sampler.reset(chain);
            llama_sampler_chain_add(chain, llama_sampler_init_top_p(config.top_p, 1));
            llama_sampler_chain_add(chain, llama_sampler_init_temp(config.temperature));
            llama_sampler_chain_add(chain, llama_sampler_init_dist(LLAMA_DEFAULT_SEED));
        }

        if (!sampler) {
            throw std::runtime_error("failed to create sampler");
        }

        for (std::uint32_t produced = 0; produced < config.max_tokens; ++produced) {
            const llama_token token = llama_sampler_sample(sampler.get(), context_.get(), -1);
            if (llama_vocab_is_eog(vocab_, token)) {
                break;
            }

            const auto piece = token_piece(vocab_, token);
            if (!on_token(sink, rust::Str(piece.data(), piece.size()))) {
                break;
            }

            auto next = token;
            const auto batch = llama_batch_get_one(&next, 1);
            const int decode_status = llama_decode(context_.get(), batch);
            if (decode_status != 0) {
                throw std::runtime_error("llama_decode failed while generating token");
            }
        }
    }

    void clear_kv_cache() {
        if (context_) {
            llama_memory_clear(llama_get_memory(context_.get()), true);
        }
    }

private:
    void ensure_context(
        std::uint32_t n_ctx,
        std::uint32_t n_batch,
        std::int32_t n_threads) {
        const bool shrink = context_capacity_ > 0 && n_ctx < context_capacity_ / 2U;
        const bool must_rebuild =
            !context_ || n_ctx > context_capacity_ || shrink || n_batch != batch_capacity_ ||
            n_threads != thread_count_;
        if (!must_rebuild) {
            return;
        }

        auto params = llama_context_default_params();
        params.n_ctx = n_ctx;
        params.n_batch = n_batch;
        params.n_ubatch = n_batch;
        params.n_threads = n_threads;
        params.n_threads_batch = n_threads;
        params.no_perf = true;

        ContextPtr replacement(llama_init_from_model(model_, params));
        if (!replacement) {
            throw std::runtime_error("failed to create llama.cpp context / KV cache");
        }

        context_ = std::move(replacement);
        context_capacity_ = n_ctx;
        batch_capacity_ = n_batch;
        thread_count_ = n_threads;
    }

    void decode_prompt(const std::vector<llama_token>& tokens, std::uint32_t n_batch) {
        std::size_t offset = 0;
        while (offset < tokens.size()) {
            const auto remaining = tokens.size() - offset;
            const auto chunk = static_cast<std::int32_t>(
                std::min<std::size_t>(remaining, static_cast<std::size_t>(n_batch)));
            auto batch = llama_batch_get_one(
                const_cast<llama_token*>(tokens.data() + offset),
                chunk);
            const int decode_status = llama_decode(context_.get(), batch);
            if (decode_status != 0) {
                throw std::runtime_error("llama_decode failed while processing prompt");
            }
            offset += static_cast<std::size_t>(chunk);
        }
    }

    llama_model* model_ = nullptr;
    const llama_vocab* vocab_ = nullptr;
    ContextPtr context_;
    std::uint32_t context_capacity_ = 0;
    std::uint32_t batch_capacity_ = 0;
    std::int32_t thread_count_ = 0;
};

InferenceEngine::InferenceEngine(rust::Str model_path, std::int32_t n_gpu_layers)
    : impl_(std::make_unique<Impl>(model_path, n_gpu_layers)) {}

InferenceEngine::~InferenceEngine() = default;
InferenceEngine::InferenceEngine(InferenceEngine&&) noexcept = default;
InferenceEngine& InferenceEngine::operator=(InferenceEngine&&) noexcept = default;

void InferenceEngine::generate(
    rust::Str prompt,
    rust::Str system_prompt,
    const GenerationConfig& config,
    TokenSink& sink) {
    impl_->generate(prompt, system_prompt, config, sink);
}

void InferenceEngine::clear_kv_cache() {
    impl_->clear_kv_cache();
}

std::unique_ptr<InferenceEngine> create_engine(rust::Str model_path, std::int32_t n_gpu_layers) {
    return std::make_unique<InferenceEngine>(model_path, n_gpu_layers);
}

} // namespace openmind
