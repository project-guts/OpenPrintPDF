use std::collections::BTreeSet;
use std::path::Path;

use lopdf::{Document, Object};
use serde::{Deserialize, Serialize};

use crate::{
    ConversionMode, OpenPrintPdfError, PdfBox, PdfInspectionResult, Result, inspect_pdf, io_error,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversionValidationResult {
    pub passed: bool,
    pub readable: bool,
    pub page_count_matches: bool,
    pub page_boxes_match: bool,
    pub fonts_remaining: usize,
    pub type3_fonts_remaining: usize,
    pub rgb_images_remaining: usize,
    pub cmyk_images: usize,
    pub image_content_retained: bool,
    pub transparency_remaining: usize,
    pub pdf_version: String,
    pub pdfx_claim: Option<String>,
    pub output_intent_matches: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

pub fn validate_conversion(
    input: &PdfInspectionResult,
    output_path: &Path,
    mode: &ConversionMode,
    expected_icc_sha256: Option<&str>,
) -> Result<(PdfInspectionResult, ConversionValidationResult)> {
    let output = inspect_pdf(output_path)?;
    let page_count_matches = input.page_count == output.page_count;
    let page_boxes_match = page_geometry_matches(input, &output, 0.02);
    let type3_fonts_remaining = output
        .fonts
        .iter()
        .filter(|font| font.subtype.as_deref() == Some("Type3"))
        .count();
    let rgb_images_remaining = output.rgb_image_count();
    let cmyk_images = output.cmyk_image_count();
    let input_image_pixels = total_image_pixels(input);
    let output_image_pixels = total_image_pixels(&output);
    let image_content_retained = !input.transparency.is_empty()
        || input_image_pixels == 0
        || output_image_pixels.saturating_mul(2) >= input_image_pixels;
    let output_icc = output
        .output_intent
        .as_ref()
        .and_then(|intent| intent.profile.as_ref())
        .map(|profile| profile.sha256.as_str());
    let output_intent_matches =
        expected_icc_sha256.is_none_or(|expected| output_icc == Some(expected));
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if !page_count_matches {
        errors.push("page count changed".into());
    }
    if !page_boxes_match {
        errors.push("page boxes changed".into());
    }
    match mode {
        ConversionMode::OutlineOnly => {
            if !output.fonts.is_empty() {
                errors.push(format!("{} fonts remain", output.fonts.len()));
            }
        }
        ConversionMode::PdfX1a { outline_fonts, .. } => {
            if *outline_fonts && !output.fonts.is_empty() {
                errors.push(format!("{} fonts remain", output.fonts.len()));
            }
            if rgb_images_remaining != 0 {
                errors.push(format!("{rgb_images_remaining} RGB images remain"));
            }
            if !output.transparency.is_empty() {
                errors.push(format!(
                    "{} transparency features remain",
                    output.transparency.len()
                ));
            }
            if output.pdf_version != "1.3" {
                errors.push(format!("expected PDF 1.3, got {}", output.pdf_version));
            }
            if !output
                .pdfx_claim
                .as_deref()
                .is_some_and(|claim| claim.contains("PDF/X-1"))
            {
                errors.push("PDF/X-1a conformance claim is missing".into());
            }
            if !output_intent_matches {
                errors.push("OutputIntent ICC profile does not match the requested profile".into());
            }
        }
    }
    if input.transparency.is_empty() && cmyk_images == 0 && !input.images.is_empty() {
        warnings.push("no CMYK images were found after conversion".into());
    }
    if !image_content_retained {
        errors.push(format!(
            "embedded image content dropped unexpectedly ({output_image_pixels} of {input_image_pixels} pixels remain)"
        ));
    }
    let passed = errors.is_empty();
    let result = ConversionValidationResult {
        passed,
        readable: true,
        page_count_matches,
        page_boxes_match,
        fonts_remaining: output.fonts.len(),
        type3_fonts_remaining,
        rgb_images_remaining,
        cmyk_images,
        image_content_retained,
        transparency_remaining: output.transparency.len(),
        pdf_version: output.pdf_version.clone(),
        pdfx_claim: output.pdfx_claim.clone(),
        output_intent_matches,
        warnings,
        errors,
    };
    Ok((output, result))
}

fn total_image_pixels(document: &PdfInspectionResult) -> u128 {
    document
        .images
        .iter()
        .filter_map(|image| {
            let width = u128::try_from(image.width?).ok()?;
            let height = u128::try_from(image.height?).ok()?;
            Some(width.saturating_mul(height))
        })
        .sum()
}

pub fn restore_page_boxes(input: &PdfInspectionResult, output_path: &Path) -> Result<()> {
    let mut document =
        Document::load(output_path).map_err(|source| OpenPrintPdfError::PdfRead {
            path: output_path.to_path_buf(),
            source,
        })?;
    let pages = document.get_pages();
    if pages.len() != input.pages.len() {
        return Err(OpenPrintPdfError::ValidationFailed(
            "cannot restore page boxes because the page count changed".into(),
        ));
    }
    for ((_, page_id), original) in pages.into_iter().zip(&input.pages) {
        let media = original.media_box;
        let dictionary = document
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|error| OpenPrintPdfError::InvalidPdf(error.to_string()))?;
        set_box(dictionary, b"MediaBox", normalized_box(media, media));
        set_optional_box(dictionary, b"CropBox", original.crop_box, media);
        set_optional_box(dictionary, b"TrimBox", original.trim_box, media);
        set_optional_box(dictionary, b"BleedBox", original.bleed_box, media);
        set_optional_box(dictionary, b"ArtBox", original.art_box, media);
    }
    document.compress();
    document
        .save(output_path)
        .map_err(|source| io_error(output_path, source))?;
    Ok(())
}

fn set_optional_box(
    dictionary: &mut lopdf::Dictionary,
    key: &[u8],
    value: Option<PdfBox>,
    media: PdfBox,
) {
    if let Some(value) = value {
        set_box(dictionary, key, normalized_box(value, media));
    } else {
        dictionary.remove(key);
    }
}

fn normalized_box(value: PdfBox, media: PdfBox) -> PdfBox {
    value.normalized_to(media)
}

fn set_box(dictionary: &mut lopdf::Dictionary, key: &[u8], value: PdfBox) {
    dictionary.set(
        key,
        Object::Array(vec![
            Object::Real(value.left as f32),
            Object::Real(value.bottom as f32),
            Object::Real(value.right as f32),
            Object::Real(value.top as f32),
        ]),
    );
}

fn page_geometry_matches(
    input: &PdfInspectionResult,
    output: &PdfInspectionResult,
    epsilon: f64,
) -> bool {
    if input.pages.len() != output.pages.len() {
        return false;
    }
    input
        .pages
        .iter()
        .zip(&output.pages)
        .all(|(before, after)| {
            normalized(before.media_box, before.media_box)
                .zip(normalized(after.media_box, after.media_box))
                .is_some_and(|(a, b)| a.approximately_eq(b, epsilon))
                && optional_box_matches(
                    before.crop_box,
                    before.media_box,
                    after.crop_box,
                    after.media_box,
                    epsilon,
                )
                && optional_box_matches(
                    before.trim_box,
                    before.media_box,
                    after.trim_box,
                    after.media_box,
                    epsilon,
                )
                && optional_box_matches(
                    before.bleed_box,
                    before.media_box,
                    after.bleed_box,
                    after.media_box,
                    epsilon,
                )
                && optional_box_matches(
                    before.art_box,
                    before.media_box,
                    after.art_box,
                    after.media_box,
                    epsilon,
                )
        })
}

fn normalized(value: PdfBox, media: PdfBox) -> Option<PdfBox> {
    Some(value.normalized_to(media))
}

fn optional_box_matches(
    before: Option<PdfBox>,
    before_media: PdfBox,
    after: Option<PdfBox>,
    after_media: PdfBox,
    epsilon: f64,
) -> bool {
    match (before, after) {
        (None, None) => true,
        (Some(before), Some(after)) => before
            .normalized_to(before_media)
            .approximately_eq(after.normalized_to(after_media), epsilon),
        _ => false,
    }
}

pub fn transparency_pages(document: &PdfInspectionResult) -> Vec<u32> {
    document
        .transparency
        .iter()
        .map(|finding| finding.page_number)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_boxes_after_origin_normalization() {
        let media = PdfBox {
            left: 0.0,
            bottom: 0.285,
            right: 100.0,
            top: 200.285,
        };
        let trim = PdfBox {
            left: 10.0,
            bottom: 10.285,
            right: 90.0,
            top: 190.285,
        };
        let normalized = trim.normalized_to(media);
        assert!(normalized.approximately_eq(
            PdfBox {
                left: 10.0,
                bottom: 10.0,
                right: 90.0,
                top: 190.0
            },
            0.001
        ));
    }
}
