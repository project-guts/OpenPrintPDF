use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use open_print_pdf_core::{
    ColorConversionReport, ConversionMode, ConversionRequest, ConversionResult, EngineInfo,
    IccProfileSource, OpenPrintPdfError, PdfInspectionResult, PrintConversionEngine, Result,
    attach_pdfx1a_identification, ensure_distinct_paths, extract_output_intent, inspect_pdf,
    parse_icc_profile, transparency_pages,
};
use sha2::{Digest, Sha256};
use tempfile::{Builder, tempdir, tempfile};
use wait_timeout::ChildExt;

mod process_sandbox;

#[derive(Debug, Clone)]
pub struct GhostscriptEngine {
    executable: PathBuf,
    timeout: Duration,
}

impl GhostscriptEngine {
    pub fn detect(configured_path: Option<&Path>) -> Result<Self> {
        let executable = match configured_path {
            Some(path) if path.is_file() => normalize_windows_verbatim_path(path),
            Some(path) => {
                return Err(OpenPrintPdfError::EngineUnavailable(format!(
                    "configured Ghostscript executable does not exist: {}",
                    path.display()
                )));
            }
            None => find_ghostscript().ok_or_else(|| {
                OpenPrintPdfError::EngineUnavailable(
                    "Ghostscript executable was not found in PATH or standard locations".into(),
                )
            })?,
        };
        Ok(Self {
            executable,
            timeout: Duration::from_secs(300),
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn version(&self) -> Result<String> {
        let mut stdout =
            tempfile().map_err(|error| OpenPrintPdfError::EngineUnavailable(error.to_string()))?;
        let mut stderr =
            tempfile().map_err(|error| OpenPrintPdfError::EngineUnavailable(error.to_string()))?;
        let mut child = Command::new(&self.executable)
            .arg("--version")
            .env_remove("GS_OPTIONS")
            .env_remove("GS_DEVICE")
            .env_remove("GS_FONTPATH")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout.try_clone().map_err(|error| {
                OpenPrintPdfError::EngineUnavailable(error.to_string())
            })?))
            .stderr(Stdio::from(stderr.try_clone().map_err(|error| {
                OpenPrintPdfError::EngineUnavailable(error.to_string())
            })?))
            .spawn()
            .map_err(|error| OpenPrintPdfError::EngineUnavailable(error.to_string()))?;
        let _sandbox = match process_sandbox::attach(&child) {
            Ok(sandbox) => sandbox,
            Err(error) => {
                terminate(&mut child);
                return Err(OpenPrintPdfError::EngineUnavailable(format!(
                    "Ghostscript process isolation failed: {error}"
                )));
            }
        };
        let status = child
            .wait_timeout(Duration::from_secs(10))
            .map_err(|error| OpenPrintPdfError::EngineUnavailable(error.to_string()))?;
        let Some(status) = status else {
            terminate(&mut child);
            return Err(OpenPrintPdfError::EngineUnavailable(
                "Ghostscript version check timed out".into(),
            ));
        };
        let stdout = read_log(&mut stdout)
            .map_err(|error| OpenPrintPdfError::EngineUnavailable(error.to_string()))?;
        let stderr = read_log(&mut stderr)
            .map_err(|error| OpenPrintPdfError::EngineUnavailable(error.to_string()))?;
        if !status.success() {
            return Err(OpenPrintPdfError::EngineUnavailable(
                stderr.trim().to_string(),
            ));
        }
        Ok(stdout.trim().to_string())
    }

    fn run(
        &self,
        current_dir: &Path,
        arguments: &[OsString],
        output_path: &Path,
    ) -> Result<(u64, Vec<String>)> {
        let start = Instant::now();
        // File-backed logs avoid a pipe-buffer deadlock while the parent waits with a timeout.
        let mut stdout_log =
            tempfile().map_err(|error| OpenPrintPdfError::ConversionFailed(error.to_string()))?;
        let mut stderr_log =
            tempfile().map_err(|error| OpenPrintPdfError::ConversionFailed(error.to_string()))?;
        let mut command = Command::new(&self.executable);
        command
            .args(arguments)
            .current_dir(current_dir)
            .env_remove("GS_OPTIONS")
            .env_remove("GS_DEVICE")
            .env_remove("GS_FONTPATH")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_log.try_clone().map_err(|error| {
                OpenPrintPdfError::ConversionFailed(error.to_string())
            })?))
            .stderr(Stdio::from(stderr_log.try_clone().map_err(|error| {
                OpenPrintPdfError::ConversionFailed(error.to_string())
            })?));
        if let Some(library_path) = portable_library_path(&self.executable) {
            command.env("GS_LIB", library_path);
        }
        let mut child = command
            .spawn()
            .map_err(|error| OpenPrintPdfError::ConversionFailed(error.to_string()))?;
        let _sandbox = match process_sandbox::attach(&child) {
            Ok(sandbox) => sandbox,
            Err(error) => {
                terminate(&mut child);
                return Err(error);
            }
        };
        let status = loop {
            if start.elapsed() >= self.timeout {
                terminate(&mut child);
                return Err(OpenPrintPdfError::ResourceLimitExceeded(format!(
                    "Ghostscript timed out after {} seconds",
                    self.timeout.as_secs()
                )));
            }
            for (label, file) in [("stdout", &stdout_log), ("stderr", &stderr_log)] {
                if file.metadata().is_ok_and(|metadata| {
                    metadata.len() > open_print_pdf_core::MAX_GHOSTSCRIPT_LOG_BYTES
                }) {
                    terminate(&mut child);
                    return Err(OpenPrintPdfError::ResourceLimitExceeded(format!(
                        "Ghostscript {label} exceeded {} MiB",
                        open_print_pdf_core::MAX_GHOSTSCRIPT_LOG_BYTES / 1024 / 1024
                    )));
                }
            }
            if output_path
                .metadata()
                .is_ok_and(|metadata| metadata.len() > open_print_pdf_core::MAX_OUTPUT_PDF_BYTES)
            {
                terminate(&mut child);
                return Err(OpenPrintPdfError::ResourceLimitExceeded(format!(
                    "generated PDF exceeded {} MiB",
                    open_print_pdf_core::MAX_OUTPUT_PDF_BYTES / 1024 / 1024
                )));
            }
            if let Some(status) = child
                .wait_timeout(Duration::from_millis(250))
                .map_err(|error| OpenPrintPdfError::ConversionFailed(error.to_string()))?
            {
                break status;
            }
        };
        let stdout = read_log(&mut stdout_log)?;
        let stderr = read_log(&mut stderr_log)?;
        let mut lines = Vec::new();
        lines.extend(
            stdout
                .lines()
                .filter(|line| is_warning(line))
                .map(str::to_string),
        );
        lines.extend(
            stderr
                .lines()
                .filter(|line| is_warning(line))
                .map(str::to_string),
        );
        if !status.success() {
            return Err(OpenPrintPdfError::ConversionFailed(format!(
                "Ghostscript exited with {}\n{}\n{}",
                status,
                stdout.trim(),
                stderr.trim()
            )));
        }
        Ok((start.elapsed().as_millis() as u64, lines))
    }
}

