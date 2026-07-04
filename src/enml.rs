//! Evernote ENML conversion.
//!
//! The MVP preserves Evernote formatting as HTML instead of trying to downgrade
//! fonts, sizes, colors, and tables to Markdown. Zola allows raw HTML in page
//! bodies, so this keeps rich notes faithful while still letting us rewrite
//! Evernote-specific tags and internal links.

use crate::models::Resource;
use html_escape::{encode_double_quoted_attribute, encode_text};
use regex::{Captures, Regex};
use std::collections::HashMap;

/// Convert an Evernote ENML document into Zola body HTML/shortcodes.
pub fn enml_to_zola_body(
	enml: &str,
	resources: &[Resource],
	note_slug_by_guid: &HashMap<String, String>,
) -> String {
	let by_hash = resources
		.iter()
		.map(|r| (r.hash.to_ascii_lowercase(), r))
		.collect::<HashMap<_, _>>();
	let mut out = enml.to_string();

	for pattern in [
		r#"(?is)<\?xml[^>]*>"#,
		r#"(?is)<!DOCTYPE[^>]*>"#,
		r#"(?is)</?en-note[^>]*>"#,
	] {
		out = Regex::new(pattern)
			.unwrap()
			.replace_all(&out, "")
			.into_owned();
	}

	out = Regex::new(r#"(?is)<en-todo\s+checked="true"\s*/?>"#)
		.unwrap()
		.replace_all(&out, r#"<input type="checkbox" checked disabled>"#)
		.into_owned();
	out = Regex::new(r#"(?is)<en-todo[^>]*/?>"#)
		.unwrap()
		.replace_all(&out, r#"<input type="checkbox" disabled>"#)
		.into_owned();

	out = Regex::new(r#"(?is)<en-media\b([^>]*)/?>"#)
		.unwrap()
		.replace_all(&out, |caps: &Captures| {
			media_replacement(caps.get(1).map(|m| m.as_str()).unwrap_or(""), &by_hash)
		})
		.into_owned();

	rewrite_internal_links(&out, note_slug_by_guid)
}

fn media_replacement(attrs: &str, by_hash: &HashMap<String, &Resource>) -> String {
	let hash = attr(attrs, "hash").unwrap_or_default().to_ascii_lowercase();
	let mime = attr(attrs, "type").unwrap_or_default();
	let Some(resource) = by_hash.get(&hash) else {
		return format!(
			r#"<span class="missing-resource">Missing Evernote resource {}</span>"#,
			encode_text(&hash)
		);
	};
	let file = encode_double_quoted_attribute(&resource.file_name);

	if mime.starts_with("image/") {
		format!(r#"<img src="{file}" alt="" loading="lazy">"#)
	} else if mime.starts_with("audio/") {
		format!(r#"{{{{ audio(src="{file}") }}}}"#)
	} else if mime.starts_with("video/") {
		format!(r#"{{{{ video(src="{file}") }}}}"#)
	} else {
		format!(
			r#"<a class="attachment" href="{file}" download>{}</a>"#,
			encode_text(&resource.file_name)
		)
	}
}

fn attr(attrs: &str, name: &str) -> Option<String> {
	let pattern = format!(r#"{name}\s*=\s*"([^"]+)""#);
	Regex::new(&pattern)
		.ok()?
		.captures(attrs)
		.and_then(|c| c.get(1))
		.map(|m| m.as_str().to_string())
}

/// Convert links to Evernote notes into relative Zola links when the referenced
/// note is part of the same generated website.
pub fn rewrite_internal_links(html: &str, note_slug_by_guid: &HashMap<String, String>) -> String {
	let evernote_link =
        Regex::new(r#"href="(?:evernote:///view/[^"]+?|https://www\.evernote\.com/shard/[^"]*?/nl/[^"]*?/)([a-f0-9-]{32,36})(?:/[^"]*)?""#)
            .unwrap();
	evernote_link
		.replace_all(html, |caps: &Captures| {
			let guid = caps.get(1).unwrap().as_str().to_ascii_lowercase();
			if let Some(slug) = note_slug_by_guid.get(&guid) {
				format!(r#"href="/posts/{slug}/""#)
			} else {
				caps.get(0).unwrap().as_str().to_string()
			}
		})
		.into_owned()
}

#[cfg(test)]
mod tests {
	use super::*;
	use pretty_assertions::assert_eq;

	#[test]
	fn replaces_en_media_with_playable_tags() {
		let resources = vec![Resource {
			hash: "abc".into(),
			file_name: "voice.mp3".into(),
			mime: "audio/mpeg".into(),
			s3_key: None,
		}];
		let body = enml_to_zola_body(
			r#"<en-note><en-media type="audio/mpeg" hash="abc"/></en-note>"#,
			&resources,
			&HashMap::new(),
		);

		assert_eq!(body, r#"{{ audio(src="voice.mp3") }}"#);
	}

	#[test]
	fn preserves_rich_html() {
		let body = enml_to_zola_body(
			r#"<en-note><span style="font-size: 20px; color: red">Text</span><table><tr><td>A</td></tr></table></en-note>"#,
			&[],
			&HashMap::new(),
		);

		assert!(body.contains("font-size: 20px"));
		assert!(body.contains("<table>"));
	}

	#[test]
	fn rewrites_internal_evernote_links() {
		let mut index = HashMap::new();
		index.insert(
			"01234567-89ab-cdef-0123-456789abcdef".into(),
			"linked-note".into(),
		);
		let html = r#"<a href="evernote:///view/1/s1/01234567-89ab-cdef-0123-456789abcdef/01234567-89ab-cdef-0123-456789abcdef/">x</a>"#;

		assert!(rewrite_internal_links(html, &index).contains(r#"/posts/linked-note/"#));
	}
}
