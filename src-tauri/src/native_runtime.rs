pub const LLAMA_CPP_COMMIT: &str = env!("OPENMINDAI_NATIVE_LLAMA_COMMIT");
pub const ABI_TAG: &str = env!("OPENMINDAI_NATIVE_ABI_TAG");

pub const WINDOWS_REQUIRED_DLLS: &[&str] =
    &["llama.dll", "ggml.dll", "ggml-base.dll", "ggml-cpu.dll"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeRuntimeContract {
    pub abi_tag: &'static str,
    pub llama_cpp_commit: &'static str,
}

pub const fn contract() -> NativeRuntimeContract {
    NativeRuntimeContract {
        abi_tag: ABI_TAG,
        llama_cpp_commit: LLAMA_CPP_COMMIT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_tag_is_derived_from_pinned_commit() {
        assert_eq!(LLAMA_CPP_COMMIT.len(), 40);
        assert_eq!(ABI_TAG, format!("llama-cxx-{}", &LLAMA_CPP_COMMIT[..12]));
    }

    #[test]
    fn windows_bundle_contract_contains_core_libraries() {
        for required in ["llama.dll", "ggml.dll", "ggml-base.dll", "ggml-cpu.dll"] {
            assert!(WINDOWS_REQUIRED_DLLS.contains(&required));
        }
    }
}