#[cfg(windows)]
fn normalize_windows_verbatim_path(path: &Path) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    const VERBATIM_PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const UNC_PREFIX: &[u16] = &[b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16];

    let encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let Some(remainder) = encoded.strip_prefix(VERBATIM_PREFIX) else {
        return path.to_path_buf();
    };
    let normalized = if let Some(network_path) = remainder.strip_prefix(UNC_PREFIX) {
        let mut value = vec![b'\\' as u16, b'\\' as u16];
        value.extend_from_slice(network_path);
        value
    } else {
        remainder.to_vec()
    };
    PathBuf::from(OsString::from_wide(&normalized))
}

#[cfg(not(windows))]
fn normalize_windows_verbatim_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn terminate(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn portable_library_path(executable: &Path) -> Option<OsString> {
    let root = executable.parent()?.parent()?;
    let resource_init = root.join("Resource").join("Init");
    let library = root.join("lib");
    if !resource_init.is_dir() || !library.is_dir() {
        return None;
    }
    let candidates = [
        resource_init,
        root.join("Resource").join("Font"),
        root.join("Resource"),
        library,
        root.join("fonts"),
    ];
    env::join_paths(candidates.into_iter().filter(|path| path.is_dir())).ok()
}

fn read_log(file: &mut fs::File) -> Result<String> {
    if file
        .metadata()
        .is_ok_and(|metadata| metadata.len() > open_print_pdf_core::MAX_GHOSTSCRIPT_LOG_BYTES)
    {
        return Err(OpenPrintPdfError::ResourceLimitExceeded(
            "Ghostscript log exceeded the configured limit".into(),
        ));
    }
    file.seek(SeekFrom::Start(0))
        .and_then(|_| {
            let mut value = String::new();
            file.read_to_string(&mut value).map(|_| value)
        })
        .map_err(|error| OpenPrintPdfError::ConversionFailed(error.to_string()))
}

impl PrintConversionEngine for GhostscriptEngine {
    fn engine_info(&self) -> Result<EngineInfo> {
        Ok(EngineInfo {
            name: "Ghostscript".into(),
            version: self.version()?,
            executable: self.executable.clone(),
        })
    }

    fn convert(&self, request: &ConversionRequest) -> Result<ConversionResult> {
        ensure_distinct_paths(&request.input_path, &request.output_path)?;
        open_print_pdf_core::validate_new_pdf_output(&request.output_path)?;
        let input = inspect_pdf(&request.input_path)?;
        let flattened_pages = match request.mode {
            ConversionMode::PdfX1a { .. } => transparency_pages(&input),
            ConversionMode::OutlineOnly => Vec::new(),
        };
        let output_path = absolute_path(&request.output_path)?;
        let input_path = request.input_path.canonicalize().map_err(|error| {
            open_print_pdf_core::OpenPrintPdfError::Io {
                path: request.input_path.clone(),
                source: error,
            }
        })?;
        let work = tempdir().map_err(|error| open_print_pdf_core::OpenPrintPdfError::Io {
            path: env::temp_dir(),
            source: error,
        })?;
        // Ghostscript's Windows command-line file handling is not consistently Unicode-safe.
        // Keep every path visible to Ghostscript ASCII-only and relative to its working
        // directory. Rust performs the copies to and from user-selected paths.
        let staged_input = work.path().join("input.pdf");
        fs::copy(&input_path, &staged_input).map_err(|source| OpenPrintPdfError::Io {
            path: input_path.clone(),
            source,
        })?;
        let generated_output = work.path().join("output.pdf");
        let relative_input = Path::new("input.pdf");
        let relative_output = Path::new("output.pdf");
        let mut color_conversion = None;
        let mut duration_ms = 0;
        let mut engine_warnings = Vec::new();
        match &request.mode {
            ConversionMode::OutlineOnly => {
                let mut arguments = common_arguments(relative_output);
                arguments.push("-dNoOutputFonts".into());
                arguments.push("-dCompatibilityLevel=1.7".into());
                arguments.push(relative_input.as_os_str().to_owned());
                let (duration, warnings) = self.run(work.path(), &arguments, &generated_output)?;
                duration_ms = duration;
                engine_warnings = warnings;
            }
            ConversionMode::PdfX1a {
                outline_fonts,
                rendering_intent,
                black_point_compensation,
                transparency_resolution_dpi,
            } => {
                let (profile_bytes, description, output_condition_identifier) =
                    destination_profile(request)?;
                let profile = parse_icc_profile(&profile_bytes);
                if profile.color_space.as_deref() != Some("CMYK") {
                    return Err(OpenPrintPdfError::MissingCmykOutputIntent);
                }
                let relative_profile = Path::new("destination.icc");
                let profile_path = work.path().join(relative_profile);
                fs::write(&profile_path, &profile_bytes).map_err(|error| {
                    open_print_pdf_core::OpenPrintPdfError::Io {
                        path: profile_path.clone(),
                        source: error,
                    }
                })?;
                let relative_definition = Path::new("open_print_pdf_pdfx_definition.ps");
                let definition_path = work.path().join(relative_definition);
                let relative_metadata = Path::new("open_print_pdf_pdfx_metadata.xmp");
                let metadata_path = work.path().join(relative_metadata);
                fs::write(&metadata_path, pdfx_metadata()).map_err(|source| {
                    open_print_pdf_core::OpenPrintPdfError::Io {
                        path: metadata_path.clone(),
                        source,
                    }
                })?;
                fs::write(
                    &definition_path,
                    pdfx_definition(
                        description.as_deref().unwrap_or("Custom CMYK"),
                        output_condition_identifier
                            .as_deref()
                            .unwrap_or("Custom CMYK"),
                        relative_profile,
                        relative_metadata,
                    ),
                )
                .map_err(|error| open_print_pdf_core::OpenPrintPdfError::Io {
                    path: definition_path.clone(),
                    source: error,
                })?;
                if *outline_fonts {
                    // Keep color conversion and font outlining in separate pdfwrite
                    // invocations. The outlined PDF must not be interpreted by
                    // Ghostscript again: Windows 11 field tests showed that a subsequent
                    // successful pdfwrite pass could silently drop later page content.
                    let relative_cmyk = Path::new("cmyk.pdf");
                    let cmyk_path = work.path().join(relative_cmyk);
                    let mut color_arguments = common_arguments(relative_cmyk);
                    color_arguments.insert(0, "--permit-file-read=destination.icc".into());
                    color_arguments.extend([
                        "-dCompatibilityLevel=1.3".into(),
                        "-sColorConversionStrategy=CMYK".into(),
                        "-sOutputICCProfile=destination.icc".into(),
                        "-sDefaultRGBProfile=srgb.icc".into(),
                        "-dOverrideICC=false".into(),
                        format!("-dRenderIntent={}", rendering_intent.ghostscript_value()).into(),
                        format!("-dBlackPtComp={}", u8::from(*black_point_compensation)).into(),
                        format!("-dImageIntent={}", rendering_intent.ghostscript_value()).into(),
                        format!("-dImageBlackPt={}", u8::from(*black_point_compensation)).into(),
                        "-dVectorIntent=1".into(),
                        "-dVectorBlackPt=1".into(),
                        "-dTextIntent=1".into(),
                        "-dTextBlackPt=1".into(),
                        format!("-r{transparency_resolution_dpi}").into(),
                        relative_input.as_os_str().to_owned(),
                    ]);
                    let (duration, warnings) =
                        self.run(work.path(), &color_arguments, &cmyk_path)?;
                    duration_ms = duration_ms.saturating_add(duration);
                    engine_warnings.extend(warnings);
                    let cmyk_inspection = inspect_pdf(&cmyk_path)?;
                    if image_content_dropped(&input, &cmyk_inspection) {
                        let diagnostics =
                            retain_stage_diagnostics(&output_path, &[("stage1-cmyk", &cmyk_path)])?;
                        return Err(stage_image_loss_error(
                            1,
                            "DeviceCMYK conversion",
                            &input,
                            &cmyk_inspection,
                            &diagnostics,
                            &self.executable,
                        ));
                    }

                    let relative_outlined = Path::new("outlined.pdf");
                    let outlined_path = work.path().join(relative_outlined);
                    let mut outline_arguments = common_arguments(relative_outlined);
                    outline_arguments.push("-dNoOutputFonts".into());
                    outline_arguments.push("-dCompatibilityLevel=1.3".into());
                    outline_arguments.push(relative_cmyk.as_os_str().to_owned());
                    let (duration, warnings) =
                        self.run(work.path(), &outline_arguments, &outlined_path)?;
                    duration_ms = duration_ms.saturating_add(duration);
                    engine_warnings.extend(warnings);
                    let outlined_inspection = inspect_pdf(&outlined_path)?;
                    if image_content_dropped(&cmyk_inspection, &outlined_inspection) {
                        let diagnostics = retain_stage_diagnostics(
                            &output_path,
                            &[
                                ("stage1-cmyk", &cmyk_path),
                                ("stage2-outlined", &outlined_path),
                            ],
                        )?;
                        return Err(stage_image_loss_error(
                            2,
                            "font outlining",
                            &cmyk_inspection,
                            &outlined_inspection,
                            &diagnostics,
                            &self.executable,
                        ));
                    }

                    let started = Instant::now();
                    attach_pdfx1a_identification(
                        &outlined_path,
                        &generated_output,
                        &profile_bytes,
                        description.as_deref().unwrap_or("Custom CMYK"),
                        output_condition_identifier
                            .as_deref()
                            .unwrap_or("Custom CMYK"),
                    )?;
                    duration_ms = duration_ms.saturating_add(
                        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    );
                } else {
                    // Without font outlining, color conversion and PDF/X declaration
                    // can safely remain a single pdfwrite invocation.
                    let mut finalize_arguments = common_arguments(relative_output);
                    finalize_arguments.insert(0, "--permit-file-read=destination.icc".into());
                    finalize_arguments.insert(
                        1,
                        "--permit-file-read=open_print_pdf_pdfx_metadata.xmp".into(),
                    );
                    finalize_arguments.extend([
                        "-dPDFX=1".into(),
                        "-dCompatibilityLevel=1.3".into(),
                        "-sColorConversionStrategy=CMYK".into(),
                        "-sOutputICCProfile=destination.icc".into(),
                        "-sDefaultRGBProfile=srgb.icc".into(),
                        "-dOverrideICC=false".into(),
                        format!("-dRenderIntent={}", rendering_intent.ghostscript_value()).into(),
                        format!("-dBlackPtComp={}", u8::from(*black_point_compensation)).into(),
                        format!("-dImageIntent={}", rendering_intent.ghostscript_value()).into(),
                        format!("-dImageBlackPt={}", u8::from(*black_point_compensation)).into(),
                        "-dVectorIntent=1".into(),
                        "-dVectorBlackPt=1".into(),
                        "-dTextIntent=1".into(),
                        "-dTextBlackPt=1".into(),
                        format!("-r{transparency_resolution_dpi}").into(),
                        relative_definition.as_os_str().to_owned(),
                        relative_input.as_os_str().to_owned(),
                    ]);
                    let (duration, warnings) =
                        self.run(work.path(), &finalize_arguments, &generated_output)?;
                    duration_ms = duration_ms.saturating_add(duration);
                    engine_warnings.extend(warnings);
                }
                color_conversion = Some(ColorConversionReport {
                    destination_profile_sha256: profile.sha256,
                    destination_profile_description: description,
                    rendering_intent: *rendering_intent,
                    black_point_compensation: *black_point_compensation,
                    assumed_srgb_images: input
                        .images
                        .iter()
                        .filter(|image| {
                            image.color_space == open_print_pdf_core::ImageColorSpace::DeviceRgb
                        })
                        .count(),
                });
            }
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|source| OpenPrintPdfError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let output_parent = output_path.parent().unwrap_or_else(|| Path::new("."));
        let staged_output = Builder::new()
            .prefix(".open-print-pdf-")
            .suffix(".pdf")
            .tempfile_in(output_parent)
            .map_err(|source| OpenPrintPdfError::Io {
                path: output_parent.to_path_buf(),
                source,
            })?
            .into_temp_path();
        fs::copy(&generated_output, &staged_output).map_err(|source| OpenPrintPdfError::Io {
            path: output_path.clone(),
            source,
        })?;
        staged_output
            .persist(&output_path)
            .map_err(|error| OpenPrintPdfError::Io {
                path: output_path.clone(),
                source: error.error,
            })?;
        if !flattened_pages.is_empty() {
            engine_warnings.push(format!(
                "transparent pages were flattened to CMYK at the configured resolution: {}",
                flattened_pages
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(ConversionResult {
            output_path,
            engine: self.engine_info()?,
            duration_ms,
            flattened_pages,
            color_conversion,
            engine_warnings,
        })
    }
}

fn common_arguments(output_path: &Path) -> Vec<OsString> {
    vec![
        "-dSAFER".into(),
        "-dBATCH".into(),
        "-dNOPAUSE".into(),
        "-dPDFSTOPONERROR".into(),
        "-sDEVICE=pdfwrite".into(),
        format!("-sOutputFile={}", output_path.to_string_lossy()).into(),
    ]
}

fn image_pixels(document: &PdfInspectionResult) -> u128 {
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

fn image_content_dropped(before: &PdfInspectionResult, after: &PdfInspectionResult) -> bool {
    let before_pixels = image_pixels(before);
    before_pixels > 0 && image_pixels(after).saturating_mul(2) < before_pixels
}

fn retain_stage_diagnostics(output_path: &Path, stages: &[(&str, &Path)]) -> Result<Vec<PathBuf>> {
    let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| OpenPrintPdfError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let stem = output_path
        .file_stem()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    for index in 1..=999 {
        let discriminator = if index == 1 {
            String::new()
        } else {
            format!("-{index}")
        };
        let destinations = stages
            .iter()
            .map(|(label, _)| parent.join(format!("{stem}.diagnostic-{label}{discriminator}.pdf")))
            .collect::<Vec<_>>();
        if destinations.iter().any(|path| path.exists()) {
            continue;
        }
        for ((_, source), destination) in stages.iter().zip(&destinations) {
            fs::copy(source, destination).map_err(|source| OpenPrintPdfError::Io {
                path: destination.clone(),
                source,
            })?;
        }
        return Ok(destinations);
    }
    Err(OpenPrintPdfError::ConversionFailed(
        "could not allocate a unique diagnostic PDF name".into(),
    ))
}

fn stage_image_loss_error(
    stage: u8,
    label: &str,
    before: &PdfInspectionResult,
    after: &PdfInspectionResult,
    diagnostics: &[PathBuf],
    executable: &Path,
) -> OpenPrintPdfError {
    OpenPrintPdfError::ValidationFailed(format!(
        "Ghostscript stage {stage} ({label}) dropped embedded image content \
         ({} images / {} pixels before, {} images / {} pixels after); \
         diagnostic PDF(s): {}; Ghostscript executable: {}",
        before.images.len(),
        image_pixels(before),
        after.images.len(),
        image_pixels(after),
        diagnostics
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        executable.display(),
    ))
}

fn destination_profile(
    request: &ConversionRequest,
) -> Result<(Vec<u8>, Option<String>, Option<String>)> {
    match &request.destination_icc {
        IccProfileSource::InputOutputIntent => {
            let intent = extract_output_intent(&request.input_path)?
                .filter(|intent| !intent.profile_bytes.is_empty())
                .ok_or(OpenPrintPdfError::MissingCmykOutputIntent)?;
            if intent.profile_bytes.len() as u64 > open_print_pdf_core::MAX_ICC_PROFILE_BYTES {
                return Err(OpenPrintPdfError::ResourceLimitExceeded(format!(
                    "embedded OutputIntent profile exceeded {} MiB",
                    open_print_pdf_core::MAX_ICC_PROFILE_BYTES / 1024 / 1024
                )));
            }
            let description = intent
                .info
                .output_condition
                .clone()
                .or_else(|| intent.info.info.clone())
                .or_else(|| intent.info.output_condition_identifier.clone());
            let identifier = registered_output_condition_identifier(
                description
                    .as_deref()
                    .or(intent.info.output_condition_identifier.as_deref()),
            )
            .map(str::to_owned)
            .or(intent.info.output_condition_identifier);
            Ok((intent.profile_bytes, description, identifier))
        }
        IccProfileSource::File(path) => {
            open_print_pdf_core::validate_icc_file(path)?;
            let bytes =
                fs::read(path).map_err(|error| open_print_pdf_core::OpenPrintPdfError::Io {
                    path: path.clone(),
                    source: error,
                })?;
            let description = path
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned());
            let identifier = registered_output_condition_identifier(description.as_deref())
                .map(str::to_owned)
                .or_else(|| description.clone());
            Ok((bytes, description, identifier))
        }
    }
}

fn registered_output_condition_identifier(description: Option<&str>) -> Option<&'static str> {
    let description = description?.to_ascii_lowercase();
    if description.contains("japan color 2001 coated") {
        Some("JC200103")
    } else if description.contains("japan color 2001 uncoated") {
        Some("JC200104")
    } else if description.contains("japan color 2002 newspaper") {
        Some("JCN2002")
    } else {
        None
    }
}

fn pdfx_definition(
    description: &str,
    output_condition_identifier: &str,
    profile_path: &Path,
    metadata_path: &Path,
) -> String {
    let description = escape_postscript_string(description);
    let output_condition_identifier = escape_postscript_string(output_condition_identifier);
    let profile_path = escape_postscript_string(&profile_path.to_string_lossy());
    let metadata_path = escape_postscript_string(&metadata_path.to_string_lossy());
    format!(
        r#"%!
[/GTS_PDFXVersion (PDF/X-1:2001)
 /GTS_PDFXConformance (PDF/X-1a:2001)
 /Title (Open Print PDF conversion)
 /Trapped /False
 /DOCINFO pdfmark
[/_objdef {{icc_pdfx}} /type /stream /OBJ pdfmark
[{{icc_pdfx}} << /N 4 >> /PUT pdfmark
[{{icc_pdfx}} ({profile_path}) (r) file /PUT pdfmark
[/_objdef {{output_intent_pdfx}} /type /dict /OBJ pdfmark
[{{output_intent_pdfx}} <<
 /Type /OutputIntent
 /S /GTS_PDFX
 /OutputCondition ({description})
 /Info ({description})
 /OutputConditionIdentifier ({output_condition_identifier})
 /RegistryName (http://www.color.org)
 /DestOutputProfile {{icc_pdfx}}
>> /PUT pdfmark
[/_objdef {{metadata_pdfx}} /type /stream /OBJ pdfmark
[{{metadata_pdfx}} << /Type /Metadata /Subtype /XML >> /PUT pdfmark
[{{metadata_pdfx}} ({metadata_path}) (r) file /PUT pdfmark
[{{Catalog}} <<
 /OutputIntents [{{output_intent_pdfx}}]
 /Metadata {{metadata_pdfx}}
>> /PUT pdfmark
"#
    )
}

fn pdfx_metadata() -> &'static str {
    r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
   xmlns:dc="http://purl.org/dc/elements/1.1/"
   xmlns:pdfx="http://ns.adobe.com/pdfx/1.3/">
   <dc:format>application/pdf</dc:format>
   <pdfx:GTS_PDFXVersion>PDF/X-1:2001</pdfx:GTS_PDFXVersion>
   <pdfx:GTS_PDFXConformance>PDF/X-1a:2001</pdfx:GTS_PDFXConformance>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#
}

fn escape_postscript_string(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '(' => "\\(".chars().collect::<Vec<_>>(),
            ')' => "\\)".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            character if character.is_ascii() && !character.is_control() => vec![character],
            _ => vec!['?'],
        })
        .collect()
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| open_print_pdf_core::OpenPrintPdfError::Io {
            path: path.to_path_buf(),
            source: error,
        })
}

fn find_ghostscript() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["gswin64c.exe", "gswin32c.exe", "gs.exe"]
    } else {
        &["gs"]
    };
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            for name in names {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    if cfg!(windows) {
        let mut installed = ["ProgramFiles", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(env::var_os)
            .flat_map(|root| {
                fs::read_dir(PathBuf::from(root).join("gs"))
                    .into_iter()
                    .flatten()
                    .filter_map(std::result::Result::ok)
                    .map(|entry| entry.path())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        installed.sort_by(|left, right| right.cmp(left));
        for root in installed {
            for name in ["gswin64c.exe", "gswin32c.exe"] {
                let candidate = root.join("bin").join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    [
        PathBuf::from("/opt/homebrew/bin/gs"),
        PathBuf::from("/usr/local/bin/gs"),
        PathBuf::from("/usr/bin/gs"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

fn is_warning(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("warning") || lower.contains("error")
}

pub fn profile_sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn normalizes_windows_verbatim_paths_for_ghostscript() {
        assert_eq!(
            normalize_windows_verbatim_path(Path::new(
                r"\\?\C:\Open Print PDF\ghostscript\bin\gswin64c.exe"
            )),
            PathBuf::from(r"C:\Open Print PDF\ghostscript\bin\gswin64c.exe")
        );
        assert_eq!(
            normalize_windows_verbatim_path(Path::new(
                r"\\?\UNC\server\share\ghostscript\bin\gswin64c.exe"
            )),
            PathBuf::from(r"\\server\share\ghostscript\bin\gswin64c.exe")
        );
    }

    #[test]
    fn escapes_postscript_strings() {
        assert_eq!(escape_postscript_string("a(b)\\c"), "a\\(b\\)\\\\c");
    }

    #[test]
    fn definition_contains_pdfx_metadata() {
        let definition = pdfx_definition(
            "Japan Color 2001 Coated",
            "JC200103",
            Path::new("/tmp/Japan Color.icc"),
            Path::new("/tmp/metadata.xmp"),
        );
        assert!(definition.contains("GTS_PDFXVersion (PDF/X-1:2001)"));
        assert!(definition.contains("GTS_PDFXConformance (PDF/X-1a:2001)"));
        assert!(definition.contains("PDF/X-1a:2001"));
        assert!(definition.contains("Japan Color 2001 Coated"));
        assert!(definition.contains("/OutputConditionIdentifier (JC200103)"));
        assert!(definition.contains("/tmp/Japan Color.icc"));
        assert!(definition.contains("/tmp/metadata.xmp"));
    }

    #[test]
    fn metadata_contains_pdfx_identification() {
        let metadata = pdfx_metadata();
        assert!(metadata.contains("<pdfx:GTS_PDFXVersion>PDF/X-1:2001"));
        assert!(metadata.contains("<pdfx:GTS_PDFXConformance>PDF/X-1a:2001"));
    }

    #[test]
    fn retains_unique_stage_diagnostics() {
        let directory = tempdir().expect("temporary directory");
        let stage1 = directory.path().join("cmyk.pdf");
        let stage2 = directory.path().join("outlined.pdf");
        fs::write(&stage1, b"stage one").expect("stage 1");
        fs::write(&stage2, b"stage two").expect("stage 2");
        let output = directory.path().join("converted.pdf");

        let first = retain_stage_diagnostics(
            &output,
            &[("stage1-cmyk", &stage1), ("stage2-outlined", &stage2)],
        )
        .expect("first diagnostics");
        let second = retain_stage_diagnostics(
            &output,
            &[("stage1-cmyk", &stage1), ("stage2-outlined", &stage2)],
        )
        .expect("second diagnostics");

        assert!(first.iter().all(|path| path.exists()));
        assert!(second.iter().all(|path| path.exists()));
        assert_ne!(first, second);
        assert_eq!(fs::read(&first[0]).expect("stage 1 copy"), b"stage one");
        assert_eq!(fs::read(&first[1]).expect("stage 2 copy"), b"stage two");
    }

    #[test]
    fn maps_registered_japan_color_output_conditions() {
        assert_eq!(
            registered_output_condition_identifier(Some("Japan Color 2001 Coated")),
            Some("JC200103")
        );
        assert_eq!(
            registered_output_condition_identifier(Some("Japan Color 2001 Uncoated")),
            Some("JC200104")
        );
        assert_eq!(
            registered_output_condition_identifier(Some("Japan Color 2002 Newspaper")),
            Some("JCN2002")
        );
    }

    #[test]
    fn ignores_non_portable_install_layouts() {
        assert!(portable_library_path(Path::new("/usr/bin/gs")).is_none());
    }
}
