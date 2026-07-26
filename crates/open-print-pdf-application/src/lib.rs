use std::path::{Path, PathBuf};

use open_print_pdf_conversion_ghostscript::GhostscriptEngine;
use open_print_pdf_core::{
    ConversionMode, ConversionRequest, ConversionResult, ConversionValidationResult, EngineInfo,
    IccProfileSource, OpenPrintPdfError, PdfInspectionResult, PreflightFinding,
    PrintConversionEngine, RenderingIntent, Result, inspect_pdf, restore_page_boxes, run_preflight,
    transparency_pages, validate_conversion, validate_icc_file, validate_new_pdf_output,
    validate_transparency_raster_budget,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineAvailability {
    pub available: bool,
    pub info: Option<EngineInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectionReport {
    pub document: PdfInspectionResult,
    pub findings: Vec<PreflightFinding>,
    pub engine: EngineAvailability,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FullConversionReport {
    pub input: PdfInspectionResult,
    pub conversion: ConversionResult,
    pub output: PdfInspectionResult,
    pub validation: ConversionValidationResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionKind {
    Outline,
    PdfX1a,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionOptions {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub kind: ConversionKind,
    pub destination_icc: Option<PathBuf>,
    pub rendering_intent: RenderingIntent,
    pub black_point_compensation: bool,
    pub outline_fonts: bool,
    pub transparency_resolution_dpi: u16,
    pub allow_transparency: bool,
}

pub fn inspect(input: &Path, ghostscript: Option<&Path>) -> Result<InspectionReport> {
    let document = inspect_pdf(input)?;
    let findings = run_preflight(&document);
    let engine = engine_availability(ghostscript);
    Ok(InspectionReport {
        document,
        findings,
        engine,
    })
}

pub fn engine_availability(ghostscript: Option<&Path>) -> EngineAvailability {
    match GhostscriptEngine::detect(ghostscript).and_then(|engine| engine.engine_info()) {
        Ok(info) => EngineAvailability {
            available: true,
            info: Some(info),
            error: None,
        },
        Err(error) => EngineAvailability {
            available: false,
            info: None,
            error: Some(error.to_string()),
        },
    }
}

pub fn convert(
    options: &ConversionOptions,
    ghostscript: Option<&Path>,
) -> Result<FullConversionReport> {
    if !(72..=2400).contains(&options.transparency_resolution_dpi) {
        return Err(OpenPrintPdfError::ConversionFailed(
            "transparency resolution must be between 72 and 2400 dpi".into(),
        ));
    }
    validate_new_pdf_output(&options.output_path)?;
    if let Some(profile) = &options.destination_icc {
        validate_icc_file(profile)?;
    }
    let input = inspect_pdf(&options.input_path)?;
    let transparent_pages = transparency_pages(&input);
    ensure_transparency_allowed(options.kind, &transparent_pages, options.allow_transparency)?;
    if options.kind == ConversionKind::PdfX1a && !transparent_pages.is_empty() {
        validate_transparency_raster_budget(
            &input,
            &transparent_pages,
            options.transparency_resolution_dpi,
        )?;
    }
    let mode = match options.kind {
        ConversionKind::Outline => ConversionMode::OutlineOnly,
        ConversionKind::PdfX1a => ConversionMode::PdfX1a {
            outline_fonts: options.outline_fonts,
            rendering_intent: options.rendering_intent,
            black_point_compensation: options.black_point_compensation,
            transparency_resolution_dpi: options.transparency_resolution_dpi,
        },
    };
    let request = ConversionRequest {
        input_path: options.input_path.clone(),
        output_path: options.output_path.clone(),
        mode: mode.clone(),
        destination_icc: options
            .destination_icc
            .clone()
            .map(IccProfileSource::File)
            .unwrap_or(IccProfileSource::InputOutputIntent),
    };
    let conversion = GhostscriptEngine::detect(ghostscript)?.convert(&request)?;
    if let Err(error) = restore_page_boxes(&input, &conversion.output_path) {
        return Err(rejected_output_error(
            error.to_string(),
            &conversion.output_path,
        ));
    }
    let expected_icc = conversion
        .color_conversion
        .as_ref()
        .map(|color| color.destination_profile_sha256.as_str());
    let (output, validation) =
        match validate_conversion(&input, &conversion.output_path, &mode, expected_icc) {
            Ok(result) => result,
            Err(error) => {
                return Err(rejected_output_error(
                    error.to_string(),
                    &conversion.output_path,
                ));
            }
        };
    if !validation.passed {
        return Err(rejected_output_error(
            validation.errors.join("; "),
            &conversion.output_path,
        ));
    }
    Ok(FullConversionReport {
        input,
        conversion,
        output,
        validation,
    })
}

fn rejected_output_error(message: String, path: &Path) -> OpenPrintPdfError {
    let retained = retain_rejected_output(path);
    OpenPrintPdfError::ValidationFailed(format!(
        "{message}; rejected diagnostic PDF kept at {} (do not use it for printing)",
        retained.display()
    ))
}

fn retain_rejected_output(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    for index in 1..=999 {
        let suffix = if index == 1 {
            ".rejected.pdf".to_string()
        } else {
            format!(".rejected-{index}.pdf")
        };
        let candidate = parent.join(format!("{stem}{suffix}"));
        if candidate.exists() {
            continue;
        }
        if std::fs::rename(path, &candidate).is_ok() {
            return candidate;
        }
        break;
    }
    path.to_path_buf()
}

fn ensure_transparency_allowed(
    kind: ConversionKind,
    transparent_pages: &[u32],
    allowed: bool,
) -> Result<()> {
    if kind == ConversionKind::PdfX1a && !transparent_pages.is_empty() && !allowed {
        return Err(OpenPrintPdfError::TransparencyConfirmationRequired(
            transparent_pages.to_vec(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_engine_is_a_report_not_an_inspection_error() {
        let availability = engine_availability(Some(Path::new("/definitely/missing/gs")));
        assert!(!availability.available);
        assert!(availability.info.is_none());
        assert!(availability.error.is_some());
    }

    #[test]
    fn retains_failed_conversion_as_rejected_pdf() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let output = directory.path().join("converted.pdf");
        std::fs::write(&output, b"%PDF-1.3\n").expect("diagnostic PDF");

        let retained = retain_rejected_output(&output);

        assert_eq!(retained, directory.path().join("converted.rejected.pdf"));
        assert!(!output.exists());
        assert!(retained.exists());
    }

    #[test]
    fn pdfx_conversion_requires_explicit_transparency_confirmation() {
        let error = ensure_transparency_allowed(ConversionKind::PdfX1a, &[1, 3], false)
            .expect_err("confirmation should be required");
        assert!(matches!(
            error,
            OpenPrintPdfError::TransparencyConfirmationRequired(pages) if pages == vec![1, 3]
        ));
        assert!(ensure_transparency_allowed(ConversionKind::PdfX1a, &[1], true).is_ok());
        assert!(ensure_transparency_allowed(ConversionKind::Outline, &[1], false).is_ok());
    }
}
