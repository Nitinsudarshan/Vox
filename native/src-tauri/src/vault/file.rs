use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

use crate::capture::web::CaptureProvenance;
use crate::vault::scribble::{ScribbleAiMetadata, ScribbleRelationship};
use crate::vault::VaultError;

/// `file_type` marker for a web capture, distinguishing it from an imported
/// document without needing a second model. Text extraction never runs on
/// one: its content is produced by `capture::web::normalize`, not by reading
/// bytes back off disk.
pub const CAPTURE_FILE_TYPE: &str = "webcapture";

/// Core data model representing an imported document file in Relay's Vault.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VaultFile {
    pub id: String,
    pub original_filename: String,
    pub file_type: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub content_hash: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_known_source_path: String,
    pub vault_path: String,
    pub extraction_status: String, // "extracted" | "pending" | "failed" | "unsupported"
    pub processing_status: String, // "ready" | "processing" | "failed"
    pub content: String,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub topics: Vec<String>,
    pub entities: Vec<String>,
    pub relationships: Vec<ScribbleRelationship>,
    pub ai_metadata: ScribbleAiMetadata,
    #[serde(default)]
    pub linked_scribble_id: Option<String>,
    /// Set when this artifact came from a capture rather than a file import.
    ///
    /// Provenance only — where the content came from and how completely it
    /// was acquired. Semantic fields (`summary`, `tags`, `topics`,
    /// `entities`) are produced later by analysis and are deliberately kept
    /// out of here, so re-analysing a capture can never rewrite the record of
    /// its source. `None` on every imported file, which is what keeps this
    /// backwards compatible with vaults written before captures existed.
    #[serde(default)]
    pub capture: Option<CaptureProvenance>,
}

impl VaultFile {
    pub fn new(
        source_path: &Path,
        vault_relative_path: &str,
        size_bytes: u64,
        content_hash: String,
    ) -> Result<Self, VaultError> {
        let filename = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled")
            .to_string();

        let ext = source_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let mime_type = match ext.as_str() {
            "pdf" => "application/pdf",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "doc" => "application/msword",
            "md" | "markdown" => "text/markdown",
            "txt" => "text/plain",
            _ => "application/octet-stream",
        }
        .to_string();

        let now = chrono::Utc::now().to_rfc3339();
        let file_id = format!("file_{}", uuid::Uuid::new_v4());

        Ok(Self {
            id: file_id,
            original_filename: filename,
            file_type: ext,
            mime_type,
            size_bytes,
            content_hash,
            created_at: now.clone(),
            updated_at: now,
            last_known_source_path: source_path.to_string_lossy().to_string(),
            vault_path: vault_relative_path.to_string(),
            extraction_status: "pending".to_string(),
            processing_status: "processing".to_string(),
            content: String::new(),
            summary: None,
            tags: Vec::new(),
            topics: Vec::new(),
            entities: Vec::new(),
            relationships: Vec::new(),
            ai_metadata: ScribbleAiMetadata::default(),
            linked_scribble_id: None,
            capture: None,
        })
    }

    /// Builds a Vault artifact from a normalized web capture.
    ///
    /// `content` is already the normalized markdown, and `extraction_status`
    /// is `"extracted"` on arrival: unlike an imported PDF, a capture's text
    /// does not have to be recovered from a binary later, so there is no
    /// pending or failed extraction state to represent.
    pub fn new_capture(
        id: String,
        original_filename: String,
        vault_relative_path: String,
        content: String,
        content_hash: String,
        provenance: CaptureProvenance,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id,
            original_filename,
            file_type: CAPTURE_FILE_TYPE.to_string(),
            mime_type: "application/json".to_string(),
            size_bytes: content.len() as u64,
            content_hash,
            created_at: provenance.captured_at.clone(),
            updated_at: now,
            last_known_source_path: provenance.url.clone(),
            vault_path: vault_relative_path,
            extraction_status: "extracted".to_string(),
            processing_status: "ready".to_string(),
            content,
            summary: None,
            tags: Vec::new(),
            topics: Vec::new(),
            entities: Vec::new(),
            relationships: Vec::new(),
            ai_metadata: ScribbleAiMetadata::default(),
            linked_scribble_id: None,
            capture: Some(provenance),
        }
    }

    /// Whether this artifact is a capture rather than an imported document.
    pub fn is_capture(&self) -> bool {
        self.capture.is_some() || self.file_type == CAPTURE_FILE_TYPE || self.file_type == "capture"
    }
}

