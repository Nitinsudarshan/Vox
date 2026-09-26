//! AI Conversation Import System.
//!
//! Handles official export packages from ChatGPT and Claude (.zip or .json),
//! allows inspecting multi-conversation archives, extracts available binary assets
//! (PDFs, code, images, docs) into the Vox vault, normalizes conversations into
//! canonical `WebCapturePayload`, and triggers structured context analysis.

pub mod chatgpt;
pub mod claude;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use serde::{Deserialize, Serialize};

use crate::capture::web::ContentBlock;
use crate::commands::CommandError;
use crate::providers::LLMClient;
use crate::vault::VaultManager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationExportItem {
    pub id: String,
    pub title: String,
    pub message_count: usize,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub has_assets: bool,
    pub asset_count: usize,
    pub already_imported_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportInspection {
    pub provider: String,
    pub provider_display: String,
    pub total_conversations: usize,
    pub conversations: Vec<ConversationExportItem>,
}

/// Converts raw text or markdown prose into structured `ContentBlock`s.
pub fn text_to_blocks(text: &str) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // 1. Code fence
        if line.starts_with("```") {
            let lang = line.trim_start_matches('`').trim();
            let language = if lang.is_empty() {
                None
            } else {
                Some(lang.to_string())
            };
            let mut code_lines = Vec::new();
            i += 1;
            while i < lines.len() {
                if lines[i].trim().starts_with("```") {
                    i += 1;
                    break;
                }
                code_lines.push(lines[i]);
                i += 1;
            }
            blocks.push(ContentBlock::Code {
                language,
                text: code_lines.join("\n"),
            });
            continue;
        }

        // 2. Heading
        if line.starts_with('#') {
            let level = line.chars().take_while(|c| *c == '#').count() as u8;
            let heading_text = line.trim_start_matches('#').trim().to_string();
            if !heading_text.is_empty() {
                blocks.push(ContentBlock::Heading {
                    level: level.clamp(1, 6),
                    text: heading_text,
                });
                i += 1;
                continue;
            }
        }

        // 3. Blockquote
        if line.starts_with('>') {
            let mut quote_lines = Vec::new();
            while i < lines.len() && lines[i].trim().starts_with('>') {
                quote_lines.push(lines[i].trim().trim_start_matches('>').trim());
                i += 1;
            }
            blocks.push(ContentBlock::Quote {
                text: quote_lines.join("\n"),
            });
            continue;
        }

        // 4. List item
        if line.starts_with("- ") || line.starts_with("* ") || (line.len() > 2 && line.chars().next().is_some_and(|c| c.is_ascii_digit()) && line.contains(". ")) {
            let ordered = line.chars().next().is_some_and(|c| c.is_ascii_digit());
            let mut items = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim();
                if l.starts_with("- ") || l.starts_with("* ") {
                    items.push(l[2..].trim().to_string());
                    i += 1;
                } else if let Some(dot_idx) = l.find(". ") {
                    if l[..dot_idx].chars().all(|c| c.is_ascii_digit()) {
                        items.push(l[dot_idx + 2..].trim().to_string());
                        i += 1;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            blocks.push(ContentBlock::List { ordered, items });
            continue;
        }

        // 5. Paragraph
        if !line.is_empty() {
            let mut para_lines = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim();
                if l.is_empty() || l.starts_with("```") || l.starts_with('#') || l.starts_with('>') || l.starts_with("- ") || l.starts_with("* ") {
                    break;
                }
                para_lines.push(lines[i]);
                i += 1;
            }
            blocks.push(ContentBlock::Paragraph {
                text: para_lines.join(" "),
            });
            continue;
        }

        i += 1;
    }

    if blocks.is_empty() && !text.trim().is_empty() {
        blocks.push(ContentBlock::Paragraph {
            text: text.trim().to_string(),
        });
    }

    blocks
}

