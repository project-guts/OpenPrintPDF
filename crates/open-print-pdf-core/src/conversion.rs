use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderingIntent {
    Perceptual,
    RelativeColorimetric,
    Saturation,
    AbsoluteColorimetric,
}

impl RenderingIntent {
    pub fn ghostscript_value(self) -> u8 {
        match self {
            Self::Perceptual => 0,
            Self::RelativeColorimetric => 1,
            Self::Saturation => 2,
            Self::AbsoluteColorimetric => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionMode {
    OutlineOnly,
    PdfX1a {
        outline_fonts: bool,
        rendering_intent: RenderingIntent,
        black_point_compensation: bool,
        transparency_resolution_dpi: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IccProfileSource {
    InputOutputIntent,
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct ConversionRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub mode: ConversionMode,
    pub destination_icc: IccProfileSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub name: String,
    pub version: String,
    pub executable: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorConversionReport {
    pub destination_profile_sha256: String,
    pub destination_profile_description: Option<String>,
    pub rendering_intent: RenderingIntent,
    pub black_point_compensation: bool,
    pub assumed_srgb_images: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionResult {
    pub output_path: PathBuf,
    pub engine: EngineInfo,
    pub duration_ms: u64,
    pub flattened_pages: Vec<u32>,
    pub color_conversion: Option<ColorConversionReport>,
    pub engine_warnings: Vec<String>,
}

pub trait PrintConversionEngine {
    fn engine_info(&self) -> Result<EngineInfo>;
    fn convert(&self, request: &ConversionRequest) -> Result<ConversionResult>;
}

pub fn ensure_distinct_paths(input: &Path, output: &Path) -> Result<()> {
    let input = input.canonicalize().unwrap_or_else(|_| input.to_path_buf());
    let output = output
        .canonicalize()
        .unwrap_or_else(|_| output.to_path_buf());
    if input == output {
        return Err(crate::OpenPrintPdfError::SameInputAndOutput);
    }
    Ok(())
}
