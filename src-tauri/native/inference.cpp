#include "openmind/native/inference.h"
#include "openmind/src/native_bridge.rs.h"

#include "llama.h"

#include <algorithm>
#include <cmath>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <limits>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

#ifdef OPENMINDAI_DYNAMIC_BACKENDS
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <filesystem>
#include "ggml-backend.h"
#endif

namespace openmind {
namespace {

std::once_flag g_backend_once;

#ifdef OPENMINDAI_DYNAMIC_BACKENDS
std::once_flag g_vulkan_once;
bool g_vulkan_available = false;
std::filesystem::path g_backend_directory;

std::filesystem::path backend_directory() {
    // Development override is a single explicit directory, never a search path.
    const auto needed = GetEnvironmentVariableW(L"OPENMINDAI_NATIVE_BACKEND_DIR", nullptr, 0);
    if (needed > 0) {
        std::wstring value(needed, L'\0');
        const auto length = GetEnvironmentVariableW(
            L"OPENMINDAI_NATIVE_BACKEND_DIR", value.data(), needed);
        if (length == 0 || length >= needed) {
            throw std::runtime_error("native backend directory changed while reading environment");
        }
        value.resize(length);
        const std::filesystem::path path(value);
        if (!path.is_absolute()) throw std::runtime_error("native backend directory must be absolute");
        return std::filesystem::canonical(path);
    }
    std::wstring path(32768, L'\0');
    const auto length = GetModuleFileNameW(nullptr, path.data(), static_cast<DWORD>(path.size()));
    if (length == 0 || length >= path.size()) {
        throw std::runtime_error("cannot locate executable directory for native backends");
    }
    path.resize(length);
    return std::filesystem::canonical(std::filesystem::path(path).parent_path());
}

ggml_backend_reg_t load_backend(const wchar_t* name) {
    const auto path = g_backend_directory / name;
    // Preload dependencies using only this directory and System32. ggml then
    // acquires its own module reference without searching PATH or the CWD.
    DWORD previous_mode = 0;
    const bool changed_mode = SetThreadErrorMode(SEM_FAILCRITICALERRORS, &previous_mode) != 0;
    const auto module = LoadLibraryExW(path.c_str(), nullptr,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (changed_mode) SetThreadErrorMode(previous_mode, nullptr);
    if (module == nullptr) return nullptr;
    struct ModuleGuard {
        HMODULE value;
        ~ModuleGuard() { FreeLibrary(value); }
    } guard{module};
    return ggml_backend_load(path.u8string().c_str());
}
#endif

void ensure_backend_initialized(std::int32_t n_gpu_layers) {
#ifdef OPENMINDAI_DYNAMIC_BACKENDS
    std::call_once(g_backend_once, [] {
        g_backend_directory = backend_directory();
        auto* cpu = load_backend(L"ggml-cpu.dll");
        if (cpu == nullptr || ggml_backend_reg_dev_count(cpu) == 0) {
            throw std::runtime_error("native CPU backend is unavailable");
        }
        // Register CPU first: llama_backend_init must not scan arbitrary backend
        // directories through ggml_backend_load_all or GGML_BACKEND_PATH.
        llama_backend_init();
    });
    if (n_gpu_layers != 0) {
        std::call_once(g_vulkan_once, [] {
            if (std::getenv("GGML_DISABLE_VULKAN") != nullptr) return;
            auto* vulkan = load_backend(L"ggml-vulkan.dll");
            g_vulkan_available = vulkan != nullptr && ggml_backend_reg_dev_count(vulkan) > 0;
        });
        if (!g_vulkan_available) {
            // The Rust router retries with n_gpu_layers=0 before emitting output.
            throw std::runtime_error("native Vulkan backend is unavailable; CPU retry required");
        }
    }
#else
    (void)n_gpu_layers;
    std::call_once(g_backend_once, [] { llama_backend_init(); });
#endif
}

std::string to_string(rust::Str value) {
    return std::string(value.data(), value.size());
}

std::string to_string(const rust::String& value) {
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

bool supported_role(const std::string& role) {
    return role == "system" || role == "user" || role == "assistant";
}

std::string apply_chat_template(
    const llama_model* model,
    const std::vector<std::pair<std::string, std::string>>& owned_messages) {
    if (owned_messages.empty()) {
        throw std::runtime_error("chat history must contain at least one message");
    }

    const char* chat_template = llama_model_chat_template(model, nullptr);
    if (chat_template == nullptr || chat_template[0] == '\0') {
        if (owned_messages.size() == 1U && owned_messages.front().first == "user") {
            return owned_messages.front().second;
        }
        throw std::runtime_error(
            "model has no chat template; refusing to flatten multi-turn role history");
    }

    std::vector<llama_chat_message> messages;
    messages.reserve(owned_messages.size());
    for (const auto& message : owned_messages) {
        if (!supported_role(message.first)) {
            throw std::runtime_error("unsupported chat message role: " + message.first);
        }
        messages.push_back({message.first.c_str(), message.second.c_str()});
    }

    int32_t required = llama_chat_apply_template(
        chat_template,
        messages.data(),
        messages.size(),
        true,
        nullptr,
        0);
    if (required < 0) {
        throw std::runtime_error("model chat template is not supported by this llama.cpp build");
    }

    std::vector<char> formatted(static_cast<std::size_t>(required) + 1U);
    int32_t written = llama_chat_apply_template(
        chat_template,
        messages.data(),
        messages.size(),
        true,
        formatted.data(),
        static_cast<int32_t>(formatted.size()));
    if (written < 0) {
        throw std::runtime_error("failed to apply model chat template");
    }
    if (written > static_cast<int32_t>(formatted.size())) {
        formatted.resize(static_cast<std::size_t>(written) + 1U);
        written = llama_chat_apply_template(
            chat_template,
            messages.data(),
            messages.size(),
            true,
            formatted.data(),
            static_cast<int32_t>(formatted.size()));
        if (written < 0 || written > static_cast<int32_t>(formatted.size())) {
            throw std::runtime_error("failed to resize model chat template buffer");
        }
    }

    return std::string(formatted.data(), static_cast<std::size_t>(written));
}

std::string format_prompt(
    const llama_model* model,
    const std::string& prompt,
    const std::string& system_prompt) {
    std::vector<std::pair<std::string, std::string>> messages;
    messages.reserve(system_prompt.empty() ? 1U : 2U);
    if (!system_prompt.empty()) {
        messages.emplace_back("system", system_prompt);
    }
    messages.emplace_back("user", prompt);
    return apply_chat_template(model, messages);
}

std::string format_messages(
    const llama_model* model,
    rust::Slice<const ChatMessage> messages) {
    std::vector<std::pair<std::string, std::string>> owned_messages;
    owned_messages.reserve(messages.size());
    for (const auto& message : messages) {
        owned_messages.emplace_back(to_string(message.role), to_string(message.content));
    }
    return apply_chat_template(model, owned_messages);
}

std::vector<llama_token> tokenize(
    const llama_vocab* vocab,
    const std::string& text,
    bool add_special) {
    if (text.size() > static_cast<std::size_t>(std::numeric_limits<std::int32_t>::max())) {
        throw std::runtime_error("prompt is too large to tokenize");
    }

    const auto text_len = static_cast<std::int32_t>(text.size());
    std::int32_t count = llama_tokenize(vocab, text.data(), text_len, nullptr, 0, add_special, true);
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
        if (value != nullptr) llama_free(value);
    }
};

struct SamplerDeleter {
    void operator()(llama_sampler* value) const noexcept {
        if (value != nullptr) llama_sampler_free(value);
    }
};

using ContextPtr = std::unique_ptr<llama_context, ContextDeleter>;
using SamplerPtr = std::unique_ptr<llama_sampler, SamplerDeleter>;

} // namespace

class InferenceEngine::Impl final {
public:
    Impl(rust::Str model_path, std::int32_t n_gpu_layers) {
        ensure_backend_initialized(n_gpu_layers);
        auto params = llama_model_default_params();
        params.n_gpu_layers = n_gpu_layers;
        model_ = llama_model_load_from_file(to_string(model_path).c_str(), params);
        if (model_ == nullptr) throw std::runtime_error("failed to load GGUF model");
        vocab_ = llama_model_get_vocab(model_);
        if (vocab_ == nullptr) {
            llama_model_free(model_);
            model_ = nullptr;
            throw std::runtime_error("loaded model does not expose a vocabulary");
        }
    }