/// An attachment name from an export, reduced to one plain file name.
///
/// Names come from the export's JSON and from its zip entries, and are
/// joined onto the capture's assets directory: a `..`, a separator of either
/// kind, or a drive prefix would write outside the vault. Returns `None` when
/// nothing safe is left.
pub(crate) fn safe_asset_file_name(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && *c != ':')
        .collect();
    // Windows drops trailing dots and spaces, so `..` in disguise is still `..`.
    let cleaned = cleaned.trim_end_matches(['.', ' ']).trim_start();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return None;
    }
    Some(cleaned.to_string())
}

/// The first `max_chars` characters of `text`, with an ellipsis if any were
/// left out. Export text is untrusted and multilingual, so this never cuts
/// at a byte count.
pub(crate) fn truncate_chars(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// The largest `conversations.json` read. Heavy users' exports run to
/// hundreds of megabytes; past this the import refuses rather than
/// exhausting memory.
const MAX_CONVERSATIONS_JSON_BYTES: u64 = 1024 * 1024 * 1024;

/// The largest single attachment copied out of an archive.
const MAX_ASSET_BYTES: u64 = 50_000_000;

/// What the first pass over an export finds: the conversations JSON, and how
/// many attachment files the archive holds (none for a bare JSON file).
struct ExportContents {
    conversations_json: Vec<u8>,
    asset_count: usize,
}

fn read_capped(reader: impl Read, limit: u64) -> Result<Vec<u8>, CommandError> {
    let mut buf = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| CommandError::new("FILE_READ_FAILED", &e.to_string()))?;
    if buf.len() as u64 > limit {
        return Err(CommandError::new(
            "EXPORT_TOO_LARGE",
            "The conversations file in this export is larger than Vox can import",
        ));
    }
    Ok(buf)
}

fn is_asset_entry(name: &str, is_dir: bool, size: u64) -> bool {
    !is_dir && !name.ends_with(".json") && size > 0 && size < MAX_ASSET_BYTES
}

/// Reads an export's conversations JSON, from a ZIP archive or a bare JSON
/// file, without reading any attachment.
fn read_export_contents(path: &Path) -> Result<ExportContents, CommandError> {
    let file = File::open(path).map_err(|e| CommandError::new("FILE_READ_FAILED", &e.to_string()))?;

    if let Ok(mut archive) = zip::ZipArchive::new(file) {
        let mut conversations_index = None;
        let mut any_json_index = None;
        let mut asset_count = 0;
        for idx in 0..archive.len() {
            let Ok(entry) = archive.by_index_raw(idx) else { continue };
            let name = entry.name().to_string();
            if name.ends_with(".json") && !name.contains("__MACOSX") {
                if name == "conversations.json" || name.ends_with("/conversations.json") {
                    conversations_index = Some(idx);
                } else if any_json_index.is_none() {
                    any_json_index = Some(idx);
                }
            } else if is_asset_entry(&name, entry.is_dir(), entry.size()) {
                asset_count += 1;
            }
        }
        if let Some(idx) = conversations_index.or(any_json_index) {
            let entry = archive
                .by_index(idx)
                .map_err(|e| CommandError::new("FILE_READ_FAILED", &e.to_string()))?;
            return Ok(ExportContents {
                conversations_json: read_capped(entry, MAX_CONVERSATIONS_JSON_BYTES)?,
                asset_count,
            });
        }
    }

    // Fallback: direct JSON file
    let file = File::open(path).map_err(|e| CommandError::new("FILE_READ_FAILED", &e.to_string()))?;
    Ok(ExportContents {
        conversations_json: read_capped(file, MAX_CONVERSATIONS_JSON_BYTES)?,
        asset_count: 0,
    })
}

/// Reads only the attachments a conversation refers to out of an archive.
/// A bare JSON export has none.
fn read_referenced_assets(path: &Path, wanted: &HashSet<String>) -> HashMap<String, Vec<u8>> {
    let mut assets = HashMap::new();
    if wanted.is_empty() {
        return assets;
    }
    let Ok(file) = File::open(path) else { return assets };
    let Ok(mut archive) = zip::ZipArchive::new(file) else { return assets };
    for idx in 0..archive.len() {
        let Ok(entry) = archive.by_index(idx) else { continue };
        if !is_asset_entry(entry.name(), entry.is_dir(), entry.size()) {
            continue;
        }
        let Some(name) = safe_asset_file_name(entry.name()) else { continue };
        if !wanted.contains(&name) || assets.contains_key(&name) {
            continue;
        }
        let mut buf = Vec::new();
        if entry.take(MAX_ASSET_BYTES).read_to_end(&mut buf).is_ok() {
            assets.insert(name, buf);
        }
    }
    assets
}