/// Calculates SHA-256 hash of a file for duplicate and modification detection.
pub fn calculate_file_hash(path: &Path) -> Result<String, VaultError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Extracts text content from a supported file format.
pub fn extract_text_from_file(file_path: &Path, file_type: &str) -> Result<String, VaultError> {
    match file_type.to_lowercase().as_str() {
        "md" | "markdown" | "txt" => {
            let bytes = fs::read(file_path)?;
            // Strip UTF-8 BOM if present
            let content_str = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                String::from_utf8_lossy(&bytes[3..]).to_string()
            } else {
                String::from_utf8_lossy(&bytes).to_string()
            };
            Ok(content_str)
        }
        "pdf" => {
            let bytes = fs::read(file_path)?;
            match pdf_extract::extract_text_from_mem(&bytes) {
                Ok(text) => {
                    let cleaned = text.trim().to_string();
                    if cleaned.is_empty() {
                        Err(VaultError::FrontmatterError(
                            "PDF contains no selectable text (scanned image or empty PDF)".to_string(),
                        ))
                    } else {
                        Ok(cleaned)
                    }
                }
                Err(e) => Err(VaultError::FrontmatterError(format!(
                    "PDF text extraction failed: {}",
                    e
                ))),
            }
        }
        "docx" => {
            let file = fs::File::open(file_path)?;
            let mut archive = ZipArchive::new(file)
                .map_err(|e| VaultError::FrontmatterError(format!("Invalid DOCX archive: {}", e)))?;

            let mut document_xml = archive
                .by_name("word/document.xml")
                .map_err(|e| VaultError::FrontmatterError(format!("DOCX missing word/document.xml: {}", e)))?;

            let mut xml_content = String::new();
            document_xml
                .read_to_string(&mut xml_content)
                .map_err(|e| VaultError::FrontmatterError(format!("Failed to read DOCX document XML: {}", e)))?;

            let extracted = extract_docx_xml_text(&xml_content);
            if extracted.trim().is_empty() {
                Err(VaultError::FrontmatterError(
                    "DOCX document contains no text".to_string(),
                ))
            } else {
                Ok(extracted)
            }
        }
        "doc" => Err(VaultError::FrontmatterError(
            "Legacy .doc format is not supported for text extraction. Please convert the file to .docx or .pdf."
                .to_string(),
        )),
        other => Err(VaultError::FrontmatterError(format!(
            "Unsupported file extension '.{}'",
            other
        ))),
    }
}

