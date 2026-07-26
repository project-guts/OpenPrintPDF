#![allow(clippy::collapsible_if)]

use std::collections::HashSet;
use std::path::Path;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use sha2::{Digest, Sha256};

use crate::{
    FontInfo, IccProfileInfo, ImageColorSpace, ImageInfo, OpenPrintPdfError, OutputIntentData,
    OutputIntentInfo, PageGeometry, PdfBox, PdfInspectionResult, Result, TransparencyFinding,
    TransparencyKind, validate_document_complexity, validate_pdf_input,
};

pub fn inspect_pdf(path: &Path) -> Result<PdfInspectionResult> {
    validate_pdf_input(path)?;
    let document = Document::load(path).map_err(|source| OpenPrintPdfError::PdfRead {
        path: path.to_path_buf(),
        source,
    })?;
    validate_document_complexity(document.get_pages().len(), document.objects.len())?;
    inspect_document(path, &document)
}

pub fn extract_output_intent(path: &Path) -> Result<Option<OutputIntentData>> {
    validate_pdf_input(path)?;
    let document = Document::load(path).map_err(|source| OpenPrintPdfError::PdfRead {
        path: path.to_path_buf(),
        source,
    })?;
    validate_document_complexity(document.get_pages().len(), document.objects.len())?;
    output_intent(&document)
}

fn inspect_document(path: &Path, document: &Document) -> Result<PdfInspectionResult> {
    let mut pages = Vec::new();
    let mut fonts = Vec::new();
    let mut images = Vec::new();
    let mut transparency = Vec::new();

    for (page_number, page_id) in document.get_pages() {
        let page = document
            .get_dictionary(page_id)
            .map_err(|error| OpenPrintPdfError::InvalidPdf(error.to_string()))?;
        let media_box = inherited_box(document, page_id, b"MediaBox")?.ok_or_else(|| {
            OpenPrintPdfError::InvalidPdf(format!("page {page_number} has no MediaBox"))
        })?;
        pages.push(PageGeometry {
            page_number,
            media_box,
            crop_box: inherited_box(document, page_id, b"CropBox")?,
            trim_box: inherited_box(document, page_id, b"TrimBox")?,
            bleed_box: inherited_box(document, page_id, b"BleedBox")?,
            art_box: inherited_box(document, page_id, b"ArtBox")?,
            rotation: inherited_integer(document, page_id, b"Rotate").unwrap_or(0) as i32,
        });

        if let Ok(group) = page.get(b"Group") {
            if dictionary_name(document, group, b"S").as_deref() == Some("Transparency") {
                transparency.push(TransparencyFinding {
                    page_number,
                    kind: TransparencyKind::TransparencyGroup,
                    resource_name: None,
                    detail: "page transparency group".into(),
                });
            }
        }

        if let Some(resources) = inherited_object(document, page_id, b"Resources") {
            let mut seen_xobjects = HashSet::new();
            inspect_resources(
                document,
                &resources,
                page_number,
                "",
                &mut seen_xobjects,
                &mut fonts,
                &mut images,
                &mut transparency,
            )?;
        }
    }

    let output_intent = output_intent(document)?.map(|intent| intent.info);
    let pdfx_claim = pdfx_claim(document);

    Ok(PdfInspectionResult {
        path: path.to_path_buf(),
        pdf_version: document.version.clone(),
        pdfx_claim,
        page_count: pages.len(),
        pages,
        fonts,
        images,
        transparency,
        output_intent,
    })
}

