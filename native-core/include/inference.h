#pragma once

#include <cstdint>
#include <memory>

#include "rust/cxx.h"

namespace openmind::native {

// Opaque Rust type declared by cxx. C++ only borrows it for the duration of a
// synchronous generation call and returns raw token bytes through rust::Fn.
// Raw bytes are intentional: a llama token can split a multi-byte UTF-8 code
// point, so Rust assembles valid UTF-8 chunks before forwarding them to UI.
class TokenSink;

class InferenceEngine final {
 public:
  ~InferenceEngine();
  InferenceEngine(InferenceEngine&&) noexcept;
  InferenceEngine& operator=(InferenceEngine&&) noexcept;

  InferenceEngine(const InferenceEngine&) = delete;
  InferenceEngine& operator=(const InferenceEngine&) = delete;

 private:
  class Impl;
  explicit InferenceEngine(std::unique_ptr<Impl> impl) noexcept;

  std::unique_ptr<Impl> impl_;

  friend std::unique_ptr<InferenceEngine> load_model(
      rust::Str model_path,
      std::uint32_t base_context_tokens,
      std::int32_t gpu_layers);
  friend void generate_stream(
      InferenceEngine& engine,
      rust::Str prompt,
      rust::Str system_prompt,
      float temperature,
      float top_p,
      std::uint32_t max_tokens,
      const TokenSink& sink,
      rust::Fn<void(const TokenSink&, rust::Slice<const std::uint8_t>)> on_token);
};

std::unique_ptr<InferenceEngine> load_model(
    rust::Str model_path,
    std::uint32_t base_context_tokens,
    std::int32_t gpu_layers);

void generate_stream(
    InferenceEngine& engine,
    rust::Str prompt,
    rust::Str system_prompt,
    float temperature,
    float top_p,
    std::uint32_t max_tokens,
    const TokenSink& sink,
    rust::Fn<void(const TokenSink&, rust::Slice<const std::uint8_t>)> on_token);

}  // namespace openmind::native
