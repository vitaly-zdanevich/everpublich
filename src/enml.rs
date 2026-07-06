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
	} else if is_gltf_model(resource, &mime) {
		format!(
			r#"{{{{ model_viewer(src="{}", alt="{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			shortcode_arg(&resource.file_name)
		)
	} else if is_stl_model(resource, &mime) {
		format!(
			r#"{{{{ stl_viewer(src="{}", label="{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			shortcode_arg(&resource.file_name)
		)
	} else if let Some(kind) = model_viewer_kind(resource, &mime) {
		format!(
			r#"{{{{ three_model_viewer(src="{}", kind="{}", label="{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			kind,
			shortcode_arg(&resource.file_name)
		)
	} else if let Some(tree) = &resource.archive_tree {
		let title = encode_text(&resource.file_name);
		format!(
			r#"<details class="attachment-preview attachment-preview-archive"><summary>{title}</summary><pre>{}</pre><p><a class="attachment" href="{file}" download>Download {title}</a></p></details>"#,
			preview_pre_text(tree)
		)
	} else if is_text_preview(resource, &mime) {
		let title = encode_text(&resource.file_name);
		let body = resource
			.text_preview
			.as_deref()
			.map(|preview| format!("<pre>{}</pre>", preview_pre_text(preview)))
			.unwrap_or_else(|| {
				format!(r#"<iframe sandbox src="{file}" title="{file}" loading="lazy"></iframe>"#)
			});
		format!(
			r#"<details class="attachment-preview attachment-preview-text"><summary>{title}</summary>{body}<p><a class="attachment" href="{file}" download>Download {title}</a></p></details>"#
		)
	} else {
		format!(
			r#"<a class="attachment" href="{file}" download>{}</a>"#,
			encode_text(&resource.file_name)
		)
	}
}

fn is_gltf_model(resource: &Resource, mime: &str) -> bool {
	matches!(
		mime.to_ascii_lowercase().as_str(),
		"model/gltf-binary" | "model/gltf+json"
	) || matches!(
		file_extension(&resource.file_name).as_deref(),
		Some("glb" | "gltf")
	)
}

fn is_stl_model(resource: &Resource, mime: &str) -> bool {
	matches!(
		mime.to_ascii_lowercase().as_str(),
		"model/stl" | "model/x.stl-ascii" | "model/x.stl-binary" | "application/sla"
	) || file_extension(&resource.file_name).as_deref() == Some("stl")
}

fn model_viewer_kind(resource: &Resource, _mime: &str) -> Option<&'static str> {
	let extension = file_extension(&resource.file_name)?;
	match extension.as_str() {
		"obj" => Some("obj"),
		"ply" => Some("ply"),
		"3mf" => Some("3mf"),
		"dae" => Some("dae"),
		"fbx" => Some("fbx"),
		"3dm" => Some("3dm"),
		"vox" => Some("vox"),
		"vtk" => Some("vtk"),
		"vtp" => Some("vtp"),
		"xyz" => Some("xyz"),
		"gcode" | "g" | "gco" => Some("gcode"),
		_ => None,
	}
}

fn is_text_preview(resource: &Resource, mime: &str) -> bool {
	let mime = mime.to_ascii_lowercase();
	mime.starts_with("text/")
		|| matches!(
			mime.as_str(),
			"application/rtf" | "application/x-rtf" | "application/vnd.oasis.opendocument.text"
		) || matches!(
		file_extension(&resource.file_name).as_deref(),
		Some(
			"txt"
				| "text" | "md"
				| "markdown" | "rtf"
				| "log" | "srt"
				| "vtt" | "ass"
				| "ssa" | "sub"
				| "sbv" | "ttml"
				| "dfxp" | "csv"
				| "tsv" | "json"
				| "xml" | "yaml"
				| "yml" | "toml"
		)
	)
}

fn file_extension(file_name: &str) -> Option<String> {
	file_name
		.rsplit_once('.')
		.map(|(_, extension)| extension.to_ascii_lowercase())
}

