//! Validate local diffusion PNG output before exposing an artifact as ready.
//!
//! Signature and file-size checks alone accept a truncated image or arbitrary
//! data that only happens to begin with the PNG magic bytes. We validate the
//! PNG chunk structure, CRCs, dimensions, and a complete bounded zlib scanline
//! stream instead. No extra image-decoder dependency is needed.

use std::{
    fs,
    io::{Read, ErrorKind},
    path::Path,
};

use flate2::read::ZlibDecoder;

use crate::app_error::AppError;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const MAX_PNG_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PNG_DIMENSION: u32 = 8192;
const MAX_PNG_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_DECOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Header {
    width: u32,
    height: u32,
    bits: u8,
    color: u8,
    interlace: u8,
}

fn invalid(detail: &str) -> AppError {
    AppError::ArtifactGenerationFailed(format!(
        "image renderer returned an invalid PNG: {detail}"
    ))
}

fn crc32(kind: &[u8], data: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in kind.iter().chain(data) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 == 1 { 0xedb8_8320 } else { 0 };
        }
    }
    !crc
}

fn header_from_ihdr(data: &[u8]) -> Result<Header, AppError> {
    if data.len() != 13 {
        return Err(invalid("IHDR must be 13 bytes"));
    }
    let width = u32::from_be_bytes(data[0..4].try_into().unwrap());
    let height = u32::from_be_bytes(data[4..8].try_into().unwrap());
    let bits = data[8];
    let color = data[9];
    let valid_bits = match color {
        0 => matches!(bits, 1 | 2 | 4 | 8 | 16),
        2 | 4 | 6 => matches!(bits, 8 | 16),
        3 => matches!(bits, 1 | 2 | 4 | 8),
        _ => false,
    };
    if width == 0
        || height == 0
        || width > MAX_PNG_DIMENSION
        || height > MAX_PNG_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PNG_PIXELS
        || !valid_bits
        || data[10] != 0
        || data[11] != 0
        || data[12] > 1
    {
        return Err(invalid("unsupported dimensions, color format, or PNG header"));
    }
    Ok(Header {
        width,
        height,
        bits,
        color,
        interlace: data[12],
    })
}

fn scanline_dimensions(size: u32, start: u32, step: u32) -> u64 {
    if size <= start {
        0
    } else {
        u64::from(size - start).div_ceil(u64::from(step))
    }
}

fn verify_image_stream(header: Header, compressed: &[u8]) -> Result<(), AppError> {
    let channels = match header.color {
        0 | 3 => 1_u64,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return Err(invalid("unsupported color format")),
    };
    let passes: &[(u32, u32, u32, u32)] = if header.interlace == 0 {
        &[(0, 0, 1, 1)]
    } else {
        &[
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ]
    };

    let mut decoder = ZlibDecoder::new(compressed);
    let mut expanded = 0_u64;
    for &(left, top, x_step, y_step) in passes {
        let width = scanline_dimensions(header.width, left, x_step);
        let height = scanline_dimensions(header.height, top, y_step);
        if width == 0 || height == 0 {
            continue;
        }
        let row_bits = width
            .checked_mul(channels)
            .and_then(|v| v.checked_mul(u64::from(header.bits)))
            .ok_or_else(|| invalid("scanline size overflow"))?;
        let row_len = row_bits.div_ceil(8);
        let row_with_filter = row_len
            .checked_add(1)
            .ok_or_else(|| invalid("scanline size overflow"))?;
        expanded = expanded
            .checked_add(
                row_with_filter
                    .checked_mul(height)
                    .ok_or_else(|| invalid("scanline size overflow"))?,
            )
            .ok_or_else(|| invalid("scanline size overflow"))?;
        if expanded > MAX_DECOMPRESSED_BYTES {
            return Err(invalid("decoded pixel data exceeds memory limit"));
        }
        let buffer_len = usize::try_from(row_with_filter)
            .map_err(|_| invalid("scanline is too large"))?;
        let mut row = vec![0_u8; buffer_len];
        for _ in 0..height {
            decoder
                .read_exact(&mut row)
                .map_err(|_| invalid("PNG scanline data is truncated or corrupt"))?;
            if row[0] > 4 {
                return Err(invalid("invalid PNG scanline filter"));
            }
        }
    }

    // A valid PNG's decoded data must end exactly after its final scanline.
    let mut extra = [0_u8; 1];
    match decoder.read(&mut extra) {
        Ok(0) => Ok(()),
        Ok(_) => Err(invalid("PNG contains extra decompressed image data")),
        Err(_) => Err(invalid("PNG compressed data failed integrity verification")),
    }
}