    ~Impl() {
        context_.reset();
        if (model_ != nullptr) llama_model_free(model_);
    }

    void generate(
        rust::Str prompt,
        rust::Str system_prompt,
        const GenerationConfig& config,
        TokenSink& sink) {
        generate_formatted(format_prompt(model_, to_string(prompt), to_string(system_prompt)), config, sink);
    }

    void generate_messages(
        rust::Slice<const ChatMessage> messages,
        const GenerationConfig& config,
        TokenSink& sink) {
        generate_formatted(format_messages(model_, messages), config, sink);
    }

    void clear_kv_cache() {
        std::lock_guard<std::mutex> guard(inference_mutex_);
        if (context_) llama_memory_clear(llama_get_memory(context_.get()), true);
    }

private:
    void generate_formatted(
        const std::string& formatted_prompt,
        const GenerationConfig& config,
        TokenSink& sink) {
        std::lock_guard<std::mutex> guard(inference_mutex_);
        if (config.max_tokens == 0) return;
        if (!std::isfinite(config.temperature) || config.temperature < 0.0F)
            throw std::runtime_error("temperature must be finite and >= 0");
        if (!std::isfinite(config.top_p) || config.top_p <= 0.0F || config.top_p > 1.0F)
            throw std::runtime_error("top_p must be finite and in (0, 1]");
        if (config.n_threads <= 0) throw std::runtime_error("n_threads must be greater than 0");

        if (formatted_prompt.size() > 1024U * 1024U || config.n_ctx > 32768U ||
            config.max_tokens > 8192U || config.n_batch > 2048U || config.n_threads > 256 ||
            config.timeout_ms == 0 || config.timeout_ms > 3600000U ||
            config.kv_cache_limit_bytes < 16ULL * 1024 * 1024 ||
            config.kv_cache_limit_bytes > 4ULL * 1024 * 1024 * 1024)
            throw std::runtime_error("native resource limit exceeded");
        const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(config.timeout_ms);
        const auto check_deadline = [&] {
            if (std::chrono::steady_clock::now() >= deadline)
                throw std::runtime_error("native generation deadline exceeded");
        };
        check_deadline();
        auto prompt_tokens = tokenize(vocab_, formatted_prompt, true);
        if (prompt_tokens.empty()) throw std::runtime_error("prompt produced no tokens");

        const std::uint64_t required =
            static_cast<std::uint64_t>(prompt_tokens.size()) + config.max_tokens + 8ULL;
        const std::uint32_t context_limit = config.n_ctx == 0 ? 8192U : config.n_ctx;
        if (required > context_limit || context_limit < 512U)
            throw std::runtime_error("native context limit exceeded; shorten the conversation or increase its configured limit");
        // Conservative dense F16 K+V estimate (GQA/sliding windows may use less).
        // This is an admission budget, not an assertion about total process RSS.
        const auto layers = std::max(llama_model_n_layer(model_), 1);
        const auto width = std::max(llama_model_n_embd(model_), 1);
        const auto per_token = static_cast<std::uint64_t>(layers) * width * 4ULL;
        const auto budget_context = std::min<std::uint64_t>(context_limit,
            config.kv_cache_limit_bytes / per_token) / 256U * 256U;
        if (budget_context < round_context(required) || budget_context < 512U)
            throw std::runtime_error("native KV memory budget exceeded");
        const std::uint32_t desired_context = std::max<std::uint32_t>(512U, round_context(required));
        const std::uint32_t desired_batch = std::max<std::uint32_t>(
            32U,
            std::min<std::uint32_t>(config.n_batch == 0 ? 512U : config.n_batch, desired_context));
        const std::int32_t desired_threads = std::max<std::int32_t>(config.n_threads, 1);

        check_deadline();
        ensure_context(desired_context, desired_batch, desired_threads);
        struct AbortGuard {
            llama_context* ctx;
            ~AbortGuard() { llama_set_abort_callback(ctx, nullptr, nullptr); }
        } abort_guard{context_.get()};
        llama_set_abort_callback(context_.get(), [](void* data) {
            return std::chrono::steady_clock::now() >= *static_cast<const std::chrono::steady_clock::time_point*>(data);
        }, const_cast<std::chrono::steady_clock::time_point*>(&deadline));
        llama_memory_clear(llama_get_memory(context_.get()), true);
        try {
            if (!decode_prompt(prompt_tokens, desired_batch, sink)) return;
        } catch (...) { check_deadline(); throw; }
        check_deadline();

        SamplerPtr sampler;
        if (config.temperature <= 0.0F) {
            sampler.reset(llama_sampler_init_greedy());
        } else {
            auto* chain = llama_sampler_chain_init(llama_sampler_chain_default_params());
            if (chain == nullptr) throw std::runtime_error("failed to create sampler chain");
            sampler.reset(chain);
            llama_sampler_chain_add(chain, llama_sampler_init_top_p(config.top_p, 1));
            llama_sampler_chain_add(chain, llama_sampler_init_temp(config.temperature));
            llama_sampler_chain_add(chain, llama_sampler_init_dist(LLAMA_DEFAULT_SEED));
        }
        if (!sampler) throw std::runtime_error("failed to create sampler");

        for (std::uint32_t produced = 0; produced < config.max_tokens; ++produced) {
            check_deadline();
            if (!on_token(sink, rust::Slice<const std::uint8_t>())) return;
            const llama_token token = llama_sampler_sample(sampler.get(), context_.get(), -1);
            if (llama_vocab_is_eog(vocab_, token)) break;

            const auto piece = token_piece(vocab_, token);
            if (!on_token(sink, rust::Slice<const std::uint8_t>(
                    reinterpret_cast<const std::uint8_t*>(piece.data()), piece.size()))) return;

            auto next = token;
            const auto batch = llama_batch_get_one(&next, 1);
            if (llama_decode(context_.get(), batch) != 0) {
                check_deadline();
                throw std::runtime_error("llama_decode failed while generating token");
            }
        }
    }

