use std::path::Path;

use lopdf::{Document, Object, Stream, dictionary};

use crate::{OpenPrintPdfError, Result, io_error};

const PDFX_VERSION: &str = "PDF/X-1:2001";
const PDFX_CONFORMANCE: &str = "PDF/X-1a:2001";

/// Adds the PDF/X-1a identification objects to an already converted PDF.
///
/// This intentionally does not rewrite page content through a PDF interpreter.
/// The input must already be PDF 1.3, CMYK, transparency-free, and have any
/// required fonts outlined.
pub fn attach_pdfx1a_identification(
    source_path: &Path,
    output_path: &Path,
    profile_bytes: &[u8],
    description: &str,
    output_condition_identifier: &str,
) -> Result<()> {
    let mut document =
        Document::load(source_path).map_err(|source| OpenPrintPdfError::PdfRead {
            path: source_path.to_path_buf(),
            source,
        })?;
    document.version = "1.3".into();

    let root_id = document
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|error| OpenPrintPdfError::InvalidPdf(error.to_string()))?;

    let profile_id = document.add_object(Stream::new(
        dictionary! {
            "N" => 4,
            "Alternate" => "DeviceCMYK",
        },
        profile_bytes.to_vec(),
    ));
    let output_intent_id = document.add_object(dictionary! {
        "Type" => "OutputIntent",
        "S" => "GTS_PDFX",
        "OutputCondition" => Object::string_literal(description.as_bytes().to_vec()),
        "Info" => Object::string_literal(description.as_bytes().to_vec()),
        "OutputConditionIdentifier" =>
            Object::string_literal(output_condition_identifier.as_bytes().to_vec()),
        "RegistryName" => Object::string_literal(b"http://www.color.org".to_vec()),
        "DestOutputProfile" => profile_id,
    });
    let metadata_id = document.add_object(Stream::new(
        dictionary! {
            "Type" => "Metadata",
            "Subtype" => "XML",
        },
        pdfx_metadata().as_bytes().to_vec(),
    ));

    let mut info = document
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|object| {
            if let Ok(id) = object.as_reference() {
                document.get_object(id).ok()
            } else {
                Some(object)
            }
        })
        .and_then(|object| object.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    info.set(
        "GTS_PDFXVersion",
        Object::string_literal(PDFX_VERSION.as_bytes().to_vec()),
    );
    info.set(
        "GTS_PDFXConformance",
        Object::string_literal(PDFX_CONFORMANCE.as_bytes().to_vec()),
    );
    info.set("Trapped", Object::Name(b"False".to_vec()));
    let info_id = document.add_object(info);
    document.trailer.set("Info", info_id);

    let catalog = document
        .get_object_mut(root_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| OpenPrintPdfError::InvalidPdf(error.to_string()))?;
    catalog.set(
        "OutputIntents",
        Object::Array(vec![Object::Reference(output_intent_id)]),
    );
    catalog.set("Metadata", metadata_id);

    document.compress();
    document
        .save(output_path)
        .map_err(|source| io_error(output_path, source))?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use lopdf::{Dictionary, Object};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn attaches_pdfx_objects_without_changing_existing_page_objects() {
        let work = tempdir().expect("temporary directory");
        let source = work.path().join("source.pdf");
        let output = work.path().join("output.pdf");
        let mut document = Document::with_version("1.3");
        let pages_id = document.add_object(dictionary! {
            "Type" => "Pages",
            "Kids" => Vec::<Object>::new(),
            "Count" => 0,
        });
        let marker_id = document.add_object(Stream::new(
            Dictionary::new(),
            b"existing page content marker".to_vec(),
        ));
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
            "OpenPrintPdfTestMarker" => marker_id,
        });
        document.trailer.set("Root", catalog_id);
        document.save(&source).expect("source PDF");

        let icc = b"test CMYK profile";
        attach_pdfx1a_identification(&source, &output, icc, "Test CMYK", "TEST-CMYK")
            .expect("PDF/X identification");

        let result = Document::load(&output).expect("output PDF");
        assert_eq!(result.version, "1.3");
        let root = result
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .and_then(|id| result.get_object(id))
            .and_then(Object::as_dict)
            .expect("catalog");
        let marker = root
            .get(b"OpenPrintPdfTestMarker")
            .and_then(Object::as_reference)
            .and_then(|id| result.get_object(id))
            .and_then(Object::as_stream)
            .expect("existing marker");
        assert_eq!(
            marker.decompressed_content().expect("marker bytes"),
            b"existing page content marker"
        );
        let output_intents = root
            .get(b"OutputIntents")
            .and_then(Object::as_array)
            .expect("OutputIntents array");
        let output_intent_id = output_intents
            .first()
            .and_then(|object| object.as_reference().ok())
            .expect("OutputIntent reference");
        let output_intent = result
            .get_object(output_intent_id)
            .and_then(Object::as_dict)
            .expect("OutputIntent");
        assert_eq!(
            output_intent
                .get(b"OutputConditionIdentifier")
                .and_then(Object::as_str)
                .expect("identifier"),
            b"TEST-CMYK"
        );
    }
}