pub(crate) fn validate_png(path: &Path) -> Result<(), AppError> {
    let size = fs::metadata(path)
        .map_err(|e| AppError::ArtifactGenerationFailed(format!("image output was not created: {e}")))?
        .len();
    if size < 45 || size > MAX_PNG_FILE_BYTES {
        return Err(invalid("file size is outside supported bounds"));
    }
    let file = fs::read(path)?;
    if !file.starts_with(PNG_SIGNATURE) {
        return Err(invalid("missing PNG signature"));
    }

    let mut pos = PNG_SIGNATURE.len();
    let mut header = None;
    let mut compressed = Vec::<u8>::new();
    let mut has_idat = false;
    let mut idat_closed = false;
    let mut seen_palette = false;
    let mut completed = false;
    while pos < file.len() {
        if file.len() - pos < 12 {
            return Err(invalid("truncated PNG chunk header"));
        }
        let length = u32::from_be_bytes(
            file[pos..pos + 4]
                .try_into()
                .map_err(|_| invalid("truncated PNG chunk length"))?,
        ) as usize;
        let next = pos
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .ok_or_else(|| invalid("PNG chunk length overflow"))?;
        if next > file.len() {
            return Err(invalid("truncated PNG chunk body"));
        }
        let kind = &file[pos + 4..pos + 8];
        let payload = &file[pos + 8..pos + 8 + length];
        let supplied_crc =
            u32::from_be_bytes(file[next - 4..next].try_into().unwrap());
        if crc32(kind, payload) != supplied_crc {
            return Err(invalid("PNG chunk checksum mismatch"));
        }
        if header.is_none() && kind != b"IHDR" {
            return Err(invalid("IHDR must be the first PNG chunk"));
        }
        if has_idat && kind != b"IDAT" {
            idat_closed = true;
        }

        if kind == b"IHDR" {
            if header.is_some() || pos != PNG_SIGNATURE.len() {
                return Err(invalid("duplicate or misplaced IHDR"));
            }
            header = Some(header_from_ihdr(payload)?);
        } else if kind == b"PLTE" {
            if has_idat {
                return Err(invalid("palette appears after image data"));
            }
            seen_palette = true;
        } else if kind == b"IDAT" {
            if idat_closed {
                return Err(invalid("non-contiguous IDAT chunks"));
            }
            compressed.extend_from_slice(payload);
            has_idat = true;
        } else if kind == b"IEND" {
            if length != 0 || !has_idat || next != file.len() {
                return Err(invalid("misplaced IEND or trailing bytes"));
            }
            completed = true;
            break;
        } else if kind[0] & 0x20 == 0 {
            return Err(invalid("unsupported critical PNG chunk"));
        }
        pos = next;
    }
    let header = header.ok_or_else(|| invalid("missing PNG header"))?;
    if !completed || !has_idat {
        return Err(invalid("missing image data or IEND"));
    }
    if header.color == 3 && !seen_palette {
        return Err(invalid("indexed PNG is missing its palette"));
    }
    verify_image_stream(header, &compressed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;

    fn append_chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out.extend_from_slice(&crc32(kind, payload).to_be_bytes());
    }

    fn png_with_scanline(scanline: &[u8], width: u32) -> Vec<u8> {
        let mut out = PNG_SIGNATURE.to_vec();
        let mut ihdr = Vec::from(width.to_be_bytes());
        ihdr.extend_from_slice(&1_u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB, no interlace
        append_chunk(&mut out, b"IHDR", &ihdr);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(scanline).unwrap();
        let compressed = encoder.finish().unwrap();
        append_chunk(&mut out, b"IDAT", &compressed);
        append_chunk(&mut out, b"IEND", &[]);
        out
    }

    fn validate(data: &[u8]) -> Result<(), AppError> {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("generated.png");
        fs::write(&path, data).unwrap();
        validate_png(&path)
    }

    #[test]
    fn accepts_valid_decoded_png() {
        assert!(validate(&png_with_scanline(&[0, 255, 64, 0], 1)).is_ok());
    }

    #[test]
    fn rejects_png_signature_with_unparsed_filler() {
        let mut data = PNG_SIGNATURE.to_vec();
        data.resize(2048, 0);
        assert!(validate(&data).is_err());
    }

    #[test]
    fn rejects_tampered_crc() {
        let mut data = png_with_scanline(&[0, 255, 64, 0], 1);
        data[29] ^= 1;
        assert!(validate(&data).is_err());
    }

    #[test]
    fn rejects_invalid_dimensions_even_with_correct_chunk_crc() {
        assert!(validate(&png_with_scanline(&[0, 255, 64, 0], 0)).is_err());
    }

    #[test]
    fn rejects_truncated_pixel_stream() {
        assert!(validate(&png_with_scanline(&[0, 255, 64], 1)).is_err());
    }

    #[test]
    fn rejects_invalid_scanline_filter() {
        assert!(validate(&png_with_scanline(&[9, 255, 64, 0], 1)).is_err());
    }

    #[test]
    fn rejects_appended_bytes_after_iend() {
        let mut data = png_with_scanline(&[0, 255, 64, 0], 1);
        data.push(42);
        assert!(validate(&data).is_err());
    }
}
