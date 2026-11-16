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


// data.pkl from a tiny PyTorch 2.x checkpoint generated with:
//
// torch.save({"weight": torch.tensor([1.0, 2.5, -3.0],
//                                   dtype=torch.float32)},
//            file, pickle_protocol=2)
//
// Unlike the minimal empty dictionary test above, this pickle describes a
// tensor whose f32 storage lives in a separate archive/data/0 ZIP entry.
// The archive is rebuilt here without a Python or PyTorch CI dependency.
const TORCH_TENSOR_PICKLE_HEX: &str = concat!(
    "80027d71005806000000776569676874710163746f7263682e5f7574696c730a",
    "5f72656275696c645f74656e736f725f76320a71022828580700000073746f",
    "72616765710363746f7263680a466c6f617453746f726167650a7104580100",
    "0000307105580300000063707571064b03747107514b004b038571084b0185",
    "71098963636f6c6c656374696f6e730a4f726465726564446963740a710a29",
    "52710b74710c52710d732e"
);

fn decode_fixture_hex(s: &str) -> Vec<u8> {
    assert_eq!(s.len() % 2, 0, "hex fixture must contain complete bytes");
    s.as_bytes()
        .chunks_exact(2)
        .map(|part| u8::from_str_radix(std::str::from_utf8(part).unwrap(), 16).unwrap())
        .collect()
}

fn write_pytorch_tensor_archive(path: &Path, include_storage: bool) -> Result<(), Box<dyn Error>> {
    let mut writer = zip::ZipWriter::new(File::create(path)?);
    fn add(writer: &mut zip::ZipWriter<File>, name: &str, data: &[u8])
        -> Result<(), Box<dyn Error>>
    {
        writer.start_file(
            name,
            zip::write::FileOptions::<()>::default()
                .compression_method(zip::CompressionMethod::Stored),
        )?;
        writer.write_all(data)?;
        Ok(())
    }
    add(&mut writer, "archive/data.pkl", &decode_fixture_hex(TORCH_TENSOR_PICKLE_HEX))?;
    add(&mut writer, "archive/byteorder", b"little")?;
    add(&mut writer, "archive/version", b"3\n")?;
    if include_storage {
        // This little-endian byte sequence is the actual 3-element f32 storage.
        let mut data = Vec::new();
        for value in [1.0_f32, 2.5, -3.0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        add(&mut writer, "archive/data/0", &data)?;
    }
    writer.finish()?;
    Ok(())
}

#[test]
fn zip9_pth_reads_real_nonempty_torch_storage_layout() -> Result<(), Box<dyn Error>> {
    let archive = ScratchFile::new("pth");
    write_pytorch_tensor_archive(archive.path(), true)?;

    let metadata = read_pth_tensor_info(archive.path(), false, None)?;
    assert_eq!(metadata.len(), 1, "expected the saved PyTorch weight tensor");

    let tensors = candle_core::pickle::read_all(archive.path())?;
    assert_eq!(tensors.len(), 1);
    assert_eq!(tensors[0].0, "weight");
    assert_eq!(tensors[0].1.to_vec1::<f32>()?, vec![1.0, 2.5, -3.0]);
    Ok(())
}

#[test]
fn zip9_pth_rejects_missing_tensor_storage() -> Result<(), Box<dyn Error>> {
    let archive = ScratchFile::new("pth");
    write_pytorch_tensor_archive(archive.path(), false)?;
    // A valid data.pkl does not make the checkpoint valid when data/0 is
    // missing; tensor loaders must never invent or silently zero the weights.
    assert!(candle_core::pickle::read_all(archive.path()).is_err());
    Ok(())
}