/// Runs archive reading off the async runtime: a large export takes seconds
/// to read, and an async worker blocked on it stalls every other command.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| CommandError::new("IMPORT_TASK_FAILED", &e.to_string()))?
}

/// Inspects an export file without writing to the vault, discovering conversations and providers.
pub async fn inspect_export_file(path: &Path, vault: &VaultManager) -> Result<ExportInspection, CommandError> {
    let owned = path.to_path_buf();
    let contents = blocking(move || read_export_contents(&owned)).await?;
    let bytes = contents.conversations_json;

    // Fetch existing captures for duplicate matching
    let existing_captures = vault.list_captures().unwrap_or_default();
    let find_existing = |url: &str| -> Option<String> {
        existing_captures.iter().find_map(|c| {
            if c.capture.as_ref().is_some_and(|p| p.url == url) {
                Some(c.id.clone())
            } else {
                None
            }
        })
    };

    let total_assets = contents.asset_count;

    // 1. Try ChatGPT
    if let Ok(chatgpt_convs) = chatgpt::parse_chatgpt_conversations(&bytes) {
        if !chatgpt_convs.is_empty() && !chatgpt_convs[0].mapping.is_empty() {
            let mut list = Vec::new();
            for c in chatgpt_convs {
                let id = c.conversation_id.clone().or_else(|| c.id.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let title = c.title.clone().unwrap_or_else(|| "ChatGPT Conversation".to_string());
                let msg_count = chatgpt::linearize_chatgpt_conversation(&c).len();
                let created_at = c.create_time.and_then(|epoch| {
                    chrono::DateTime::from_timestamp(epoch.trunc() as i64, 0).map(|dt| dt.to_rfc3339())
                });
                let updated_at = c.update_time.and_then(|epoch| {
                    chrono::DateTime::from_timestamp(epoch.trunc() as i64, 0).map(|dt| dt.to_rfc3339())
                });
                let url = format!("https://chatgpt.com/c/{}", id);
                let already_imported_id = find_existing(&url);

                list.push(ConversationExportItem {
                    id,
                    title,
                    message_count: msg_count,
                    created_at,
                    updated_at,
                    has_assets: total_assets > 0,
                    asset_count: total_assets,
                    already_imported_id,
                });
            }
            return Ok(ExportInspection {
                provider: "chatgpt".to_string(),
                provider_display: "ChatGPT Export".to_string(),
                total_conversations: list.len(),
                conversations: list,
            });
        }
    }

    // 2. Try Claude
    if let Ok(claude_convs) = claude::parse_claude_conversations(&bytes) {
        if !claude_convs.is_empty() {
            let mut list = Vec::new();
            for c in claude_convs {
                let id = c.uuid.clone();
                let title = c.name.clone().unwrap_or_else(|| "Claude Conversation".to_string());
                let msg_count = c.chat_messages.len();
                let url = format!("https://claude.ai/chat/{}", id);
                let already_imported_id = find_existing(&url);

                let has_embedded_content = c.chat_messages.iter().any(|m| {
                    !m.files.is_empty() || m.attachments.iter().any(|a| a.extracted_content.is_some())
                });

                list.push(ConversationExportItem {
                    id,
                    title,
                    message_count: msg_count,
                    created_at: c.created_at,
                    updated_at: c.updated_at,
                    has_assets: total_assets > 0 || has_embedded_content,
                    asset_count: total_assets,
                    already_imported_id,
                });
            }
            return Ok(ExportInspection {
                provider: "claude".to_string(),
                provider_display: "Claude Export".to_string(),
                total_conversations: list.len(),
                conversations: list,
            });
        }
    }

    Err(CommandError::new("UNRECOGNIZED_EXPORT", "Could not recognize file as a ChatGPT or Claude export archive"))
}

/// Imports a selected conversation from an export package into the vault.
pub async fn import_export_conversation(
    export_path: &Path,
    target_conversation_id: &str,
    duplicate_mode: Option<&str>,
    vault: &VaultManager,
    settings: &crate::settings::AppSettings,
) -> Result<crate::vault::VaultFile, CommandError> {
    let owned = export_path.to_path_buf();
    let bytes = blocking(move || read_export_contents(&owned)).await?.conversations_json;
    let assets_of = |wanted: HashSet<String>| {
        let owned = export_path.to_path_buf();
        blocking(move || Ok(read_referenced_assets(&owned, &wanted)))
    };

    // 1. Try ChatGPT
    if let Ok(chatgpt_convs) = chatgpt::parse_chatgpt_conversations(&bytes) {
        if let Some(target) = chatgpt_convs.into_iter().find(|c| {
            c.conversation_id.as_deref() == Some(target_conversation_id)
                || c.id.as_deref() == Some(target_conversation_id)
        }) {
            let assets = assets_of(chatgpt::referenced_asset_names(&target)).await?;
            return save_and_analyze_imported(
                target.title.as_deref().unwrap_or("ChatGPT Conversation"),
                |assets_dir| chatgpt::chatgpt_to_capture_payload(&target, Some(assets_dir), &assets),
                export_path,
                duplicate_mode,
                vault,
                settings,
            )
            .await;
        }
    }

    // 2. Try Claude
    if let Ok(claude_convs) = claude::parse_claude_conversations(&bytes) {
        if let Some(target) = claude_convs.into_iter().find(|c| c.uuid == target_conversation_id) {
            let assets = assets_of(claude::referenced_asset_names(&target)).await?;
            return save_and_analyze_imported(
                target.name.as_deref().unwrap_or("Claude Conversation"),
                |assets_dir| claude::claude_to_capture_payload(&target, Some(assets_dir), &assets),
                export_path,
                duplicate_mode,
                vault,
                settings,
            )
            .await;
        }
    }

    Err(CommandError::new("CONVERSATION_NOT_FOUND", "Specified conversation ID was not found in the export"))
}

async fn save_and_analyze_imported<F>(
    _raw_title: &str,
    payload_builder: F,
    _export_path: &Path,
    duplicate_mode: Option<&str>,
    vault: &VaultManager,
    settings: &crate::settings::AppSettings,
) -> Result<crate::vault::VaultFile, CommandError>
where
    F: FnOnce(&Path) -> crate::capture::web::WebCapturePayload,
{
    // Attachments are written to a staging folder first: the capture's own
    // id is only known once `save_capture` has run, and it may resolve to an
    // existing capture of the same conversation. Assets used to be written
    // under a second, invented id that nothing pointed to.
    let staging = vault
        .vault_dir()
        .join("captures")
        .join(format!(".import-{}", uuid::Uuid::new_v4()));
    let assets_dir = staging.join("assets");
    std::fs::create_dir_all(&assets_dir)
        .map_err(|e| CommandError::new("ASSET_DIR_FAILED", &e.to_string()))?;
    let result = save_staged_import(&assets_dir, payload_builder, duplicate_mode, vault, settings).await;
    let _ = std::fs::remove_dir_all(&staging);
    result
}

async fn save_staged_import<F>(
    assets_dir: &Path,
    payload_builder: F,
    duplicate_mode: Option<&str>,
    vault: &VaultManager,
    settings: &crate::settings::AppSettings,
) -> Result<crate::vault::VaultFile, CommandError>
where
    F: FnOnce(&Path) -> crate::capture::web::WebCapturePayload,
{
    let mut payload = payload_builder(assets_dir);

    // If duplicate_mode == Some("new"), assign unique URL so it doesn't merge/update previous capture
    if duplicate_mode == Some("new") {
        payload.url = format!("{}#import-{}", payload.url, uuid::Uuid::new_v4());
    }

    // Normalize
    let normalized = crate::capture::web::normalize::normalize(&payload)
        .map_err(|e| CommandError::new("NORMALIZATION_FAILED", &e.to_string()))?;

    // Save capture to vault
    let vault_file = vault
        .save_capture(normalized)
        .map_err(|e| CommandError::new("SAVE_CAPTURE_FAILED", &e.to_string()))?;
    move_staged_assets(assets_dir, &vault.vault_file_dir(&vault_file).join("assets"))
        .map_err(|e| CommandError::new("ASSET_MOVE_FAILED", &e.to_string()))?;

    // Run context analysis
    let llm = LLMClient::new(settings.provider.clone());
    // An imported export is a conversation capture, so this resolves to the
    // conversation prompt through the same dispatch a live capture uses —
    // §36's requirement that imports do not depend on the repository path.
    let context =
        crate::capture::web::context::extract_source_context(Some(&llm), &vault_file, &payload)
            .await;

    let _ = vault.save_capture_context(&vault_file.id, &context);

    Ok(vault_file)
}

/// Moves staged attachments into the capture they belong to, replacing any
/// same-named file an earlier import of the conversation left there.
fn move_staged_assets(staged: &Path, target: &Path) -> std::io::Result<()> {
    let mut entries = std::fs::read_dir(staged)?.peekable();
    if entries.peek().is_none() {
        return Ok(());
    }
    std::fs::create_dir_all(target)?;
    for entry in entries {
        let entry = entry?;
        let dest = target.join(entry.file_name());
        if dest.exists() {
            std::fs::remove_file(&dest)?;
        }
        std::fs::rename(entry.path(), dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vox_import_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    /// The first pass reads only the conversations; the second reads only
    /// the attachments the chosen conversation names.
    #[test]
    fn export_archives_are_read_in_two_bounded_passes() {
        let dir = scratch_dir();
        let archive = dir.join("export.zip");
        write_zip(
            &archive,
            &[
                ("conversations.json", b"[]"),
                ("files/spec.md", b"# spec"),
                ("files/unrelated.bin", b"xxxx"),
            ],
        );

        let contents = read_export_contents(&archive).unwrap();
        assert_eq!(contents.conversations_json, b"[]");
        assert_eq!(contents.asset_count, 2);

        let wanted: HashSet<String> = ["spec.md".to_string()].into_iter().collect();
        let assets = read_referenced_assets(&archive, &wanted);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets["spec.md"], b"# spec");

        let bare = dir.join("conversations.json");
        std::fs::write(&bare, b"[1]").unwrap();
        assert_eq!(read_export_contents(&bare).unwrap().conversations_json, b"[1]");
        assert!(read_referenced_assets(&bare, &wanted).is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Staged attachments end up inside the capture that was saved, and a
    /// re-import replaces rather than duplicates them.
    #[test]
    fn staged_assets_move_into_the_saved_capture() {
        let dir = scratch_dir();
        let staged = dir.join(".import-x").join("assets");
        let target = dir.join("capture_1").join("assets");
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(staged.join("spec.md"), "new").unwrap();
        std::fs::write(target.join("spec.md"), "old").unwrap();

        move_staged_assets(&staged, &target).unwrap();

        assert_eq!(std::fs::read_to_string(target.join("spec.md")).unwrap(), "new");
        assert!(std::fs::read_dir(&staged).unwrap().next().is_none());

        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        move_staged_assets(&empty, &dir.join("capture_2").join("assets")).unwrap();
        assert!(!dir.join("capture_2").exists(), "no attachments, no assets folder");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn text_to_blocks_handles_mixed_markdown() {
        let text = "Here is an introduction.\n\n# Architecture\n\n```rust\nfn hello() {}\n```\n\n- Point 1\n- Point 2\n\n> Important quote";
        let blocks = text_to_blocks(text);
        assert_eq!(blocks.len(), 5);
        assert!(matches!(&blocks[0], ContentBlock::Paragraph { .. }));
        assert!(matches!(&blocks[1], ContentBlock::Heading { level: 1, .. }));
        assert!(matches!(&blocks[2], ContentBlock::Code { language: Some(l), .. } if l == "rust"));
        assert!(matches!(&blocks[3], ContentBlock::List { ordered: false, items } if items.len() == 2));
        assert!(matches!(&blocks[4], ContentBlock::Quote { .. }));
    }
}
