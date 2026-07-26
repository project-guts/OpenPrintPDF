use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use crate::{OpenPrintPdfError, PdfInspectionResult, Result};

pub const MAX_INPUT_PDF_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ICC_PROFILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_OUTPUT_PDF_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_GHOSTSCRIPT_LOG_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_PDF_PAGES: usize = 2_000;
pub const MAX_PDF_OBJECTS: usize = 1_000_000;
pub const MAX_RASTER_PIXELS_PER_PAGE: f64 = 300_000_000.0;
pub const MAX_TOTAL_RASTER_PIXELS: f64 = 1_000_000_000.0;

pub fn validate_pdf_input(path: &Path) -> Result<()> {
    if !has_extension(path, "pdf") {
        return Err(OpenPrintPdfError::UnsafeInput(format!(
            "input must have a .pdf extension: {}",
            path.display()
        )));
    }
    let metadata = fs::metadata(path).map_err(|source| OpenPrintPdfError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(OpenPrintPdfError::UnsafeInput(format!(
            "input is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() == 0 || metadata.len() > MAX_INPUT_PDF_BYTES {
        return Err(OpenPrintPdfError::ResourceLimitExceeded(format!(
            "PDF size must be between 1 byte and {} MiB (actual: {} bytes)",
            MAX_INPUT_PDF_BYTES / 1024 / 1024,
            metadata.len()
        )));
    }
    let mut file = File::open(path).map_err(|source| OpenPrintPdfError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut header = [0_u8; 1024];
    let count = file
        .read(&mut header)
        .map_err(|source| OpenPrintPdfError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if !header[..count].windows(5).any(|bytes| bytes == b"%PDF-") {
        return Err(OpenPrintPdfError::UnsafeInput(
            "PDF signature was not found in the first 1024 bytes".into(),
        ));
    }
    Ok(())
}

pub fn validate_document_complexity(page_count: usize, object_count: usize) -> Result<()> {
    if page_count > MAX_PDF_PAGES {
        return Err(OpenPrintPdfError::ResourceLimitExceeded(format!(
            "PDF has {page_count} pages; maximum is {MAX_PDF_PAGES}"
        )));
    }
    if object_count > MAX_PDF_OBJECTS {
        return Err(OpenPrintPdfError::ResourceLimitExceeded(format!(
            "PDF has {object_count} objects; maximum is {MAX_PDF_OBJECTS}"
        )));
    }
    Ok(())
}

pub fn validate_new_pdf_output(path: &Path) -> Result<()> {
    if !has_extension(path, "pdf") || path.exists() {
        return Err(OpenPrintPdfError::UnsafeOutput(path.to_path_buf()));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if parent.exists() && !parent.is_dir() {
        return Err(OpenPrintPdfError::UnsafeOutput(path.to_path_buf()));
    }
    Ok(())
}

pub fn validate_icc_file(path: &Path) -> Result<()> {
    if !(has_extension(path, "icc") || has_extension(path, "icm")) {
        return Err(OpenPrintPdfError::UnsafeInput(format!(
            "color profile must have an .icc or .icm extension: {}",
            path.display()
        )));
    }
    let metadata = fs::metadata(path).map_err(|source| OpenPrintPdfError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_ICC_PROFILE_BYTES {
        return Err(OpenPrintPdfError::ResourceLimitExceeded(format!(
            "ICC profile must be a regular file no larger than {} MiB",
            MAX_ICC_PROFILE_BYTES / 1024 / 1024
        )));
    }
    Ok(())
}

pub fn validate_transparency_raster_budget(
    inspection: &PdfInspectionResult,
    transparent_pages: &[u32],
    dpi: u16,
) -> Result<()> {
    let mut total = 0.0;
    for page_number in transparent_pages {
        let page = inspection
            .pages
            .iter()
            .find(|page| page.page_number == *page_number)
            .ok_or_else(|| OpenPrintPdfError::InvalidPdf("page geometry is missing".into()))?;
        let width_inches = page.media_box.width().abs() / 72.0;
        let height_inches = page.media_box.height().abs() / 72.0;
        let pixels = width_inches * f64::from(dpi) * height_inches * f64::from(dpi);
        if !pixels.is_finite() || pixels <= 0.0 || pixels > MAX_RASTER_PIXELS_PER_PAGE {
            return Err(OpenPrintPdfError::ResourceLimitExceeded(format!(
                "page {page_number} would rasterize to approximately {pixels:.0} pixels; per-page maximum is {MAX_RASTER_PIXELS_PER_PAGE:.0}"
            )));
        }
        total += pixels;
    }
    if total > MAX_TOTAL_RASTER_PIXELS {
        return Err(OpenPrintPdfError::ResourceLimitExceeded(format!(
            "transparent pages would rasterize to approximately {total:.0} pixels in total; maximum is {MAX_TOTAL_RASTER_PIXELS:.0}"
        )));
    }
    Ok(())
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rejects_non_pdf_extension_and_signature() {
        let dir = tempdir().unwrap();
        let wrong_extension = dir.path().join("input.txt");
        fs::write(&wrong_extension, b"%PDF-1.7").unwrap();
        assert!(validate_pdf_input(&wrong_extension).is_err());

        let wrong_signature = dir.path().join("input.pdf");
        fs::write(&wrong_signature, b"not a PDF").unwrap();
        assert!(validate_pdf_input(&wrong_signature).is_err());
    }

    #[test]
    fn accepts_signature_after_leading_bytes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("input.PDF");
        fs::write(&path, b"leading bytes\n%PDF-1.7").unwrap();
        assert!(validate_pdf_input(&path).is_ok());
    }

    #[test]
    fn refuses_to_overwrite_output() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("output.pdf");
        fs::write(&path, b"existing").unwrap();
        assert!(validate_new_pdf_output(&path).is_err());
    }

    #[test]
    fn rejects_pdf_over_size_limit_without_allocating_it() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("huge.pdf");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_INPUT_PDF_BYTES + 1).unwrap();
        assert!(matches!(
            validate_pdf_input(&path),
            Err(OpenPrintPdfError::ResourceLimitExceeded(_))
        ));
    }

    #[test]
    fn enforces_document_complexity_limits() {
        assert!(validate_document_complexity(MAX_PDF_PAGES, MAX_PDF_OBJECTS).is_ok());
        assert!(validate_document_complexity(MAX_PDF_PAGES + 1, 1).is_err());
        assert!(validate_document_complexity(1, MAX_PDF_OBJECTS + 1).is_err());
    }
}