    void ensure_context(std::uint32_t n_ctx, std::uint32_t n_batch, std::int32_t n_threads) {
        const bool shrink = context_capacity_ > 0 && n_ctx < context_capacity_ / 2U;
        const bool must_rebuild =
            !context_ || n_ctx > context_capacity_ || shrink || n_batch != batch_capacity_ ||
            n_threads != thread_count_;
        if (!must_rebuild) return;

        auto params = llama_context_default_params();
        params.n_ctx = n_ctx;
        params.n_batch = n_batch;
        params.n_ubatch = n_batch;
        params.n_threads = n_threads;
        params.n_threads_batch = n_threads;
        params.no_perf = true;

        // Release the old KV/compute buffers before allocating their replacement.
        // Keeping both live during resize can double peak context memory.
        context_.reset();
        context_capacity_ = batch_capacity_ = 0;
        ContextPtr replacement(llama_init_from_model(model_, params));
        if (!replacement) throw std::runtime_error("failed to create llama.cpp context / KV cache");
        context_ = std::move(replacement);
        context_capacity_ = n_ctx;
        batch_capacity_ = n_batch;
        thread_count_ = n_threads;
    }

    bool decode_prompt(const std::vector<llama_token>& tokens, std::uint32_t n_batch, TokenSink& sink) {
        std::size_t offset = 0;
        while (offset < tokens.size()) {
            if (!on_token(sink, rust::Slice<const std::uint8_t>())) return false;
            const auto remaining = tokens.size() - offset;
            const auto chunk = static_cast<std::int32_t>(
                std::min<std::size_t>(remaining, static_cast<std::size_t>(n_batch)));
            auto batch = llama_batch_get_one(const_cast<llama_token*>(tokens.data() + offset), chunk);
            if (llama_decode(context_.get(), batch) != 0)
                throw std::runtime_error("llama_decode failed while processing prompt");
            offset += static_cast<std::size_t>(chunk);
        }
        return true;
    }

    llama_model* model_ = nullptr;
    const llama_vocab* vocab_ = nullptr;
    ContextPtr context_;
    std::uint32_t context_capacity_ = 0;
    std::uint32_t batch_capacity_ = 0;
    std::int32_t thread_count_ = 0;
    std::mutex inference_mutex_;
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

void InferenceEngine::generate_messages(
    rust::Slice<const ChatMessage> messages,
    const GenerationConfig& config,
    TokenSink& sink) {
    impl_->generate_messages(messages, config, sink);
}

void InferenceEngine::clear_kv_cache() { impl_->clear_kv_cache(); }

std::unique_ptr<InferenceEngine> create_engine(rust::Str model_path, std::int32_t n_gpu_layers) {
    return std::make_unique<InferenceEngine>(model_path, n_gpu_layers);
}

} // namespace openmind