/// Extracts paragraph text from a DOCX `word/document.xml`.
///
/// Text inside `<w:t>` is kept to the character. Word splits a sentence into
/// runs wherever the formatting changes, and the space between two words often
/// sits at the edge of a run — `"This is "`, `"bold"`, `" text"` — so trimming
/// each piece (as this once did) glued the words together. Paragraphs are
/// trimmed once assembled instead. `<w:tab/>` and `<w:br/>` inside a run become
/// a tab and a line break; the `<w:tab>` tab-stop *definitions* in paragraph
/// properties are not text and are skipped.
///
/// Entity and character references (`&amp;`, `&#8217;`) arrive from the parser
/// as their own events and are resolved here.
fn extract_docx_xml_text(xml_content: &str) -> String {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(xml_content);

    let mut result = String::new();
    let mut current_paragraph = String::new();
    let mut in_text_node = false;
    let mut in_tab_stops = false;

    let flush = |paragraph: &mut String, result: &mut String| {
        let trimmed = paragraph.trim();
        if !trimmed.is_empty() {
            result.push_str(trimmed);
            result.push_str("\n\n");
        }
        paragraph.clear();
    };

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                "w:t" => in_text_node = true,
                "w:tabs" => in_tab_stops = true,
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.name().as_ref() {
                "w:tab" if !in_tab_stops => current_paragraph.push('\t'),
                "w:br" | "w:cr" => current_paragraph.push('\n'),
                _ => {}
            },
            Ok(Event::Text(e)) if in_text_node => {
                current_paragraph.push_str(&e.xml10_content());
            }
            Ok(Event::GeneralRef(e)) if in_text_node => {
                if let Ok(Some(ch)) = e.resolve_char_ref() {
                    current_paragraph.push(ch);
                } else if let Some(resolved) = quick_xml::escape::resolve_predefined_entity(&e) {
                    current_paragraph.push_str(resolved);
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                "w:t" => in_text_node = false,
                "w:tabs" => in_tab_stops = false,
                "w:p" => flush(&mut current_paragraph, &mut result),
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    flush(&mut current_paragraph, &mut result);

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF built by hand, so the tests need no fixture file.
    /// `extra` is appended to the catalog dictionary, which is how a test
    /// smuggles an arbitrary object into what the parser must read.
    fn minimal_pdf(text: &str, extra_catalog_entry: &str) -> Vec<u8> {
        let content = format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET");
        let objects = [
            format!("<< /Type /Catalog /Pages 2 0 R {extra_catalog_entry} >>"),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
                .to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn write_temp(name: &str, bytes: &[u8]) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("file_test_pdf_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn pdf_text_is_extracted() {
        let (dir, path) = write_temp("hello.pdf", &minimal_pdf("Hello Vox", ""));
        let extracted = extract_text_from_file(&path, "pdf").expect("text extracts");
        assert!(extracted.contains("Hello Vox"), "{extracted}");
        let _ = fs::remove_dir_all(dir);
    }

    /// RUSTSEC-2026-0187: lopdf < 0.42 recursed once per nesting level, so a
    /// hostile PDF of deeply nested arrays overflowed the stack and aborted
    /// the whole app — from a document the user merely imported. Whatever the
    /// parser now decides about such a file, the process must survive it.
    #[test]
    fn a_deeply_nested_pdf_does_not_overflow_the_stack() {
        let depth = 100_000;
        let nested = format!("/Nested {}{}", "[".repeat(depth), "]".repeat(depth));
        let (dir, path) = write_temp("nested.pdf", &minimal_pdf("Hello", &nested));

        // An explicit, modest stack, so the result does not depend on the
        // platform's default thread size.
        let outcome = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || extract_text_from_file(&path, "pdf").is_ok())
            .expect("spawn")
            .join();
        assert!(outcome.is_ok(), "parsing a nested PDF must not panic the thread");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn docx_text_keeps_the_spaces_between_runs() {
        let xml = r#"<w:document><w:body><w:p>
            <w:r><w:t xml:space="preserve">This is </w:t></w:r>
            <w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r>
            <w:r><w:t xml:space="preserve"> text</w:t></w:r>
        </w:p></w:body></w:document>"#;
        assert_eq!(extract_docx_xml_text(xml), "This is bold text");
    }

    #[test]
    fn docx_text_resolves_entity_and_character_references() {
        let xml = r#"<w:p><w:r><w:t>Q&amp;A &lt;draft&gt; Asha&#8217;s &#x2014; done</w:t></w:r></w:p>"#;
        assert_eq!(extract_docx_xml_text(xml), "Q&A <draft> Asha\u{2019}s \u{2014} done");
    }

    #[test]
    fn docx_text_keeps_run_tabs_and_breaks_but_not_tab_stop_definitions() {
        let xml = r#"<w:p>
            <w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs></w:pPr>
            <w:r><w:t>Name</w:t><w:tab/><w:t>Value</w:t><w:br/><w:t>Next line</w:t></w:r>
        </w:p>"#;
        assert_eq!(extract_docx_xml_text(xml), "Name\tValue\nNext line");
    }

    #[test]
    fn docx_paragraphs_are_separated_and_empty_ones_dropped() {
        let xml = r#"<w:body>
            <w:p><w:r><w:t>First</w:t></w:r></w:p>
            <w:p></w:p>
            <w:p><w:r><w:t xml:space="preserve">  Second  </w:t></w:r></w:p>
        </w:body>"#;
        assert_eq!(extract_docx_xml_text(xml), "First\n\nSecond");
    }

    #[test]
    fn test_extract_text_markdown_and_txt() {
        let dir = std::env::temp_dir().join(format!("file_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let md_path = dir.join("doc.md");
        fs::write(&md_path, "# Relay Architecture\n\nLocal-first knowledge engine.").unwrap();

        let extracted = extract_text_from_file(&md_path, "md").unwrap();
        assert!(extracted.contains("Relay Architecture"));

        let hash = calculate_file_hash(&md_path).unwrap();
        assert_eq!(hash.len(), 64);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_extract_text_legacy_doc_returns_error() {
        let dir = std::env::temp_dir().join(format!("file_test_doc_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let doc_path = dir.join("legacy.doc");
        fs::write(&doc_path, b"binary doc contents").unwrap();

        let res = extract_text_from_file(&doc_path, "doc");
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Legacy .doc format is not supported"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_docx_xml_text_extraction() {
        let xml = r#"
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:body>
                    <w:p><w:r><w:t>Project Blueprint</w:t></w:r></w:p>
                    <w:p><w:r><w:t>Local-first secure vault.</w:t></w:r></w:p>
                </w:body>
            </w:document>
        "#;
        let extracted = extract_docx_xml_text(xml);
        assert!(extracted.contains("Project Blueprint"));
        assert!(extracted.contains("Local-first secure vault."));
    }
}
