use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use open_print_pdf_application::{
    ConversionKind, ConversionOptions, FullConversionReport, convert,
};
use open_print_pdf_conversion_ghostscript::GhostscriptEngine;
use open_print_pdf_core::{
    PdfInspectionResult, PreflightFinding, PrintConversionEngine, RenderingIntent, inspect_pdf,
    run_preflight,
};

#[derive(Debug, Parser)]
#[command(
    name = "pdfx1a-convert",
    version,
    about = "Inspect and convert print PDFs"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Inspect PDF structure without Ghostscript.
    Inspect {
        input: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Run native preflight checks without Ghostscript.
    Preflight {
        input: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Convert every font to vector paths.
    Outline {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        ghostscript: Option<PathBuf>,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Convert to PDF/X-1a with DeviceCMYK images and optional font outlining.
    ConvertPdfx1a {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Use this destination ICC instead of the input PDF/X OutputIntent.
        #[arg(long)]
        icc: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = IntentArgument::Relative)]
        intent: IntentArgument,
        #[arg(long)]
        no_black_point_compensation: bool,
        #[arg(long)]
        no_outline: bool,
        #[arg(long, default_value_t = 720)]
        transparency_dpi: u16,
        #[arg(long)]
        ghostscript: Option<PathBuf>,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Show the detected Ghostscript engine.
    EngineInfo {
        #[arg(long)]
        ghostscript: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum IntentArgument {
    Perceptual,
    Relative,
    Saturation,
    Absolute,
}

impl From<IntentArgument> for RenderingIntent {
    fn from(value: IntentArgument) -> Self {
        match value {
            IntentArgument::Perceptual => Self::Perceptual,
            IntentArgument::Relative => Self::RelativeColorimetric,
            IntentArgument::Saturation => Self::Saturation,
            IntentArgument::Absolute => Self::AbsoluteColorimetric,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Inspect { input, json } => {
            let inspection = inspect_pdf(&input)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&inspection)?);
            } else {
                print_inspection(&inspection);
            }
        }
        Commands::Preflight { input, json } => {
            let inspection = inspect_pdf(&input)?;
            let findings = run_preflight(&inspection);
            if json {
                println!("{}", serde_json::to_string_pretty(&findings)?);
            } else {
                print_preflight(&findings);
            }
        }
        Commands::Outline {
            input,
            output,
            ghostscript,
            report,
        } => {
            let output = output.unwrap_or_else(|| default_output(&input, "outlined"));
            run_conversion(
                ghostscript.as_deref(),
                ConversionOptions {
                    input_path: input,
                    output_path: output,
                    kind: ConversionKind::Outline,
                    destination_icc: None,
                    rendering_intent: RenderingIntent::RelativeColorimetric,
                    black_point_compensation: true,
                    outline_fonts: true,
                    transparency_resolution_dpi: 720,
                    allow_transparency: true,
                },
                report.as_deref(),
            )?;
        }
        Commands::ConvertPdfx1a {
            input,
            output,
            icc,
            intent,
            no_black_point_compensation,
            no_outline,
            transparency_dpi,
            ghostscript,
            report,
        } => {
            let output = output.unwrap_or_else(|| default_output(&input, "pdfx1a"));
            run_conversion(
                ghostscript.as_deref(),
                ConversionOptions {
                    input_path: input,
                    output_path: output,
                    kind: ConversionKind::PdfX1a,
                    destination_icc: icc,
                    rendering_intent: intent.into(),
                    black_point_compensation: !no_black_point_compensation,
                    outline_fonts: !no_outline,
                    transparency_resolution_dpi: transparency_dpi,
                    allow_transparency: true,
                },
                report.as_deref(),
            )?;
        }
        Commands::EngineInfo { ghostscript } => {
            let engine = GhostscriptEngine::detect(ghostscript.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&engine.engine_info()?)?);
        }
    }
    Ok(())
}

fn run_conversion(
    ghostscript: Option<&Path>,
    options: ConversionOptions,
    report_path: Option<&Path>,
) -> Result<()> {
    let input = inspect_pdf(&options.input_path)?;
    if !input.transparency.is_empty() {
        let mut pages = input
            .transparency
            .iter()
            .map(|finding| finding.page_number)
            .collect::<Vec<_>>();
        pages.sort_unstable();
        pages.dedup();
        eprintln!(
            "WARNING: This PDF contains transparency on page(s): {}",
            join_numbers(&pages)
        );
        eprintln!(
            "Those pages will be flattened to an 8-bit DeviceCMYK image at the configured resolution."
        );
        eprintln!(
            "Text and line art on those pages will also become raster data; conversion will continue."
        );
    }
    let report: FullConversionReport = convert(&options, ghostscript)?;
    if let Some(path) = report_path {
        let json = serde_json::to_vec_pretty(&report)?;
        fs::write(path, json)
            .with_context(|| format!("failed to write report {}", path.display()))?;
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn print_inspection(document: &PdfInspectionResult) {
    println!("File: {}", document.path.display());
    println!("PDF version: {}", document.pdf_version);
    println!(
        "PDF/X claim: {}",
        document.pdfx_claim.as_deref().unwrap_or("none")
    );
    println!("Pages: {}", document.page_count);
    println!("Fonts: {}", document.fonts.len());
    println!("Images: {}", document.images.len());
    println!("RGB images: {}", document.rgb_image_count());
    println!("CMYK images: {}", document.cmyk_image_count());
    println!("Transparency findings: {}", document.transparency.len());
    if let Some(intent) = &document.output_intent {
        println!(
            "OutputIntent: {}",
            intent
                .output_condition_identifier
                .as_deref()
                .or(intent.info.as_deref())
                .unwrap_or("unnamed")
        );
        if let Some(profile) = &intent.profile {
            println!(
                "OutputIntent ICC: {} {} bytes",
                profile.sha256, profile.byte_length
            );
        }
    } else {
        println!("OutputIntent: none");
    }
}

fn print_preflight(findings: &[PreflightFinding]) {
    if findings.is_empty() {
        println!("No preflight findings.");
        return;
    }
    for finding in findings {
        println!(
            "{:?}\t{:?}\t{}\t{}",
            finding.severity,
            finding.rule,
            finding
                .page_number
                .map(|page| format!("page {page}"))
                .unwrap_or_else(|| "document".into()),
            finding.message
        );
    }
}

fn default_output(input: &Path, suffix: &str) -> PathBuf {
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    let stem = input
        .file_stem()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    parent.join(format!("{stem}-{suffix}.pdf"))
}

fn join_numbers(numbers: &[u32]) -> String {
    numbers
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
