#include "inference.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <limits>
#include <mutex>
#include <stdexcept>
#include <string>
#include <thread>
#include <utility>
#include <vector>

#include "llama.h"

namespace openmind::native {
namespace {

constexpr std::uint32_t kMinimumContext = 512;
constexpr std::uint32_t kDefaultContext = 4096;
constexpr std::uint32_t kPrefillBatch = 2048;
constexpr std::uint32_t kMaxGeneratedTokens = 65536;

class BackendLifetime final {
 public:
  BackendLifetime() { llama_backend_init(); }
  ~BackendLifetime() { llama_backend_free(); }

  BackendLifetime(const BackendLifetime&) = delete;
  BackendLifetime& operator=(const BackendLifetime&) = delete;
};

BackendLifetime& backend_lifetime() {
  static BackendLifetime backend;
  return backend;
}

std::uint32_t round_context(std::uint64_t required, std::uint32_t limit) {
  if (required > limit) {
    throw std::runtime_error(
        "requested prompt + generation exceeds the model training context window");
  }

  std::uint64_t value = kMinimumContext;
  while (value < required && value < limit) {
    value = std::min<std::uint64_t>(value * 2, limit);
  }
  if (value < required) {
    throw std::runtime_error("unable to allocate a large enough KV context");
  }
  return static_cast<std::uint32_t>(value);
}

std::vector<llama_token> tokenize(const llama_vocab* vocab, const std::string& text) {
  if (text.size() > static_cast<std::size_t>(std::numeric_limits<std::int32_t>::max())) {
    throw std::runtime_error("prompt is too large to tokenize");
  }

  const auto text_len = static_cast<std::int32_t>(text.size());
  std::int32_t count = llama_tokenize(
      vocab, text.data(), text_len, nullptr, 0, true, true);

  if (count == std::numeric_limits<std::int32_t>::min()) {
    throw std::runtime_error("tokenization overflow");
  }
  if (count > 0) {
    // The API normally reports required capacity as a negative value when the
    // provided token buffer is too small. Accept a positive result as well for
    // compatibility with models/backends that can tokenize into zero capacity.
    std::vector<llama_token> tokens(static_cast<std::size_t>(count));
    const auto written = llama_tokenize(
        vocab, text.data(), text_len, tokens.data(), count, true, true);
    if (written < 0) {
      throw std::runtime_error("tokenization failed while filling the token buffer");
    }
    tokens.resize(static_cast<std::size_t>(written));
    return tokens;
  }

  const std::int32_t required = -count;
  if (required <= 0) {
    return {};
  }

  std::vector<llama_token> tokens(static_cast<std::size_t>(required));
  const auto written = llama_tokenize(
      vocab, text.data(), text_len, tokens.data(), required, true, true);
  if (written < 0) {
    throw std::runtime_error("tokenization failed while filling the token buffer");
  }
  tokens.resize(static_cast<std::size_t>(written));
  return tokens;
}

std::string token_piece(const llama_vocab* vocab, llama_token token) {
  std::vector<char> buffer(256);
  std::int32_t written = llama_token_to_piece(
      vocab, token, buffer.data(), static_cast<std::int32_t>(buffer.size()), 0, false);
  if (written < 0) {
    const auto required = static_cast<std::size_t>(-written);
    buffer.resize(required);
    written = llama_token_to_piece(
        vocab, token, buffer.data(), static_cast<std::int32_t>(buffer.size()), 0, false);
  }
  if (written < 0) {
    throw std::runtime_error("failed to decode generated token piece");
  }
  return std::string(buffer.data(), static_cast<std::size_t>(written));
}

std::string format_chat(
    const llama_model* model,
    const std::string& system_prompt,
    const std::string& prompt) {
  const char* chat_template = llama_model_chat_template(model, nullptr);
  if (chat_template == nullptr || *chat_template == '\0') {
    std::string fallback;
    if (!system_prompt.empty()) {
      fallback.append("System:\n").append(system_prompt).append("\n\n");
    }
    fallback.append("User:\n").append(prompt).append("\n\nAssistant:\n");
    return fallback;
  }

  std::vector<llama_chat_message> messages;
  messages.reserve(system_prompt.empty() ? 1 : 2);
  if (!system_prompt.empty()) {
    messages.push_back({"system", system_prompt.c_str()});
  }
  messages.push_back({"user", prompt.c_str()});

  std::vector<char> output(
      std::max<std::size_t>(1024, 2 * (system_prompt.size() + prompt.size()) + 256));
  std::int32_t required = llama_chat_apply_template(
      chat_template,
      messages.data(),
      messages.size(),
      true,
      output.data(),
      static_cast<std::int32_t>(output.size()));
  if (required < 0) {
    throw std::runtime_error("failed to apply the model chat template");
  }

  if (static_cast<std::size_t>(required) > output.size()) {
    output.resize(static_cast<std::size_t>(required));
    required = llama_chat_apply_template(
        chat_template,
        messages.data(),
        messages.size(),
        true,
        output.data(),
        static_cast<std::int32_t>(output.size()));
    if (required < 0 || static_cast<std::size_t>(required) > output.size()) {
      throw std::runtime_error("model chat template output exceeded allocated capacity");
    }
  }

  return std::string(output.data(), static_cast<std::size_t>(required));
}

std::int32_t worker_threads() {
  const auto detected = std::thread::hardware_concurrency();
  if (detected <= 2) {
    return 1;
  }
  return static_cast<std::int32_t>(std::min<unsigned int>(detected - 1, 16));
}

struct SamplerGuard final {
  llama_sampler* value = nullptr;
  ~SamplerGuard() {
    if (value != nullptr) {
      llama_sampler_free(value);
    }
  }
};

}  // namespace

class InferenceEngine::Impl final {
 public:
  Impl(std::string model_path, std::uint32_t base_context_tokens, std::int32_t gpu_layers)
      : base_context_tokens_(std::max(base_context_tokens, kMinimumContext)) {
    (void)backend_lifetime();

    auto model_params = llama_model_default_params();
    model_params.n_gpu_layers = gpu_layers < 0
        ? std::numeric_limits<std::int32_t>::max()
        : gpu_layers;

    model_ = llama_model_load_from_file(model_path.c_str(), model_params);
    if (model_ == nullptr) {
      throw std::runtime_error("failed to load GGUF model: " + model_path);
    }

    vocab_ = llama_model_get_vocab(model_);
    if (vocab_ == nullptr) {
      throw std::runtime_error("loaded model does not expose a vocabulary");
    }

    const auto train_context = llama_model_n_ctx_train(model_);
    model_context_limit_ = train_context > 0
        ? static_cast<std::uint32_t>(train_context)
        : std::max(base_context_tokens_, kDefaultContext);
    base_context_tokens_ = std::min(base_context_tokens_, model_context_limit_);
  }

