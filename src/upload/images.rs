use base64::{engine::general_purpose::STANDARD, Engine};
use image::imageops::FilterType;
use std::io::Cursor;

use crate::models::ImageSource;

const MAX_DIMENSION: u32 = 720;
const JPEG_QUALITY: u8 = 80;

/// Compress and resize an image, returning a new ImageSource encoded as JPEG.
/// Returns the original unchanged if decoding fails.
pub fn compress_image(source: &ImageSource) -> ImageSource {
    match try_compress(source) {
        Ok(compressed) => compressed,
        Err(_) => source.clone(),
    }
}

fn try_compress(source: &ImageSource) -> Result<ImageSource, Box<dyn std::error::Error>> {
    let raw = STANDARD.decode(&source.data)?;
    let img = image::load_from_memory(&raw)?;

    let (w, h) = (img.width(), img.height());
    let img = if w > MAX_DIMENSION || h > MAX_DIMENSION {
        img.resize(MAX_DIMENSION, MAX_DIMENSION, FilterType::Lanczos3)
    } else {
        img
    };

    let mut buf = Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY);
    img.write_with_encoder(encoder)?;

    let data = STANDARD.encode(buf.into_inner());

    Ok(ImageSource {
        source_type: "base64".to_string(),
        media_type: "image/jpeg".to_string(),
        data,
    })
}
