#pragma once

#include <cstdint>
#include <memory>

#include "rust/cxx.h"

namespace openmind::native {

// Opaque Rust type declared by cxx. C++ only borrows it for the duration of a
// synchronous generation call and returns each token through rust::Fn.
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
      rust::Fn<void(const TokenSink&, rust::Str)> on_token);
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
    rust::Fn<void(const TokenSink&, rust::Str)> on_token);

}  // namespace openmind::native
