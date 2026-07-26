use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PdfBox {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

impl PdfBox {
    pub fn width(self) -> f64 {
        self.right - self.left
    }

    pub fn height(self) -> f64 {
        self.top - self.bottom
    }

    pub fn normalized_to(self, media: Self) -> Self {
        Self {
            left: self.left - media.left,
            bottom: self.bottom - media.bottom,
            right: self.right - media.left,
            top: self.top - media.bottom,
        }
    }

    pub fn approximately_eq(self, other: Self, epsilon: f64) -> bool {
        (self.left - other.left).abs() <= epsilon
            && (self.bottom - other.bottom).abs() <= epsilon
            && (self.right - other.right).abs() <= epsilon
            && (self.top - other.top).abs() <= epsilon
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageGeometry {
    pub page_number: u32,
    pub media_box: PdfBox,
    pub crop_box: Option<PdfBox>,
    pub trim_box: Option<PdfBox>,
    pub bleed_box: Option<PdfBox>,
    pub art_box: Option<PdfBox>,
    pub rotation: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontInfo {
    pub page_number: u32,
    pub resource_name: String,
    pub base_name: Option<String>,
    pub subtype: Option<String>,
    pub embedded: bool,
    pub subset: bool,
    pub has_to_unicode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageColorSpace {
    DeviceRgb,
    DeviceCmyk,
    DeviceGray,
    IccRgb,
    IccCmyk,
    IccGray,
    Indexed,
    Separation,
    DeviceN,
    Unknown,
}

impl ImageColorSpace {
    pub fn is_rgb(self) -> bool {
        matches!(self, Self::DeviceRgb | Self::IccRgb)
    }

    pub fn is_cmyk(self) -> bool {
        matches!(self, Self::DeviceCmyk | Self::IccCmyk)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfo {
    pub page_number: u32,
    pub resource_name: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bits_per_component: Option<i64>,
    pub color_space: ImageColorSpace,
    pub has_soft_mask: bool,
    pub has_image_mask: bool,
    pub icc_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransparencyKind {
    SoftMask,
    ConstantAlpha,
    TransparencyGroup,
    BlendMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransparencyFinding {
    pub page_number: u32,
    pub kind: TransparencyKind,
    pub resource_name: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IccProfileInfo {
    pub byte_length: usize,
    pub sha256: String,
    pub version: Option<String>,
    pub device_class: Option<String>,
    pub color_space: Option<String>,
    pub pcs: Option<String>,
    pub preferred_intent: Option<u32>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputIntentInfo {
    pub subtype: Option<String>,
    pub output_condition: Option<String>,
    pub output_condition_identifier: Option<String>,
    pub info: Option<String>,
    pub registry_name: Option<String>,
    pub profile: Option<IccProfileInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfInspectionResult {
    pub path: PathBuf,
    pub pdf_version: String,
    pub pdfx_claim: Option<String>,
    pub page_count: usize,
    pub pages: Vec<PageGeometry>,
    pub fonts: Vec<FontInfo>,
    pub images: Vec<ImageInfo>,
    pub transparency: Vec<TransparencyFinding>,
    pub output_intent: Option<OutputIntentInfo>,
}

impl PdfInspectionResult {
    pub fn rgb_image_count(&self) -> usize {
        self.images
            .iter()
            .filter(|image| image.color_space.is_rgb())
            .count()
    }

    pub fn cmyk_image_count(&self) -> usize {
        self.images
            .iter()
            .filter(|image| image.color_space.is_cmyk())
            .count()
    }
}

#[derive(Debug, Clone)]
pub struct OutputIntentData {
    pub info: OutputIntentInfo,
    pub profile_bytes: Vec<u8>,
}
