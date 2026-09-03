//! Explicit, local adapter activation. Candidate bytes are verified at model load.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAdapterSpec {
    pub path: PathBuf,
    pub sha256: String,
    pub base_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Activation {
    schema_version: u32,
    profile_id: String,
    model_id: String,
    candidate: Option<PathBuf>,
}
#[derive(Deserialize)]
struct Candidate {
    schema_version: u32,
    profile_id: String,
    model_id: String,
    base_sha256: String,
    adapter_path: PathBuf,
    adapter_sha256: String,
    evaluation: Evaluation,
}
#[derive(Deserialize)]
struct Evaluation {
    accepted: bool,
    baseline_loss: f64,
    candidate_loss: f64,
    holdout_examples: usize,
}
fn bounded_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    if !path.is_absolute() {
        return Err("adapter manifest path must be absolute".into());
    }
    let mut data = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(65537)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > 65536 {
        return Err("adapter manifest is too large".into());
    }
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}
pub fn resolve(activation: &Path, model_id: &str) -> Result<Option<NativeAdapterSpec>, String> {
    let active: Activation = bounded_json(activation)?;
    if active.schema_version != 1 || active.model_id != model_id || active.profile_id.is_empty() {
        return Err("adapter activation identity mismatch".into());
    }
    let Some(path) = active.candidate else {
        return Ok(None);
    };
    let candidate: Candidate = bounded_json(&path)?;
    let e = &candidate.evaluation;
    if candidate.schema_version != 1
        || candidate.profile_id != active.profile_id
        || candidate.model_id != model_id
        || !candidate.adapter_path.is_absolute()
        || !e.accepted
        || !e.baseline_loss.is_finite()
        || !e.candidate_loss.is_finite()
        || e.candidate_loss < 0.0
        || e.candidate_loss >= e.baseline_loss
        || e.holdout_examples < 12
    {
        return Err("adapter has not passed its identity and holdout evaluation gates".into());
    }
    for digest in [&candidate.base_sha256, &candidate.adapter_sha256] {
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid adapter hash".into());
        }
    }
    Ok(Some(NativeAdapterSpec {
        path: candidate.adapter_path,
        sha256: candidate.adapter_sha256,
        base_sha256: candidate.base_sha256,
    }))
}
pub fn hash_file(path: &Path, max_bytes: u64) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size < 4 || size > max_bytes {
        return Err("adapter or base model size outside supported limits".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    let mut read = 0_u64;
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        read += n as u64;
        if read > max_bytes {
            return Err("file grew beyond limit".into());
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
impl NativeAdapterSpec {
    pub fn verify(&self, model: &Path) -> Result<(), String> {
        if hash_file(model, 8 * 1024 * 1024 * 1024)? != self.base_sha256
            || hash_file(&self.path, 512 * 1024 * 1024)? != self.sha256
        {
            return Err("adapter or base model checksum mismatch".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn identity_evaluation_and_checksums_gate_loading() {
        let root = std::env::temp_dir().join(format!(
            "openmind-adapter-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let model = root.join("base.gguf");
        let adapter = root.join("adapter.gguf");
        fs::write(&model, b"GGUFbase").unwrap();
        fs::write(&adapter, b"GGUFadapter").unwrap();
        let candidate = root.join("candidate.json");
        let active = root.join("active.json");
        let mut data = json!({"schema_version":1,"profile_id":"local","model_id":"nano","base_sha256":hash_file(&model,1024).unwrap(),"adapter_path":adapter,"adapter_sha256":hash_file(&adapter,1024).unwrap(),"evaluation":{"accepted":true,"baseline_loss":3.0,"candidate_loss":2.0,"holdout_examples":12}});
        fs::write(&candidate, data.to_string()).unwrap();
        fs::write(&active,json!({"schema_version":1,"profile_id":"local","model_id":"nano","candidate":candidate}).to_string()).unwrap();
        let spec = resolve(&active, "nano").unwrap().unwrap();
        assert!(spec.verify(&model).is_ok());
        assert!(resolve(&active, "other").is_err());
        fs::write(&adapter, b"GGUFtampered").unwrap();
        assert!(spec.verify(&model).is_err());
        data["evaluation"]["candidate_loss"] = json!(4.0);
        fs::write(&candidate, data.to_string()).unwrap();
        assert!(resolve(&active, "nano").is_err());
        fs::write(
            &active,
            json!({"schema_version":1,"profile_id":"local","model_id":"nano","candidate":null})
                .to_string(),
        )
        .unwrap();
        assert!(resolve(&active, "nano").unwrap().is_none());
    }
}
