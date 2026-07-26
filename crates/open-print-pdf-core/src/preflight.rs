use serde::{Deserialize, Serialize};

use crate::{ImageColorSpace, PdfInspectionResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightRuleId {
    Type3FontDetected,
    FontNotEmbedded,
    RgbImageDetected,
    DeviceRgbWithoutProfile,
    TransparencyDetected,
    MissingTrimBox,
    MissingBleedBox,
    MissingOutputIntent,
    OutputIntentNotCmyk,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightFinding {
    pub rule: PreflightRuleId,
    pub severity: Severity,
    pub page_number: Option<u32>,
    pub message: String,
}

pub fn run_preflight(document: &PdfInspectionResult) -> Vec<PreflightFinding> {
    let mut findings = Vec::new();
    for font in &document.fonts {
        if font.subtype.as_deref() == Some("Type3") {
            findings.push(PreflightFinding {
                rule: PreflightRuleId::Type3FontDetected,
                severity: Severity::Warning,
                page_number: Some(font.page_number),
                message: format!("Type 3 font detected: {}", font.resource_name),
            });
        }
        if !font.embedded {
            findings.push(PreflightFinding {
                rule: PreflightRuleId::FontNotEmbedded,
                severity: Severity::Error,
                page_number: Some(font.page_number),
                message: format!("font is not embedded: {}", font.resource_name),
            });
        }
    }
    for image in &document.images {
        if image.color_space.is_rgb() {
            findings.push(PreflightFinding {
                rule: PreflightRuleId::RgbImageDetected,
                severity: Severity::Warning,
                page_number: Some(image.page_number),
                message: format!(
                    "RGB image requires CMYK conversion: {}",
                    image.resource_name
                ),
            });
        }
        if image.color_space == ImageColorSpace::DeviceRgb {
            findings.push(PreflightFinding {
                rule: PreflightRuleId::DeviceRgbWithoutProfile,
                severity: Severity::Warning,
                page_number: Some(image.page_number),
                message: format!(
                    "untagged DeviceRGB image will be treated as sRGB: {}",
                    image.resource_name
                ),
            });
        }
    }
    for transparency in &document.transparency {
        findings.push(PreflightFinding {
            rule: PreflightRuleId::TransparencyDetected,
            severity: Severity::Warning,
            page_number: Some(transparency.page_number),
            message: format!(
                "page {} will be flattened to a CMYK image: {}",
                transparency.page_number, transparency.detail
            ),
        });
    }
    for page in &document.pages {
        if page.trim_box.is_none() {
            findings.push(PreflightFinding {
                rule: PreflightRuleId::MissingTrimBox,
                severity: Severity::Warning,
                page_number: Some(page.page_number),
                message: "TrimBox is missing".into(),
            });
        }
        if page.bleed_box.is_none() {
            findings.push(PreflightFinding {
                rule: PreflightRuleId::MissingBleedBox,
                severity: Severity::Warning,
                page_number: Some(page.page_number),
                message: "BleedBox is missing".into(),
            });
        }
    }
    match &document.output_intent {
        None => findings.push(PreflightFinding {
            rule: PreflightRuleId::MissingOutputIntent,
            severity: Severity::Error,
            page_number: None,
            message: "PDF/X conversion needs a CMYK OutputIntent or an explicit ICC profile".into(),
        }),
        Some(intent)
            if intent
                .profile
                .as_ref()
                .and_then(|profile| profile.color_space.as_deref())
                != Some("CMYK") =>
        {
            findings.push(PreflightFinding {
                rule: PreflightRuleId::OutputIntentNotCmyk,
                severity: Severity::Error,
                page_number: None,
                message: "OutputIntent profile is not CMYK".into(),
            });
        }
        Some(_) => {}
    }
    findings
}