fn shortcode_arg(value: &str) -> String {
	value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn preview_pre_text(text: &str) -> String {
	encode_text(text).replace('\n', "&#10;")
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
	let evernote_link = Regex::new(
		r#"(?is)<a\b([^>]*)\bhref="(?:evernote:///view/[^"]+?|https://www\.evernote\.com/shard/[^"]*?/nl/[^"]*?/)([a-f0-9-]{32,36})(?:/[^"]*)?"([^>]*)>"#,
	)
	.unwrap();
	evernote_link
		.replace_all(html, |caps: &Captures| {
			let guid = caps.get(2).unwrap().as_str().to_ascii_lowercase();
			if let Some(slug) = note_slug_by_guid.get(&guid) {
				let attrs = format!(
					r#"{}href="/posts/{slug}/"{}"#,
					caps.get(1).unwrap().as_str(),
					caps.get(3).unwrap().as_str()
				);
				format!("<a{}>", add_class_attr(&attrs, "internal-link"))
			} else {
				caps.get(0).unwrap().as_str().to_string()
			}
		})
		.into_owned()
}

fn add_class_attr(attrs: &str, class_name: &str) -> String {
	let class = Regex::new(r#"(?is)\bclass\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap();
	if let Some(caps) = class.captures(attrs) {
		let current = caps
			.get(1)
			.or_else(|| caps.get(2))
			.or_else(|| caps.get(3))
			.map(|value| value.as_str())
			.unwrap_or_default();
		if current.split_whitespace().any(|class| class == class_name) {
			return attrs.to_string();
		}
		let quoted = format!(r#"class="{current} {class_name}""#);
		let matched = caps.get(0).unwrap();
		let mut out = String::new();
		out.push_str(&attrs[..matched.start()]);
		out.push_str(&quoted);
		out.push_str(&attrs[matched.end()..]);
		return out;
	}
	format!(r#"{attrs} class="{class_name}""#)
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
			text_preview: None,
			archive_tree: None,
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

		let rewritten = rewrite_internal_links(html, &index);
		assert!(rewritten.contains(r#"/posts/linked-note/"#));
		assert!(rewritten.contains(r#"class="internal-link""#));
	}

	#[test]
	fn previews_text_attachments_and_models() {
		let resources = vec![
			Resource {
				hash: "txt".into(),
				file_name: "notes.md".into(),
				mime: "text/markdown".into(),
				s3_key: None,
				text_preview: Some("# Notes\n\nHello".into()),
				archive_tree: None,
			},
			Resource {
				hash: "glb".into(),
				file_name: "shape.glb".into(),
				mime: "model/gltf-binary".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
			},
			Resource {
				hash: "stl".into(),
				file_name: "mesh.stl".into(),
				mime: "model/stl".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
			},
		];
		let body = enml_to_zola_body(
			r#"<en-note><en-media type="text/markdown" hash="txt"/><en-media type="model/gltf-binary" hash="glb"/><en-media type="model/stl" hash="stl"/></en-note>"#,
			&resources,
			&HashMap::new(),
		);

		assert!(body.contains(r#"<details class="attachment-preview attachment-preview-text">"#));
		assert!(body.contains("<pre># Notes&#10;&#10;Hello</pre>"));
		assert!(body.contains(r#"{{ model_viewer(src="shape.glb", alt="shape.glb") }}"#));
		assert!(body.contains(r#"{{ stl_viewer(src="mesh.stl", label="mesh.stl") }}"#));
	}

	#[test]
	fn previews_archive_trees_subtitles_and_extra_3d_formats() {
		let resources = vec![
			Resource {
				hash: "zip".into(),
				file_name: "archive.zip".into(),
				mime: "application/zip".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: Some("docs/\ndocs/readme.txt".into()),
			},
			Resource {
				hash: "sub".into(),
				file_name: "movie.srt".into(),
				mime: "application/x-subrip".into(),
				s3_key: None,
				text_preview: Some("1\n00:00:00,000 --> 00:00:02,000\nHello".into()),
				archive_tree: None,
			},
			Resource {
				hash: "obj".into(),
				file_name: "mesh.obj".into(),
				mime: "model/obj".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
			},
		];
		let body = enml_to_zola_body(
			r#"<en-note><en-media type="application/zip" hash="zip"/><en-media type="application/x-subrip" hash="sub"/><en-media type="model/obj" hash="obj"/></en-note>"#,
			&resources,
			&HashMap::new(),
		);

		assert!(
			body.contains(r#"<details class="attachment-preview attachment-preview-archive">"#)
		);
		assert!(body.contains("docs/readme.txt"));
		assert!(body.contains("movie.srt"));
		assert!(body.contains("00:00:00,000 --&gt; 00:00:02,000"));
		assert!(
			body.contains(
				r#"{{ three_model_viewer(src="mesh.obj", kind="obj", label="mesh.obj") }}"#
			)
		);
	}
}