  ~Impl() {
    if (context_ != nullptr) {
      llama_free(context_);
      context_ = nullptr;
    }
    if (model_ != nullptr) {
      llama_model_free(model_);
      model_ = nullptr;
    }
  }

  void generate(
      const std::string& prompt,
      const std::string& system_prompt,
      float temperature,
      float top_p,
      std::uint32_t max_tokens,
      const TokenSink& sink,
      rust::Fn<void(const TokenSink&, rust::Slice<const std::uint8_t>)> on_token) {
    std::scoped_lock generation_lock(generation_mutex_);

    if (prompt.empty()) {
      throw std::invalid_argument("prompt cannot be empty");
    }
    if (!std::isfinite(temperature) || temperature < 0.0F || temperature > 5.0F) {
      throw std::invalid_argument("temperature must be finite and between 0 and 5");
    }
    if (!std::isfinite(top_p) || top_p <= 0.0F || top_p > 1.0F) {
      throw std::invalid_argument("top_p must be finite and in the range (0, 1]");
    }
    if (max_tokens == 0 || max_tokens > kMaxGeneratedTokens) {
      throw std::invalid_argument("max_tokens must be between 1 and 65536");
    }

    const std::string formatted = format_chat(model_, system_prompt, prompt);
    const std::vector<llama_token> prompt_tokens = tokenize(vocab_, formatted);
    if (prompt_tokens.empty()) {
      throw std::runtime_error("prompt tokenization produced no tokens");
    }

    const std::uint64_t required_context =
        static_cast<std::uint64_t>(prompt_tokens.size()) + max_tokens + 8;
    ensure_context(required_context);

    llama_memory_clear(llama_get_memory(context_), true);

    const auto n_batch = std::max<std::uint32_t>(1, llama_n_batch(context_));
    for (std::size_t offset = 0; offset < prompt_tokens.size();) {
      const auto count = std::min<std::size_t>(
          prompt_tokens.size() - offset,
          static_cast<std::size_t>(n_batch));
      llama_batch batch = llama_batch_get_one(
          const_cast<llama_token*>(prompt_tokens.data() + offset),
          static_cast<std::int32_t>(count));
      if (llama_decode(context_, batch) != 0) {
        throw std::runtime_error("llama_decode failed while evaluating the prompt");
      }
      offset += count;
    }

    SamplerGuard sampler;
    sampler.value = llama_sampler_chain_init(llama_sampler_chain_default_params());
    if (sampler.value == nullptr) {
      throw std::runtime_error("failed to create llama sampler chain");
    }

    if (temperature <= 0.0F) {
      llama_sampler_chain_add(sampler.value, llama_sampler_init_greedy());
    } else {
      llama_sampler_chain_add(sampler.value, llama_sampler_init_top_p(top_p, 1));
      llama_sampler_chain_add(sampler.value, llama_sampler_init_temp(temperature));
      llama_sampler_chain_add(sampler.value, llama_sampler_init_dist(LLAMA_DEFAULT_SEED));
    }

    for (std::uint32_t generated = 0; generated < max_tokens; ++generated) {
      const llama_token token = llama_sampler_sample(sampler.value, context_, -1);
      if (llama_vocab_is_eog(vocab_, token)) {
        break;
      }

      const std::string piece = token_piece(vocab_, token);
      if (!piece.empty()) {
        const auto* bytes = reinterpret_cast<const std::uint8_t*>(piece.data());
        on_token(sink, rust::Slice<const std::uint8_t>(bytes, piece.size()));
      }

      if (generated + 1 == max_tokens) {
        break;
      }

      llama_token next = token;
      llama_batch batch = llama_batch_get_one(&next, 1);
      if (llama_decode(context_, batch) != 0) {
        throw std::runtime_error("llama_decode failed while generating a token");
      }
    }
  }

