//! Native prompt capability gate (K21) and Rig user-message assembly.

use base64::engine::general_purpose::{STANDARD as BASE64, URL_SAFE};
use base64::Engine as _;
use rig::completion::message::{ImageMediaType, MimeType, UserContent};
use rig::completion::Message;

use crate::acp::types::PromptInputBlock;

/// Decoded-byte cap for one image forwarded to the model.
pub const MAX_DECODED_IMAGE_BYTES: usize = 4 * 1024 * 1024;

/// Text kept for the model plus dropped non-text attachments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePromptView {
    pub text: String,
    pub image_blocks: usize,
    pub dropped_non_text: usize,
}

impl NativePromptView {
    pub fn reject_reason(&self) -> Option<&'static str> {
        if !self.text.trim().is_empty() || self.image_blocks > 0 {
            None
        } else {
            Some("Prompt must contain at least one text block")
        }
    }

    pub fn omission_hint(&self) -> Option<String> {
        if self.dropped_non_text == 0 {
            return None;
        }
        Some(format!(
            "[Note: Codeg Agent ignored {} non-text attachment(s).]",
            self.dropped_non_text
        ))
    }

    /// User text plus an omission hint so mixed resource/text is not silent.
    pub fn model_text(&self) -> String {
        match self.omission_hint() {
            Some(hint) if self.text.trim().is_empty() => hint,
            Some(hint) => format!("{}\n\n{hint}", self.text),
            None => self.text.clone(),
        }
    }
}

pub fn inspect_native_prompt(blocks: &[PromptInputBlock]) -> NativePromptView {
    let mut texts = Vec::new();
    let mut image_blocks = 0usize;
    let mut dropped_non_text = 0usize;
    for block in blocks {
        match block {
            PromptInputBlock::Text { text } => texts.push(text.as_str()),
            PromptInputBlock::Image { .. } => {
                image_blocks += 1;
            }
            PromptInputBlock::Resource { .. } | PromptInputBlock::ResourceLink { .. } => {
                dropped_non_text += 1;
            }
        }
    }
    NativePromptView {
        text: texts.join("\n"),
        image_blocks,
        dropped_non_text,
    }
}

/// Build the Rig user message for one Prompt. Images become `UserContent::Image`
/// with base64 data; missing mime or decoded size over 4 MiB is skipped with a
/// text note. Audio/video/other resources stay dropped via [`NativePromptView::omission_hint`].
pub fn assemble_native_prompt(blocks: &[PromptInputBlock]) -> Message {
    let mut content: Vec<UserContent> = Vec::new();
    let mut skip_notes: Vec<String> = Vec::new();
    for block in blocks {
        match block {
            PromptInputBlock::Text { text } => {
                if !text.is_empty() {
                    content.push(UserContent::text(text.clone()));
                }
            }
            PromptInputBlock::Image {
                data, mime_type, ..
            } => match user_image_content(data, mime_type) {
                Ok(image) => content.push(image),
                Err(note) => skip_notes.push(format!("[Note: {note}.]")),
            },
            PromptInputBlock::Resource { .. } | PromptInputBlock::ResourceLink { .. } => {}
        }
    }
    if let Some(hint) = inspect_native_prompt(blocks).omission_hint() {
        content.push(UserContent::text(hint));
    }
    if !skip_notes.is_empty() {
        content.push(UserContent::text(skip_notes.join("\n")));
    }
    Message::User { content }
}

