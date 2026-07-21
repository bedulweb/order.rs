//! DIY ddddocr-compatible captcha OCR.
//!
//! Findings baked in:
//! - Use Python default model `common_old.onnx` (not beta `common.onnx`)
//! - Preprocess: resize h=64 Lanczos3, grayscale, **pixel/255 only** (not mean-std)
//! - Output is f32 logits `[seq, 1, 8210]` → argmax + CTC (blank=0, collapse repeats)
//! - crates.io `ddddocr` 0.1.0 is broken (extracts i64); 86maid was slower/less accurate here
//! - Latency: ~93% in ONNX run; use multi-thread ORT + warmup

use crate::error::{Error, Result};
use image::{imageops::FilterType, load_from_memory};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Value;
use std::path::Path;
use std::sync::Mutex;
use tracing::{debug, info};

/// Offline captcha OCR backed by ONNX Runtime.
pub struct CaptchaOcr {
    session: Mutex<Session>,
    charset: Vec<String>,
}

impl CaptchaOcr {
    /// Load model + charset. Call [`warmup`] once after construction in long-lived processes.
    pub fn load(model_path: &Path, charset_path: &Path, intra_threads: usize) -> Result<Self> {
        let charset = load_charset(charset_path)?;
        if charset.len() < 2 {
            return Err(Error::Ocr("charset too short".into()));
        }

        let threads = intra_threads.max(1);
        info!(
            model = %model_path.display(),
            threads,
            charset_len = charset.len(),
            "loading OCR model"
        );

        let session = Session::builder()
            .map_err(|e| Error::Ocr(e.to_string()))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| Error::Ocr(e.to_string()))?
            .with_intra_threads(threads)
            .map_err(|e| Error::Ocr(e.to_string()))?
            .with_inter_threads(1)
            .map_err(|e| Error::Ocr(e.to_string()))?
            .commit_from_file(model_path)
            .map_err(|e| Error::Ocr(e.to_string()))?;

        Ok(Self {
            session: Mutex::new(session),
            charset,
        })
    }

    /// Run a dummy inference so the first real captcha is not cold.
    pub fn warmup(&self) -> Result<()> {
        // 1x64x8 gray PNG-ish synthetic via tiny image
        let mut img = image::GrayImage::new(32, 16);
        for p in img.pixels_mut() {
            p.0 = [255];
        }
        let mut png = Vec::new();
        {
            let dynimg = image::DynamicImage::ImageLuma8(img);
            dynimg
                .write_to(
                    &mut std::io::Cursor::new(&mut png),
                    image::ImageFormat::Png,
                )
                .map_err(|e| Error::Ocr(e.to_string()))?;
        }
        let _ = self.classify_bytes(&png)?;
        debug!("OCR warmup done");
        Ok(())
    }

    /// Recognize captcha text from raw image bytes (PNG/JPEG).
    pub fn classify_bytes(&self, png: &[u8]) -> Result<String> {
        let img = load_from_memory(png)?;
        let new_width = ((img.width() as f32) * (64.0 / img.height() as f32)) as u32;
        let resized = img.resize_exact(new_width.max(1), 64, FilterType::Lanczos3);
        let gray = resized.to_luma8();
        let (h, w) = (gray.height() as usize, gray.width() as usize);

        // Python ddddocr: pixel / 255.0 only
        let mut data = Vec::with_capacity(h * w);
        for &px in gray.as_raw() {
            data.push(px as f32 * (1.0 / 255.0));
        }

        let input = Value::from_array((vec![1usize, 1, h, w], data))
            .map_err(|e| Error::Ocr(e.to_string()))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| Error::Ocr("OCR session lock poisoned".into()))?;

        let outs = session
            .run(ort::inputs!["input1" => input])
            .map_err(|e| Error::Ocr(e.to_string()))?;

        let (shape, logits) = outs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| Error::Ocr(format!("expected f32 logits: {e}")))?;

        Ok(ctc_decode(shape.as_ref(), logits, &self.charset))
    }
}

fn load_charset(path: &Path) -> Result<Vec<String>> {
    let raw = std::fs::read_to_string(path)?;
    let chars: Vec<String> = serde_json::from_str(&raw)?;
    Ok(chars)
}

/// Greedy CTC: argmax per timestep, skip blank (0) and consecutive duplicates.
fn ctc_decode(shape: &[i64], data: &[f32], charset: &[String]) -> String {
    let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
    let (seq, nclass) = match dims.as_slice() {
        [s, 1, c] if *c > 100 => (*s, *c),
        [1, s, c] if *c > 100 => (*s, *c),
        d => {
            let c = *d.last().unwrap_or(&1);
            (data.len() / c.max(1), c)
        }
    };

    let mut out = String::with_capacity(8);
    let mut last = -1i64;
    for t in 0..seq {
        let base = t * nclass;
        if base + nclass > data.len() {
            break;
        }
        let sl = &data[base..base + nclass];
        let (mut bi, mut bv) = (0usize, f32::NEG_INFINITY);
        for (i, &v) in sl.iter().enumerate() {
            if v > bv {
                bv = v;
                bi = i;
            }
        }
        let item = bi as i64;
        if item == last {
            continue;
        }
        last = item;
        if item == 0 {
            continue;
        }
        if let Some(ch) = charset.get(item as usize) {
            if !ch.is_empty() {
                out.push_str(ch);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctc_simple() {
        // one timestep, class 3 wins; charset[3] = "a"
        let charset = vec![
            "".into(),
            "x".into(),
            "y".into(),
            "a".into(),
            "b".into(),
        ];
        // shape [1,1,5], logits favor index 3
        let logits = [0.0f32, 0.1, 0.2, 9.0, 0.3];
        let shape = [1i64, 1, 5];
        assert_eq!(ctc_decode(&shape, &logits, &charset), "a");
    }
}
