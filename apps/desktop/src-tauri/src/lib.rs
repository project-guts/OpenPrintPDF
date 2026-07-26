use std::{
    fs,
    path::{Path, PathBuf},
};

use open_print_pdf_application::{
    ConversionOptions, EngineAvailability, FullConversionReport, InspectionReport,
    convert as convert_document, engine_availability, inspect as inspect_document,
};
use open_print_pdf_core::ensure_distinct_paths;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tempfile::Builder;

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn bundled_ghostscript(app: &AppHandle) -> Option<PathBuf> {
    let root = app.path().resource_dir().ok()?.join("ghostscript");
    Some(if cfg!(windows) {
        root.join("bin").join("gswin64c.exe")
    } else {
        root.join("bin").join("gs")
    })
}

#[tauri::command]
fn engine_status(app: AppHandle) -> EngineAvailability {
    let configured = bundled_ghostscript(&app);
    engine_availability(configured.as_deref())
}

#[tauri::command]
fn inspect_pdf(app: AppHandle, path: String) -> Result<InspectionReport, String> {
    let configured = bundled_ghostscript(&app);
    inspect_document(PathBuf::from(path).as_path(), configured.as_deref())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn convert_pdf(
    app: AppHandle,
    mut options: ConversionOptions,
) -> Result<FullConversionReport, String> {
    let configured = bundled_ghostscript(&app);
    let requested_output = options.output_path.clone();
    ensure_distinct_paths(&options.input_path, &requested_output)
        .map_err(|error| error.to_string())?;

    let temporary_output = temporary_pdf_path(&requested_output)?;
    options.output_path = temporary_output.clone();
    let conversion = convert_document(&options, configured.as_deref());
    let mut report = match conversion {
        Ok(report) => report,
        Err(error) => {
            let _ = fs::remove_file(&temporary_output);
            return Err(error.to_string());
        }
    };

    replace_output(&temporary_output, &requested_output)?;
    report.conversion.output_path = requested_output.clone();
    report.output.path = requested_output;
    Ok(report)
}

fn temporary_pdf_path(destination: &Path) -> Result<PathBuf, String> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("出力先フォルダを作成できません: {error}"))?;
    let file = Builder::new()
        .prefix(".open-print-pdf-")
        .suffix(".pdf")
        .tempfile_in(parent)
        .map_err(|error| format!("一時出力ファイルを作成できません: {error}"))?;
    let path = file.path().to_path_buf();
    file.close()
        .map_err(|error| format!("一時出力ファイルを準備できません: {error}"))?;
    Ok(path)
}

fn replace_output(source: &Path, destination: &Path) -> Result<(), String> {
    if !destination.exists() {
        return fs::rename(source, destination)
            .map_err(|error| format!("変換結果を保存できません: {error}"));
    }
    if !destination.is_file() {
        return Err(format!(
            "出力先は通常のファイルではありません: {}",
            destination.display()
        ));
    }

    let backup = temporary_pdf_path(destination)?;
    fs::rename(destination, &backup)
        .map_err(|error| format!("既存ファイルを上書き用に退避できません: {error}"))?;
    if let Err(error) = fs::rename(source, destination) {
        let restore_error = fs::rename(&backup, destination).err();
        return Err(match restore_error {
            Some(restore_error) => format!(
                "変換結果を上書きできず、元のファイルの復元にも失敗しました: {error}; 復元エラー: {restore_error}; 退避先: {}",
                backup.display()
            ),
            None => format!("変換結果を上書きできません。元のファイルは保持されています: {error}"),
        });
    }
    fs::remove_file(&backup).map_err(|error| {
        format!("上書きは完了しましたが、退避ファイルを削除できません: {error}")
    })?;
    Ok(())
}

#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    let requested = PathBuf::from(path);
    let path = requested
        .canonicalize()
        .map_err(|_| format!("path does not exist: {}", requested.display()))?;
    if path.is_file()
        && !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
    {
        return Err("only PDF files and folders can be opened".into());
    }
    if !path.is_file() && !path.is_dir() {
        return Err("only PDF files and folders can be opened".into());
    }
    app.opener()
        .open_path(path.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            app_version,
            engine_status,
            inspect_pdf,
            convert_pdf,
            open_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running Open Print PDF");
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    #[test]
    fn application_version_comes_from_the_compiled_package() {
        assert_eq!(super::app_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn replaces_an_existing_output() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("output.pdf");
        let source = directory.path().join("converted.pdf");
        fs::write(&destination, b"old").unwrap();
        fs::write(&source, b"new").unwrap();

        super::replace_output(&source, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert!(!source.exists());
    }

    #[test]
    fn moves_output_when_destination_is_new() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("output.pdf");
        let source = directory.path().join("converted.pdf");
        fs::write(&source, b"new").unwrap();

        super::replace_output(&source, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert!(!source.exists());
    }
}