/// Text parts of an assembled prompt, used as the transcript / history stub.
/// Image bytes stay on the current-turn `Message` only.
pub fn native_prompt_store_text(message: &Message) -> String {
    match message {
        Message::User { content } => content
            .iter()
            .filter_map(|part| match part {
                UserContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn user_image_content(data: &str, mime_type: &str) -> Result<UserContent, String> {
    let mime = normalize_image_mime(mime_type);
    if mime.is_empty() {
        return Err("skipped an image with missing mime type".into());
    }
    let media_type = ImageMediaType::from_mime_type(&mime)
        .ok_or_else(|| format!("skipped an image with unsupported mime type ({mime})"))?;
    let bytes = decode_image_bytes(data);
    if bytes.is_empty() {
        return Err("skipped an image with empty payload".into());
    }
    if bytes.len() > MAX_DECODED_IMAGE_BYTES {
        return Err("skipped an image exceeding the 4 MiB limit".into());
    }
    Ok(UserContent::image_base64(
        BASE64.encode(bytes),
        Some(media_type),
        None,
    ))
}

fn normalize_image_mime(mime_type: &str) -> String {
    let mime = mime_type.trim().to_ascii_lowercase();
    match mime.as_str() {
        "image/jpg" => "image/jpeg".to_string(),
        _ => mime,
    }
}

fn decode_image_bytes(data: &str) -> Vec<u8> {
    let payload = match data.split_once("base64,") {
        Some((_, b64)) => b64.trim(),
        None => data.trim(),
    };
    if let Some(bytes) = decode_base64(payload) {
        return bytes;
    }
    payload.as_bytes().to_vec()
}

fn decode_base64(payload: &str) -> Option<Vec<u8>> {
    if let Ok(bytes) = BASE64.decode(payload.as_bytes()) {
        return Some(bytes);
    }
    let padded = pad_base64(payload);
    if padded != payload {
        if let Ok(bytes) = BASE64.decode(padded.as_bytes()) {
            return Some(bytes);
        }
    }
    if let Ok(bytes) = URL_SAFE.decode(payload.as_bytes()) {
        return Some(bytes);
    }
    let padded = pad_base64(payload);
    URL_SAFE.decode(padded.as_bytes()).ok()
}

fn pad_base64(payload: &str) -> String {
    let rem = payload.len() % 4;
    if rem == 0 {
        payload.to_string()
    } else {
        let mut padded = payload.to_string();
        padded.extend(std::iter::repeat_n('=', 4 - rem));
        padded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::message::DocumentSourceKind;

    #[test]
    fn text_only_has_no_hint() {
        let view = inspect_native_prompt(&[PromptInputBlock::Text {
            text: "hello".into(),
        }]);
        assert_eq!(view.reject_reason(), None);
        assert_eq!(view.model_text(), "hello");
        assert!(view.omission_hint().is_none());
    }

    #[test]
    fn image_only_is_accepted() {
        let view = inspect_native_prompt(&[PromptInputBlock::Image {
            data: "abc".into(),
            mime_type: "image/png".into(),
            uri: None,
        }]);
        assert_eq!(view.reject_reason(), None);
        assert_eq!(view.image_blocks, 1);
        assert_eq!(view.dropped_non_text, 0);
        assert!(view.omission_hint().is_none());
    }

    #[test]
    fn whitespace_plus_image_is_accepted() {
        let view = inspect_native_prompt(&[
            PromptInputBlock::Text {
                text: "  \n".into(),
            },
            PromptInputBlock::Image {
                data: "abc".into(),
                mime_type: "image/png".into(),
                uri: None,
            },
        ]);
        assert_eq!(view.reject_reason(), None);
        assert_eq!(view.image_blocks, 1);
        assert_eq!(view.dropped_non_text, 0);
    }

    #[test]
    fn mixed_text_and_image_keeps_image_without_omission_hint() {
        let view = inspect_native_prompt(&[
            PromptInputBlock::Text {
                text: "describe this".into(),
            },
            PromptInputBlock::Image {
                data: "abc".into(),
                mime_type: "image/png".into(),
                uri: None,
            },
        ]);
        assert_eq!(view.reject_reason(), None);
        assert_eq!(view.image_blocks, 1);
        assert_eq!(view.dropped_non_text, 0);
        assert_eq!(view.model_text(), "describe this");
        assert!(view.omission_hint().is_none());
        let dumped = view.model_text();
        assert!(!dumped.contains("images are not supported"), "{dumped}");
    }

    #[test]
    fn mixed_text_and_resource_hints_without_claiming_images_unsupported() {
        let view = inspect_native_prompt(&[
            PromptInputBlock::Text {
                text: "see attached".into(),
            },
            PromptInputBlock::Resource {
                uri: "file://clip.wav".into(),
                mime_type: Some("audio/wav".into()),
                text: None,
                blob: Some("abc".into()),
            },
        ]);
        assert_eq!(view.reject_reason(), None);
        assert_eq!(view.image_blocks, 0);
        assert_eq!(view.dropped_non_text, 1);
        let text = view.model_text();
        assert!(text.starts_with("see attached"), "{text}");
        assert!(text.contains("ignored 1 non-text"), "{text}");
        assert!(!text.contains("images are not supported"), "{text}");
    }

    #[test]
    fn empty_text_is_rejected() {
        let view = inspect_native_prompt(&[PromptInputBlock::Text { text: "  ".into() }]);
        assert_eq!(
            view.reject_reason(),
            Some("Prompt must contain at least one text block")
        );
    }

    #[test]
    fn assemble_native_prompt_keeps_text_and_png() {
        let message = assemble_native_prompt(&[
            PromptInputBlock::Text {
                text: "describe this".into(),
            },
            PromptInputBlock::Image {
                data: "iVBORw0KGgo=".into(),
                mime_type: "image/png".into(),
                uri: None,
            },
        ]);
        let Message::User { content } = message else {
            panic!("expected user message: {message:?}");
        };
        assert_eq!(content.len(), 2, "{content:?}");
        assert!(
            matches!(&content[0], UserContent::Text(text) if text.text == "describe this"),
            "{content:?}"
        );
        match &content[1] {
            UserContent::Image(image) => {
                assert_eq!(image.media_type, Some(ImageMediaType::PNG));
                let DocumentSourceKind::Base64(b64) = &image.data else {
                    panic!("expected base64 image, got {:?}", image.data);
                };
                assert_eq!(
                    BASE64.decode(b64).unwrap(),
                    BASE64.decode("iVBORw0KGgo=").unwrap()
                );
            }
            other => panic!("expected image, got {other:?}"),
        }
    }

    #[test]
    fn assemble_native_prompt_image_only() {
        let message = assemble_native_prompt(&[PromptInputBlock::Image {
            data: "iVBORw0KGgo=".into(),
            mime_type: "image/png".into(),
            uri: None,
        }]);
        let Message::User { content } = &message else {
            panic!("expected user message: {message:?}");
        };
        assert_eq!(content.len(), 1, "{content:?}");
        assert!(matches!(content[0], UserContent::Image(_)));
        assert!(native_prompt_store_text(&message).is_empty());
    }

    #[test]
    fn assemble_skips_image_with_missing_mime() {
        let message = assemble_native_prompt(&[
            PromptInputBlock::Text {
                text: "look".into(),
            },
            PromptInputBlock::Image {
                data: "iVBORw0KGgo=".into(),
                mime_type: String::new(),
                uri: None,
            },
        ]);
        let text = native_prompt_store_text(&message);
        assert!(text.contains("look"), "{text}");
        assert!(text.contains("missing mime"), "{text}");
        let Message::User { content } = message else {
            panic!("expected user message");
        };
        assert!(
            content
                .iter()
                .all(|part| !matches!(part, UserContent::Image(_))),
            "{content:?}"
        );
    }

    #[test]
    fn assemble_skips_image_over_decoded_size_cap() {
        let raw = vec![0u8; MAX_DECODED_IMAGE_BYTES + 1];
        let message = assemble_native_prompt(&[
            PromptInputBlock::Text { text: "big".into() },
            PromptInputBlock::Image {
                data: BASE64.encode(&raw),
                mime_type: "image/png".into(),
                uri: None,
            },
        ]);
        let text = native_prompt_store_text(&message);
        assert!(text.contains("4 MiB"), "{text}");
        let Message::User { content } = message else {
            panic!("expected user message");
        };
        assert!(
            content
                .iter()
                .all(|part| !matches!(part, UserContent::Image(_))),
            "{content:?}"
        );
    }

    #[test]
    fn assemble_encodes_raw_bytes_as_base64() {
        let message = assemble_native_prompt(&[PromptInputBlock::Image {
            data: "not base64 !!".into(),
            mime_type: "image/jpeg".into(),
            uri: None,
        }]);
        let Message::User { content } = message else {
            panic!("expected user message");
        };
        match &content[0] {
            UserContent::Image(image) => {
                assert_eq!(image.media_type, Some(ImageMediaType::JPEG));
                let DocumentSourceKind::Base64(b64) = &image.data else {
                    panic!("expected base64, got {:?}", image.data);
                };
                assert_eq!(BASE64.decode(b64).unwrap(), b"not base64 !!");
            }
            other => panic!("expected image, got {other:?}"),
        }
    }
}