#[allow(clippy::too_many_arguments)]
fn inspect_resources(
    document: &Document,
    resources_object: &Object,
    page_number: u32,
    prefix: &str,
    seen_xobjects: &mut HashSet<ObjectId>,
    fonts: &mut Vec<FontInfo>,
    images: &mut Vec<ImageInfo>,
    transparency: &mut Vec<TransparencyFinding>,
) -> Result<()> {
    let resources = resolved(document, resources_object)
        .and_then(Object::as_dict)
        .map_err(|error| OpenPrintPdfError::InvalidPdf(error.to_string()))?;

    if let Ok(fonts_object) = resources.get(b"Font") {
        if let Ok(font_dictionary) = resolved(document, fonts_object).and_then(Object::as_dict) {
            for (name, font_object) in font_dictionary.iter() {
                if let Ok(font) = resolved(document, font_object).and_then(Object::as_dict) {
                    let base_name = dictionary_name(document, font_object, b"BaseFont");
                    let resource_name = format!("{prefix}{}", name_to_string(name));
                    fonts.push(FontInfo {
                        page_number,
                        resource_name,
                        subset: base_name.as_deref().is_some_and(is_subset_font_name),
                        base_name,
                        subtype: dictionary_name(document, font_object, b"Subtype"),
                        embedded: font_is_embedded(document, font),
                        has_to_unicode: font.has(b"ToUnicode"),
                    });
                }
            }
        }
    }

    if let Ok(states_object) = resources.get(b"ExtGState") {
        if let Ok(states) = resolved(document, states_object).and_then(Object::as_dict) {
            for (name, state_object) in states.iter() {
                if let Ok(state) = resolved(document, state_object).and_then(Object::as_dict) {
                    inspect_graphics_state(
                        document,
                        state,
                        page_number,
                        format!("{prefix}{}", name_to_string(name)),
                        transparency,
                    );
                }
            }
        }
    }

    if let Ok(xobjects_object) = resources.get(b"XObject") {
        if let Ok(xobjects) = resolved(document, xobjects_object).and_then(Object::as_dict) {
            for (name, xobject) in xobjects.iter() {
                let resource_name = format!("{prefix}{}", name_to_string(name));
                if let Object::Reference(id) = xobject {
                    if !seen_xobjects.insert(*id) {
                        continue;
                    }
                }
                let Ok(stream) = resolved(document, xobject).and_then(Object::as_stream) else {
                    continue;
                };
                match dictionary_name_from_dict(document, &stream.dict, b"Subtype").as_deref() {
                    Some("Image") => inspect_image(
                        document,
                        stream,
                        page_number,
                        resource_name,
                        images,
                        transparency,
                    ),
                    Some("Form") => {
                        if let Ok(group) = stream.dict.get(b"Group") {
                            if dictionary_name(document, group, b"S").as_deref()
                                == Some("Transparency")
                            {
                                transparency.push(TransparencyFinding {
                                    page_number,
                                    kind: TransparencyKind::TransparencyGroup,
                                    resource_name: Some(resource_name.clone()),
                                    detail: "form transparency group".into(),
                                });
                            }
                        }
                        if let Ok(nested) = stream.dict.get(b"Resources") {
                            inspect_resources(
                                document,
                                nested,
                                page_number,
                                &format!("{resource_name}/"),
                                seen_xobjects,
                                fonts,
                                images,
                                transparency,
                            )?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn inspect_image(
    document: &Document,
    stream: &Stream,
    page_number: u32,
    resource_name: String,
    images: &mut Vec<ImageInfo>,
    transparency: &mut Vec<TransparencyFinding>,
) {
    let (color_space, icc_sha256) = stream
        .dict
        .get(b"ColorSpace")
        .ok()
        .map(|object| image_color_space(document, object))
        .unwrap_or((ImageColorSpace::Unknown, None));
    let has_soft_mask = stream.dict.get(b"SMask").ok().is_some_and(
        |object| !matches!(resolved(document, object), Ok(Object::Name(name)) if name == b"None"),
    );
    if has_soft_mask {
        transparency.push(TransparencyFinding {
            page_number,
            kind: TransparencyKind::SoftMask,
            resource_name: Some(resource_name.clone()),
            detail: "image soft mask (/SMask)".into(),
        });
    }
    images.push(ImageInfo {
        page_number,
        resource_name,
        width: dictionary_integer(&stream.dict, b"Width"),
        height: dictionary_integer(&stream.dict, b"Height"),
        bits_per_component: dictionary_integer(&stream.dict, b"BitsPerComponent"),
        color_space,
        has_soft_mask,
        has_image_mask: stream.dict.has(b"ImageMask") || stream.dict.has(b"Mask"),
        icc_sha256,
    });
}

fn inspect_graphics_state(
    document: &Document,
    state: &Dictionary,
    page_number: u32,
    resource_name: String,
    findings: &mut Vec<TransparencyFinding>,
) {
    for key in [b"ca".as_slice(), b"CA".as_slice()] {
        if state
            .get(key)
            .ok()
            .and_then(number)
            .is_some_and(|value| value < 0.999_999)
        {
            findings.push(TransparencyFinding {
                page_number,
                kind: TransparencyKind::ConstantAlpha,
                resource_name: Some(resource_name.clone()),
                detail: format!("{} is less than 1", name_to_string(key)),
            });
        }
    }
    if let Ok(blend_mode) = state.get(b"BM") {
        let non_normal = match resolved(document, blend_mode) {
            Ok(Object::Name(name)) => name != b"Normal",
            Ok(Object::Array(modes)) => modes.iter().any(|mode| {
                !matches!(resolved(document, mode), Ok(Object::Name(name)) if name == b"Normal")
            }),
            _ => false,
        };
        if non_normal {
            findings.push(TransparencyFinding {
                page_number,
                kind: TransparencyKind::BlendMode,
                resource_name: Some(resource_name.clone()),
                detail: "non-Normal blend mode".into(),
            });
        }
    }
    if let Ok(mask) = state.get(b"SMask") {
        if !matches!(resolved(document, mask), Ok(Object::Name(name)) if name == b"None") {
            findings.push(TransparencyFinding {
                page_number,
                kind: TransparencyKind::SoftMask,
                resource_name: Some(resource_name),
                detail: "graphics state soft mask".into(),
            });
        }
    }
}

fn image_color_space(document: &Document, object: &Object) -> (ImageColorSpace, Option<String>) {
    let Ok(object) = resolved(document, object) else {
        return (ImageColorSpace::Unknown, None);
    };
    match object {
        Object::Name(name) => (
            match name.as_slice() {
                b"DeviceRGB" => ImageColorSpace::DeviceRgb,
                b"DeviceCMYK" => ImageColorSpace::DeviceCmyk,
                b"DeviceGray" => ImageColorSpace::DeviceGray,
                _ => ImageColorSpace::Unknown,
            },
            None,
        ),
        Object::Array(items) if !items.is_empty() => {
            let family = resolved(document, &items[0])
                .ok()
                .and_then(|item| item.as_name().ok());
            match family {
                Some(b"ICCBased") if items.len() > 1 => {
                    let profile = resolved(document, &items[1])
                        .ok()
                        .and_then(|item| item.as_stream().ok());
                    if let Some(profile) = profile {
                        let components =
                            dictionary_integer(&profile.dict, b"N").unwrap_or_default();
                        let bytes = stream_bytes(profile);
                        let sha256 = (!bytes.is_empty()).then(|| sha256(&bytes));
                        let space = match components {
                            1 => ImageColorSpace::IccGray,
                            3 => ImageColorSpace::IccRgb,
                            4 => ImageColorSpace::IccCmyk,
                            _ => ImageColorSpace::Unknown,
                        };
                        (space, sha256)
                    } else {
                        (ImageColorSpace::Unknown, None)
                    }
                }
                Some(b"Indexed") => (ImageColorSpace::Indexed, None),
                Some(b"Separation") => (ImageColorSpace::Separation, None),
                Some(b"DeviceN") => (ImageColorSpace::DeviceN, None),
                _ => (ImageColorSpace::Unknown, None),
            }
        }
        _ => (ImageColorSpace::Unknown, None),
    }
}

fn font_is_embedded(document: &Document, font: &Dictionary) -> bool {
    if let Ok(descriptor) = font.get(b"FontDescriptor") {
        if descriptor_has_font_file(document, descriptor) {
            return true;
        }
    }
    if let Ok(descendants) = font.get(b"DescendantFonts") {
        if let Ok(descendants) = resolved(document, descendants).and_then(Object::as_array) {
            for descendant in descendants {
                if let Ok(dictionary) = resolved(document, descendant).and_then(Object::as_dict) {
                    if let Ok(descriptor) = dictionary.get(b"FontDescriptor") {
                        if descriptor_has_font_file(document, descriptor) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn descriptor_has_font_file(document: &Document, object: &Object) -> bool {
    resolved(document, object)
        .and_then(Object::as_dict)
        .is_ok_and(|descriptor| {
            descriptor.has(b"FontFile")
                || descriptor.has(b"FontFile2")
                || descriptor.has(b"FontFile3")
        })
}

fn output_intent(document: &Document) -> Result<Option<OutputIntentData>> {
    let catalog = document
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|root| resolved(document, root).ok())
        .and_then(|root| root.as_dict().ok());
    let Some(catalog) = catalog else {
        return Ok(None);
    };
    let Some(first) = catalog
        .get(b"OutputIntents")
        .ok()
        .and_then(|intents| resolved(document, intents).ok())
        .and_then(|intents| intents.as_array().ok())
        .and_then(|intents| intents.first())
    else {
        return Ok(None);
    };
    let dictionary = resolved(document, first)
        .and_then(Object::as_dict)
        .map_err(|error| OpenPrintPdfError::InvalidPdf(error.to_string()))?;
    let profile_bytes = dictionary
        .get(b"DestOutputProfile")
        .ok()
        .and_then(|profile| resolved(document, profile).ok())
        .and_then(|profile| profile.as_stream().ok())
        .map(stream_bytes)
        .unwrap_or_default();
    let profile = (!profile_bytes.is_empty()).then(|| parse_icc_profile(&profile_bytes));
    let info = OutputIntentInfo {
        subtype: dictionary_name_from_dict(document, dictionary, b"S"),
        output_condition: dictionary_text(document, dictionary, b"OutputCondition"),
        output_condition_identifier: dictionary_text(
            document,
            dictionary,
            b"OutputConditionIdentifier",
        ),
        info: dictionary_text(document, dictionary, b"Info"),
        registry_name: dictionary_text(document, dictionary, b"RegistryName"),
        profile,
    };
    Ok(Some(OutputIntentData {
        info,
        profile_bytes,
    }))
}

fn pdfx_claim(document: &Document) -> Option<String> {
    if let Ok(info) = document.trailer.get(b"Info") {
        if let Ok(info) = resolved(document, info).and_then(Object::as_dict) {
            if let Some(value) = dictionary_text(document, info, b"GTS_PDFXConformance") {
                return Some(value);
            }
            if let Some(value) = dictionary_text(document, info, b"GTS_PDFXVersion") {
                return Some(value);
            }
        }
    }
    let catalog = document
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|root| resolved(document, root).ok())
        .and_then(|root| root.as_dict().ok())?;
    let metadata = catalog.get(b"Metadata").ok()?;
    let stream = resolved(document, metadata).ok()?.as_stream().ok()?;
    let metadata_bytes = stream_bytes(stream);
    let text = String::from_utf8_lossy(&metadata_bytes);
    for marker in ["PDF/X-1a:2001", "PDF/X-1a", "PDF/X-4", "PDF/X-3"] {
        if text.contains(marker) {
            return Some(marker.to_string());
        }
    }
    None
}

pub fn parse_icc_profile(data: &[u8]) -> IccProfileInfo {
    let text = |range: std::ops::Range<usize>| {
        data.get(range)
            .and_then(|value| std::str::from_utf8(value).ok())
            .map(|value| value.trim().to_string())
    };
    let version = data
        .get(8..12)
        .map(|value| format!("{}.{}.{}", value[0], value[1] >> 4, value[1] & 0x0f));
    let preferred_intent = data
        .get(64..68)
        .map(|value| u32::from_be_bytes(value.try_into().expect("four bytes")));
    let mut tags = Vec::new();
    if let Some(count_bytes) = data.get(128..132) {
        let count = u32::from_be_bytes(count_bytes.try_into().expect("four bytes")) as usize;
        for index in 0..count.min(4096) {
            let offset = 132 + index * 12;
            if let Some(signature) = data.get(offset..offset + 4) {
                if let Ok(signature) = std::str::from_utf8(signature) {
                    tags.push(signature.to_string());
                }
            }
        }
    }
    tags.sort();
    IccProfileInfo {
        byte_length: data.len(),
        sha256: sha256(data),
        version,
        device_class: text(12..16),
        color_space: text(16..20),
        pcs: text(20..24),
        preferred_intent,
        tags,
    }
}

fn stream_bytes(stream: &Stream) -> Vec<u8> {
    stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone())
}

fn sha256(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn inherited_box(document: &Document, page_id: ObjectId, key: &[u8]) -> Result<Option<PdfBox>> {
    let Some(object) = inherited_object(document, page_id, key) else {
        return Ok(None);
    };
    let array = resolved(document, &object)
        .and_then(Object::as_array)
        .map_err(|error| OpenPrintPdfError::InvalidPdf(error.to_string()))?;
    if array.len() != 4 {
        return Err(OpenPrintPdfError::InvalidPdf(format!(
            "{} must have four numbers",
            name_to_string(key)
        )));
    }
    let values = array.iter().map(number).collect::<Option<Vec<_>>>();
    let Some(values) = values else {
        return Err(OpenPrintPdfError::InvalidPdf(format!(
            "{} contains a non-numeric value",
            name_to_string(key)
        )));
    };
    Ok(Some(PdfBox {
        left: values[0],
        bottom: values[1],
        right: values[2],
        top: values[3],
    }))
}

fn inherited_integer(document: &Document, page_id: ObjectId, key: &[u8]) -> Option<i64> {
    inherited_object(document, page_id, key).and_then(|object| match object {
        Object::Integer(value) => Some(value),
        _ => None,
    })
}

fn inherited_object(document: &Document, mut id: ObjectId, key: &[u8]) -> Option<Object> {
    let mut visited = HashSet::new();
    while visited.insert(id) {
        let dictionary = document.get_dictionary(id).ok()?;
        if let Ok(value) = dictionary.get(key) {
            return Some(value.clone());
        }
        id = dictionary.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn resolved<'a>(document: &'a Document, object: &'a Object) -> lopdf::Result<&'a Object> {
    match object {
        Object::Reference(id) => document.get_object(*id),
        _ => Ok(object),
    }
}

fn dictionary_integer(dictionary: &Dictionary, key: &[u8]) -> Option<i64> {
    match dictionary.get(key).ok()? {
        Object::Integer(value) => Some(*value),
        _ => None,
    }
}

fn number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(*value as f64),
        _ => None,
    }
}

fn dictionary_name(document: &Document, object: &Object, key: &[u8]) -> Option<String> {
    let dictionary = resolved(document, object).ok()?.as_dict().ok()?;
    dictionary_name_from_dict(document, dictionary, key)
}

fn dictionary_name_from_dict(
    document: &Document,
    dictionary: &Dictionary,
    key: &[u8],
) -> Option<String> {
    let value = resolved(document, dictionary.get(key).ok()?).ok()?;
    value.as_name().ok().map(name_to_string)
}

fn dictionary_text(document: &Document, dictionary: &Dictionary, key: &[u8]) -> Option<String> {
    object_text(resolved(document, dictionary.get(key).ok()?).ok()?)
}

fn object_text(object: &Object) -> Option<String> {
    match object {
        Object::String(bytes, StringFormat::Literal | StringFormat::Hexadecimal) => {
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
        Object::Name(bytes) => Some(name_to_string(bytes)),
        _ => None,
    }
}

fn name_to_string(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

fn is_subset_font_name(name: &str) -> bool {
    name.as_bytes()
        .get(0..7)
        .is_some_and(|prefix| prefix[6] == b'+' && prefix[..6].iter().all(u8::is_ascii_uppercase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_icc_transform_tags() {
        let mut profile = vec![0_u8; 160];
        profile[8] = 4;
        profile[9] = 0x30;
        profile[12..16].copy_from_slice(b"prtr");
        profile[16..20].copy_from_slice(b"CMYK");
        profile[20..24].copy_from_slice(b"Lab ");
        profile[64..68].copy_from_slice(&1_u32.to_be_bytes());
        profile[128..132].copy_from_slice(&1_u32.to_be_bytes());
        profile[132..136].copy_from_slice(b"B2A1");
        let parsed = parse_icc_profile(&profile);
        assert_eq!(parsed.version.as_deref(), Some("4.3.0"));
        assert_eq!(parsed.color_space.as_deref(), Some("CMYK"));
        assert!(parsed.tags.contains(&"B2A1".to_string()));
    }

    #[test]
    fn detects_subset_font_names() {
        assert!(is_subset_font_name("ABCDEF+NotoSansJP"));
        assert!(!is_subset_font_name("NotoSansJP"));
    }
}
