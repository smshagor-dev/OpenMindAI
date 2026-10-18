// Regression tests copied into a temporary, ZIP 9-patched candle-core tree.
//
// These are deliberately minimal test archives, not proof of loading an
// actual pretrained PyTorch model. Run only under the isolated migration
// probe; production still uses published dependencies until audited.
use candle_core::{npy::NpzTensors, pickle::read_pth_tensor_info, DType, Device, Tensor};
use std::{
    error::Error,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct ScratchFile(PathBuf);

impl ScratchFile {
    fn new(suffix: &str) -> Self {
        let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "openmind-zip9-reader-{}-{}.{suffix}",
            std::process::id(),
            sequence
        );
        Self(std::env::temp_dir().join(name))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn zip9_npz_eager_and_lazy_tensors_round_trip() -> Result<(), Box<dyn Error>> {
    let archive = ScratchFile::new("npz");
    let original = Tensor::new(&[1.0_f32, 2.5, -3.0], &Device::Cpu)?;
    Tensor::write_npz(&[("weights", &original)], archive.path())?;

    let eager = Tensor::read_npz(archive.path())?;
    assert_eq!(eager.len(), 1);
    assert_eq!(eager[0].0, "weights");
    assert_eq!(eager[0].1.to_vec1::<f32>()?, vec![1.0, 2.5, -3.0]);

    let lazy = NpzTensors::new(archive.path())?;
    assert_eq!(lazy.names().len(), 1);
    let (shape, dtype) = lazy.get_shape_and_dtype("weights")?;
    assert_eq!(shape.dims(), &[3]);
    assert_eq!(dtype, DType::F32);
    let restored = lazy.get("weights")?.expect("NPZ tensor was not found");
    assert_eq!(restored.to_vec1::<f32>()?, vec![1.0, 2.5, -3.0]);
    Ok(())
}

#[test]
fn zip9_pth_reader_parses_minimal_pickle_dictionary() -> Result<(), Box<dyn Error>> {
    // A PyTorch-style ZIP root with data.pkl containing a protocol-2 empty
    // dict: PROTO(2), EMPTY_DICT, STOP. This exercises ZIP filename access
    // and the Candle pickle parser, but not real weight storage recovery.
    let archive = ScratchFile::new("pth");
    let mut writer = zip::ZipWriter::new(File::create(archive.path())?);
    let options = zip::write::FileOptions::<()>::default()
        .compression_method(zip::CompressionMethod::Stored);
    writer.start_file("synthetic/data.pkl", options)?;
    writer.write_all(&[0x80, 0x02, b'}', b'.'])?;
    writer.finish()?;

    let tensors = read_pth_tensor_info(archive.path(), false, None)?;
    assert!(tensors.is_empty());
    Ok(())
}

#[test]
fn zip9_npz_and_pth_readers_reject_non_zip_bytes() -> Result<(), Box<dyn Error>> {
    let archive = ScratchFile::new("zip");
    fs::write(archive.path(), b"plain text is not a ZIP archive")?;

    assert!(Tensor::read_npz(archive.path()).is_err());
    assert!(NpzTensors::new(archive.path()).is_err());
    assert!(read_pth_tensor_info(archive.path(), false, None).is_err());
    Ok(())
}