 private:
  void ensure_context(std::uint64_t required) {
    const std::uint32_t minimum = std::max(base_context_tokens_, kMinimumContext);
    const std::uint32_t rounded = round_context(std::max<std::uint64_t>(required, minimum), model_context_limit_);

    if (context_ != nullptr && context_tokens_ >= rounded) {
      return;
    }

    if (context_ != nullptr) {
      llama_free(context_);
      context_ = nullptr;
    }

    auto context_params = llama_context_default_params();
    context_params.n_ctx = rounded;
    context_params.n_batch = std::min<std::uint32_t>(rounded, kPrefillBatch);
    context_params.n_threads = worker_threads();
    context_params.n_threads_batch = context_params.n_threads;

    context_ = llama_init_from_model(model_, context_params);
    if (context_ == nullptr) {
      context_tokens_ = 0;
      throw std::runtime_error("failed to allocate llama context/KV cache");
    }
    context_tokens_ = rounded;
  }

  llama_model* model_ = nullptr;
  const llama_vocab* vocab_ = nullptr;
  llama_context* context_ = nullptr;
  std::uint32_t base_context_tokens_ = kDefaultContext;
  std::uint32_t model_context_limit_ = kDefaultContext;
  std::uint32_t context_tokens_ = 0;
  std::mutex generation_mutex_;
};

InferenceEngine::InferenceEngine(std::unique_ptr<Impl> impl) noexcept
    : impl_(std::move(impl)) {}
InferenceEngine::~InferenceEngine() = default;
InferenceEngine::InferenceEngine(InferenceEngine&&) noexcept = default;
InferenceEngine& InferenceEngine::operator=(InferenceEngine&&) noexcept = default;

std::unique_ptr<InferenceEngine> load_model(
    rust::Str model_path,
    std::uint32_t base_context_tokens,
    std::int32_t gpu_layers) {
  if (model_path.empty()) {
    throw std::invalid_argument("model_path cannot be empty");
  }
  std::string path(model_path.data(), model_path.size());
  auto impl = std::make_unique<InferenceEngine::Impl>(
      std::move(path),
      base_context_tokens == 0 ? kDefaultContext : base_context_tokens,
      gpu_layers);
  return std::unique_ptr<InferenceEngine>(new InferenceEngine(std::move(impl)));
}

void generate_stream(
    InferenceEngine& engine,
    rust::Str prompt,
    rust::Str system_prompt,
    float temperature,
    float top_p,
    std::uint32_t max_tokens,
    const TokenSink& sink,
    rust::Fn<void(const TokenSink&, rust::Slice<const std::uint8_t>)> on_token) {
  if (!engine.impl_) {
    throw std::runtime_error("inference engine is not initialized");
  }

  engine.impl_->generate(
      std::string(prompt.data(), prompt.size()),
      std::string(system_prompt.data(), system_prompt.size()),
      temperature,
      top_p,
      max_tokens,
      sink,
      on_token);
}

}  // namespace openmind::native
