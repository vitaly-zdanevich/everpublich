//! Evernote ENML conversion.
//!
//! The MVP preserves Evernote formatting as HTML instead of trying to downgrade
//! fonts, sizes, colors, and tables to Markdown. Zola allows raw HTML in page
//! bodies, so this keeps rich notes faithful while still letting us rewrite
//! Evernote-specific tags and internal links.

use crate::models::Resource;
use html_escape::{encode_double_quoted_attribute, encode_text};
use regex::{Captures, Regex};
use std::collections::{HashMap, HashSet};

/// Attachment rendering options parsed from an `everpublich:config` note.
#[derive(Debug, Clone, Copy)]
pub struct EnmlRenderOptions<'a> {
	/// Whether attachment previews are rendered at all.
	pub previews_enabled: bool,
	/// Lowercase file names whose previews should be disabled.
	pub disabled_preview_files: &'a HashSet<String>,
}

impl Default for EnmlRenderOptions<'_> {
	fn default() -> Self {
		Self {
			previews_enabled: true,
			disabled_preview_files: empty_disabled_preview_files(),
		}
	}
}

fn empty_disabled_preview_files() -> &'static HashSet<String> {
	static FILES: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
	FILES.get_or_init(HashSet::new)
}

/// Convert an Evernote ENML document into Zola body HTML/shortcodes.
pub fn enml_to_zola_body(
	enml: &str,
	resources: &[Resource],
	note_slug_by_guid: &HashMap<String, String>,
) -> String {
	enml_to_zola_body_with_options(
		enml,
		resources,
		note_slug_by_guid,
		&EnmlRenderOptions::default(),
	)
}

/// Convert an Evernote ENML document with notebook-level render options.
pub fn enml_to_zola_body_with_options(
	enml: &str,
	resources: &[Resource],
	note_slug_by_guid: &HashMap<String, String>,
	options: &EnmlRenderOptions<'_>,
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
			media_replacement(
				caps.get(1).map(|m| m.as_str()).unwrap_or(""),
				&by_hash,
				options,
			)
		})
		.into_owned();

	out = normalize_evernote_rich_blocks(&out);

	rewrite_internal_links(&out, note_slug_by_guid)
}

/// Convert Evernote desktop rich blocks that are stored as CSS custom
/// properties into semantic HTML understood by browsers.
fn normalize_evernote_rich_blocks(html: &str) -> String {
	let out = convert_evernote_todo_lists(html);
	let out = convert_evernote_toggles(&out);
	convert_evernote_callouts(&out)
}

/// Render Evernote task lists as disabled browser checkboxes.
fn convert_evernote_todo_lists(html: &str) -> String {
	let list = Regex::new(r#"(?is)<ul\b(?P<attrs>[^>]*)>(?P<body>.*?)</ul>"#).unwrap();
	list.replace_all(html, |caps: &Captures| {
		let attrs = caps.name("attrs").map(|m| m.as_str()).unwrap_or_default();
		if evernote_style_value(attrs, "--en-todo").is_none_or(|value| value != "true") {
			return caps.get(0).unwrap().as_str().to_string();
		}
		let body =
			convert_evernote_todo_items(caps.name("body").map(|m| m.as_str()).unwrap_or_default());
		format!(r#"<ul class="evernote-todo-list">{body}</ul>"#)
	})
	.into_owned()
}

/// Convert individual Evernote task list items into checkbox labels.
fn convert_evernote_todo_items(html: &str) -> String {
	let item = Regex::new(r#"(?is)<li\b(?P<attrs>[^>]*)>(?P<body>.*?)</li>"#).unwrap();
	item.replace_all(html, |caps: &Captures| {
		let attrs = caps.name("attrs").map(|m| m.as_str()).unwrap_or_default();
		let mut checked =
			evernote_style_value(attrs, "--en-checked").is_some_and(|value| value == "true");
		let mut body = caps.name("body").map(|m| m.as_str()).unwrap_or_default();
		if let Some((div_attrs, div_body)) = unwrap_single_div(body) {
			checked |=
				evernote_style_value(div_attrs, "--en-checked").is_some_and(|value| value == "true");
			body = div_body;
		}
		let checked_attr = if checked { " checked" } else { "" };
		format!(
			r#"<li class="evernote-todo-item"><label><input type="checkbox"{checked_attr} disabled> <span>{body}</span></label></li>"#
		)
	})
	.into_owned()
}

/// Render Evernote toggles as native disclosure widgets.
fn convert_evernote_toggles(html: &str) -> String {
	let toggle = Regex::new(
		r#"(?is)<div\b(?P<attrs>[^>]*--en-toggle\s*:\s*true[^>]*)>\s*<div\b(?P<summary_attrs>[^>]*--en-toggleSummary\s*:\s*true[^>]*)>(?P<summary>.*?)</div>\s*<div\b(?P<content_attrs>[^>]*--en-toggleContent\s*:\s*true[^>]*)>(?P<content>(?:\s*<div\b[^>]*>.*?</div>\s*)+)</div>\s*</div>"#,
	)
	.unwrap();
	toggle
		.replace_all(html, |caps: &Captures| {
			let attrs = caps.name("attrs").map(|m| m.as_str()).unwrap_or_default();
			let open = evernote_style_value(attrs, "--en-isCollapsed")
				.is_none_or(|value| value != "true");
			let open_attr = if open { " open" } else { "" };
			let summary = caps.name("summary").map(|m| m.as_str()).unwrap_or_default();
			let content = caps.name("content").map(|m| m.as_str()).unwrap_or_default();
			format!(
				r#"<details class="evernote-toggle"{open_attr}><summary>{summary}</summary><div class="evernote-toggle__content">{content}</div></details>"#
			)
		})
		.into_owned()
}

/// Render Evernote callouts with their emoji marker and content panel.
fn convert_evernote_callouts(html: &str) -> String {
	let callout = Regex::new(
		r#"(?is)<div\b(?P<attrs>[^>]*--en-callout\s*:\s*true[^>]*)>\s*<div\b[^>]*>(?P<body>.*?)</div>\s*</div>"#,
	)
	.unwrap();
	callout
		.replace_all(html, |caps: &Captures| {
			let attrs = caps.name("attrs").map(|m| m.as_str()).unwrap_or_default();
			let emoji = evernote_style_value(attrs, "--en-emoji")
				.filter(|emoji| !emoji.trim().is_empty())
				.map(|emoji| {
					format!(
						r#"<span class="evernote-callout__emoji" aria-hidden="true">{}</span>"#,
						encode_text(&emoji)
					)
				})
				.unwrap_or_default();
			let body = caps.name("body").map(|m| m.as_str()).unwrap_or_default();
			format!(
				r#"<aside class="evernote-callout">{emoji}<div class="evernote-callout__body">{body}</div></aside>"#
			)
		})
		.into_owned()
}

/// Return the body and attributes for an HTML fragment that is one wrapping div.
fn unwrap_single_div(html: &str) -> Option<(&str, &str)> {
	let div = Regex::new(r#"(?is)^\s*<div\b(?P<attrs>[^>]*)>(?P<body>.*?)</div>\s*$"#).unwrap();
	let caps = div.captures(html)?;
	Some((
		caps.name("attrs").map(|m| m.as_str()).unwrap_or_default(),
		caps.name("body").map(|m| m.as_str()).unwrap_or_default(),
	))
}

/// Read one Evernote CSS custom property from an element's attribute text.
fn evernote_style_value(attrs: &str, property: &str) -> Option<String> {
	let style = style_attr(attrs)?;
	style.split(';').find_map(|declaration| {
		let (name, value) = declaration.split_once(':')?;
		if name.trim().eq_ignore_ascii_case(property) {
			Some(value.trim().trim_matches('"').to_string())
		} else {
			None
		}
	})
}

/// Extract a `style` attribute from double-quoted, single-quoted, or compact
/// unquoted HTML attributes produced by the Evernote cache serializer.
fn style_attr(attrs: &str) -> Option<String> {
	let style = Regex::new(r#"(?is)\bstyle\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap();
	let caps = style.captures(attrs)?;
	caps.get(1)
		.or_else(|| caps.get(2))
		.or_else(|| caps.get(3))
		.map(|m| m.as_str().to_string())
}

fn media_replacement(
	attrs: &str,
	by_hash: &HashMap<String, &Resource>,
	options: &EnmlRenderOptions<'_>,
) -> String {
	let hash = attr(attrs, "hash").unwrap_or_default().to_ascii_lowercase();
	let source_mime = attr(attrs, "type").unwrap_or_default();
	let Some(resource) = by_hash.get(&hash) else {
		return format!(
			r#"<span class="missing-resource">Missing Evernote resource {}</span>"#,
			encode_text(&hash)
		);
	};
	let mime = if resource.mime.trim().is_empty() {
		source_mime.as_str()
	} else {
		resource.mime.as_str()
	};
	if preview_disabled(resource, options) {
		return attachment_download_link_for_resource(
			resource,
			resource
				.original_file_name
				.as_deref()
				.unwrap_or(&resource.file_name),
		);
	}
	let file = encode_double_quoted_attribute(&resource.file_name);

	if mime.starts_with("image/") {
		if let Some(original_file_name) = &resource.original_file_name {
			let original = encode_double_quoted_attribute(original_file_name);
			let title = encode_text(original_file_name);
			format!(
				r#"<figure class="attachment-preview-image"><img src="{file}" alt="" loading="lazy"><figcaption><a class="attachment" href="{original}" download>Download original {title}</a></figcaption></figure>"#
			)
		} else {
			format!(r#"<img src="{file}" alt="" loading="lazy">"#)
		}
	} else if is_midi(resource, mime) {
		format!(
			r#"{{{{ midi_player(src="{}", label="{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			shortcode_arg(&resource.file_name)
		)
	} else if mime.starts_with("audio/") {
		format!(r#"{{{{ audio(src="{file}") }}}}"#)
	} else if mime.starts_with("video/") {
		format!(r#"{{{{ video(src="{file}") }}}}"#)
	} else if is_swf(resource, mime) {
		format!(
			r#"{{{{ ruffle(src="{}", label="{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			shortcode_arg(&resource.file_name)
		)
	} else if is_epub(resource, mime) {
		format!(
			r#"{{{{ epub_viewer(src="{}", label="{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			shortcode_arg(&resource.file_name)
		)
	} else if let Some(kind) = comic_book_kind(resource, mime) {
		format!(
			r#"{{{{ comic_viewer(src="{}", kind="{}", label="{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			kind,
			shortcode_arg(&resource.file_name)
		)
	} else if is_font(resource, mime) {
		format!(
			r#"{{{{ font_preview(src="{}", label="{}", family="everpublich-font-{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			shortcode_arg(&resource.file_name),
			shortcode_arg(&resource.hash)
		)
	} else if is_gltf_model(resource, mime) {
		format!(
			r#"{{{{ model_viewer(src="{}", alt="{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			shortcode_arg(&resource.file_name)
		)
	} else if is_stl_model(resource, mime) {
		format!(
			r#"{{{{ stl_viewer(src="{}", label="{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			shortcode_arg(&resource.file_name)
		)
	} else if let Some(kind) = model_viewer_kind(resource, mime) {
		format!(
			r#"{{{{ three_model_viewer(src="{}", kind="{}", label="{}") }}}}"#,
			shortcode_arg(&resource.file_name),
			kind,
			shortcode_arg(&resource.file_name)
		)
	} else if let Some(tree) = &resource.archive_tree {
		let title = encode_text(&resource.file_name);
		let title_attr = attachment_title_attr(resource);
		format!(
			r#"<details class="attachment-preview attachment-preview-archive"><summary{title_attr}>{title}</summary><pre>{}</pre><p><a class="attachment" href="{file}" download{title_attr}>Download {title}</a></p></details>"#,
			preview_pre_text(tree),
		)
	} else if is_pdf(resource, mime) {
		let title = encode_text(&resource.file_name);
		let title_attr = attachment_title_attr(resource);
		format!(
			r#"<details class="attachment-preview attachment-preview-pdf"><summary{title_attr}>{title}</summary><iframe src="{file}" title="{title}" loading="lazy"></iframe><p><a class="attachment" href="{file}" download{title_attr}>Download {title}</a></p></details>"#
		)
	} else if is_text_preview(resource, mime) {
		let title = encode_text(&resource.file_name);
		let title_attr = attachment_title_attr(resource);
		let body = resource
			.text_preview
			.as_deref()
			.map(|preview| format!("<pre>{}</pre>", preview_pre_text(preview)))
			.unwrap_or_else(|| {
				format!(r#"<iframe sandbox src="{file}" title="{file}" loading="lazy"></iframe>"#)
			});
		format!(
			r#"<details class="attachment-preview attachment-preview-text"><summary{title_attr}>{title}</summary>{body}<p><a class="attachment" href="{file}" download{title_attr}>Download {title}</a></p></details>"#
		)
	} else {
		attachment_download_link_for_resource(resource, &resource.file_name)
	}
}

fn preview_disabled(resource: &Resource, options: &EnmlRenderOptions<'_>) -> bool {
	!options.previews_enabled
		|| options
			.disabled_preview_files
			.contains(&resource.file_name.to_ascii_lowercase())
		|| resource
			.original_file_name
			.as_ref()
			.is_some_and(|file_name| {
				options
					.disabled_preview_files
					.contains(&file_name.to_ascii_lowercase())
			})
}

fn attachment_download_link_for_resource(resource: &Resource, file_name: &str) -> String {
	let file = encode_double_quoted_attribute(file_name);
	let title_attr = attachment_title_attr(resource);
	format!(
		r#"<a class="attachment" href="{file}" download{title_attr}>{}</a>"#,
		encode_text(file_name)
	)
}

fn attachment_title_attr(resource: &Resource) -> String {
	let Some(title) = attachment_title(resource) else {
		return String::new();
	};
	let title = encode_double_quoted_attribute(&title)
		.replace('\n', "&#10;")
		.replace('`', "&#96;");
	format!(r#" title="{title}""#)
}

fn attachment_title(resource: &Resource) -> Option<String> {
	let mut lines = Vec::new();
	if let Some(size) = resource.size_bytes {
		lines.push(format!("Size: {}", format_attachment_size(size)));
	}
	if let Some(tree) = resource.archive_tree.as_deref().and_then(non_empty_trimmed) {
		if !lines.is_empty() {
			lines.push(String::new());
		}
		lines.push("Files:".to_string());
		lines.extend(
			tree.lines()
				.filter_map(non_empty_trimmed)
				.map(str::to_string),
		);
	}
	(!lines.is_empty()).then(|| lines.join("\n"))
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
	let value = value.trim();
	(!value.is_empty()).then_some(value)
}

fn format_attachment_size(size: u64) -> String {
	const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
	let mut value = size as f64;
	let mut unit = UNITS[0];
	for next_unit in UNITS.iter().skip(1) {
		if value < 1024.0 {
			break;
		}
		value /= 1024.0;
		unit = next_unit;
	}
	if unit == "B" {
		format!("{size} B")
	} else if value >= 10.0 {
		format!("{value:.0} {unit}")
	} else {
		format!("{value:.1} {unit}")
	}
}

fn is_font(resource: &Resource, mime: &str) -> bool {
	matches!(
		mime.to_ascii_lowercase().as_str(),
		"font/ttf"
			| "font/otf"
			| "font/woff"
			| "font/woff2"
			| "application/font-sfnt"
			| "application/font-woff"
			| "application/x-font-ttf"
			| "application/x-font-otf"
			| "application/x-font-woff"
			| "application/vnd.ms-fontobject"
	) || matches!(
		file_extension(&resource.file_name).as_deref(),
		Some("ttf" | "otf" | "woff" | "woff2" | "eot")
	)
}

fn is_pdf(resource: &Resource, mime: &str) -> bool {
	mime.eq_ignore_ascii_case("application/pdf")
		|| file_extension(&resource.file_name).as_deref() == Some("pdf")
}

fn is_swf(resource: &Resource, mime: &str) -> bool {
	matches!(
		mime.to_ascii_lowercase().as_str(),
		"application/x-shockwave-flash" | "application/vnd.adobe.flash.movie"
	) || file_extension(&resource.file_name).as_deref() == Some("swf")
}

fn is_midi(resource: &Resource, mime: &str) -> bool {
	matches!(
		mime.to_ascii_lowercase().as_str(),
		"audio/midi" | "audio/x-midi" | "audio/mid" | "audio/x-mid" | "application/x-midi"
	) || matches!(
		file_extension(&resource.file_name).as_deref(),
		Some("mid" | "midi" | "kar")
	)
}

fn is_epub(resource: &Resource, mime: &str) -> bool {
	mime.eq_ignore_ascii_case("application/epub+zip")
		|| file_extension(&resource.file_name).as_deref() == Some("epub")
}

fn comic_book_kind(resource: &Resource, mime: &str) -> Option<&'static str> {
	let extension = file_extension(&resource.file_name)?;
	match extension.as_str() {
		"cbz" => Some("cbz"),
		"cbr" => Some("cbr"),
		_ if mime.eq_ignore_ascii_case("application/vnd.comicbook+zip") => Some("cbz"),
		_ if mime.eq_ignore_ascii_case("application/vnd.comicbook-rar") => Some("cbr"),
		_ => None,
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
	encode_text(text)
		.replace('`', "&#96;")
		.replace('\n', "&#10;")
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
			original_file_name: None,
			mime: "audio/mpeg".into(),
			s3_key: None,
			text_preview: None,
			archive_tree: None,
			size_bytes: None,
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
	fn converts_evernote_rich_todos_toggles_and_callouts() {
		let body = enml_to_zola_body(
			r#"<en-note><ul style="--en-todo:true"><li style="--en-checked:false"><div>This in unckecked</div></li><li style="--en-checked:true"><div>This is checked</div></li></ul><div style="--en-toggle:true; --en-isCollapsed:false;--en-requiredFeatures:&quot;[&bsol;&quot;toggle&bsol;&quot;]&quot;"><div style="--en-toggleSummary:true">This is the name of my toggle</div><div style="--en-toggleContent:true"><div>This is inside my toggle</div></div></div><div style="--en-callout:true; --en-emoji:💡;--en-requiredFeatures:&quot;[&bsol;&quot;callout&bsol;&quot;]&quot;"><div>This is my callout example</div></div></en-note>"#,
			&[],
			&HashMap::new(),
		);

		assert!(body.contains(r#"<ul class="evernote-todo-list">"#));
		assert!(
			body.contains(r#"<input type="checkbox" disabled> <span>This in unckecked</span>"#)
		);
		assert!(
			body.contains(
				r#"<input type="checkbox" checked disabled> <span>This is checked</span>"#
			)
		);
		assert!(body.contains(r#"<details class="evernote-toggle" open>"#));
		assert!(body.contains("<summary>This is the name of my toggle</summary>"));
		assert!(
			body.contains(
				r#"<div class="evernote-toggle__content"><div>This is inside my toggle</div></div>"#
			),
			"{body}"
		);
		assert!(body.contains(r#"<aside class="evernote-callout">"#));
		assert!(
			body.contains(r#"<span class="evernote-callout__emoji" aria-hidden="true">💡</span>"#)
		);
		assert!(
			body.contains(
				r#"<div class="evernote-callout__body">This is my callout example</div>"#
			)
		);
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
				original_file_name: None,
				mime: "text/markdown".into(),
				s3_key: None,
				text_preview: Some("# Notes\n\nHello".into()),
				archive_tree: None,
				size_bytes: None,
			},
			Resource {
				hash: "glb".into(),
				file_name: "shape.glb".into(),
				original_file_name: None,
				mime: "model/gltf-binary".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
				size_bytes: None,
			},
			Resource {
				hash: "stl".into(),
				file_name: "mesh.stl".into(),
				original_file_name: None,
				mime: "model/stl".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
				size_bytes: None,
			},
			Resource {
				hash: "pdf".into(),
				file_name: "document.pdf".into(),
				original_file_name: None,
				mime: "application/pdf".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
				size_bytes: None,
			},
		];
		let body = enml_to_zola_body(
			r#"<en-note><en-media type="text/markdown" hash="txt"/><en-media type="model/gltf-binary" hash="glb"/><en-media type="model/stl" hash="stl"/><en-media type="application/pdf" hash="pdf"/></en-note>"#,
			&resources,
			&HashMap::new(),
		);

		assert!(body.contains(r#"<details class="attachment-preview attachment-preview-text">"#));
		assert!(body.contains("<pre># Notes&#10;&#10;Hello</pre>"));
		assert!(body.contains(r#"{{ model_viewer(src="shape.glb", alt="shape.glb") }}"#));
		assert!(body.contains(r#"{{ stl_viewer(src="mesh.stl", label="mesh.stl") }}"#));
		assert!(body.contains(r#"<details class="attachment-preview attachment-preview-pdf">"#));
		assert!(body.contains(
			r#"<iframe src="document.pdf" title="document.pdf" loading="lazy"></iframe>"#
		));
	}

	#[test]
	fn previews_archive_trees_subtitles_and_extra_3d_formats() {
		let resources = vec![
			Resource {
				hash: "zip".into(),
				file_name: "archive.zip".into(),
				original_file_name: None,
				mime: "application/zip".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: Some(".\n`-- docs\n    `-- readme.txt".into()),
				size_bytes: None,
			},
			Resource {
				hash: "sub".into(),
				file_name: "movie.srt".into(),
				original_file_name: None,
				mime: "application/x-subrip".into(),
				s3_key: None,
				text_preview: Some("1\n00:00:00,000 --> 00:00:02,000\nHello".into()),
				archive_tree: None,
				size_bytes: None,
			},
			Resource {
				hash: "obj".into(),
				file_name: "mesh.obj".into(),
				original_file_name: None,
				mime: "model/obj".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
				size_bytes: None,
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
		assert!(body.contains("&#96;-- docs"));
		assert!(body.contains("&#96;-- readme.txt"));
		assert!(body.contains("movie.srt"));
		assert!(body.contains("00:00:00,000 --&gt; 00:00:02,000"));
		assert!(
			body.contains(
				r#"{{ three_model_viewer(src="mesh.obj", kind="obj", label="mesh.obj") }}"#
			)
		);
	}

	#[test]
	fn renders_swf_with_ruffle_shortcode() {
		let resources = vec![Resource {
			hash: "flash".into(),
			file_name: "animation.swf".into(),
			original_file_name: None,
			mime: "application/x-shockwave-flash".into(),
			s3_key: None,
			text_preview: None,
			archive_tree: None,
			size_bytes: None,
		}];
		let body = enml_to_zola_body(
			r#"<en-note><en-media type="application/x-shockwave-flash" hash="flash"/></en-note>"#,
			&resources,
			&HashMap::new(),
		);

		assert!(body.contains(r#"{{ ruffle(src="animation.swf", label="animation.swf") }}"#));
	}

	#[test]
	fn renders_midi_books_comics_and_fonts_with_viewer_shortcodes() {
		let resources = vec![
			Resource {
				hash: "midi".into(),
				file_name: "song.mid".into(),
				original_file_name: None,
				mime: "audio/midi".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
				size_bytes: None,
			},
			Resource {
				hash: "epub".into(),
				file_name: "book.epub".into(),
				original_file_name: None,
				mime: "application/epub+zip".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
				size_bytes: None,
			},
			Resource {
				hash: "cbz".into(),
				file_name: "comic.cbz".into(),
				original_file_name: None,
				mime: "application/vnd.comicbook+zip".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
				size_bytes: None,
			},
			Resource {
				hash: "font".into(),
				file_name: "letters.woff2".into(),
				original_file_name: None,
				mime: "font/woff2".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
				size_bytes: None,
			},
		];
		let body = enml_to_zola_body(
			r#"<en-note><en-media type="audio/midi" hash="midi"/><en-media type="application/epub+zip" hash="epub"/><en-media type="application/vnd.comicbook+zip" hash="cbz"/><en-media type="font/woff2" hash="font"/></en-note>"#,
			&resources,
			&HashMap::new(),
		);

		assert!(body.contains(r#"{{ midi_player(src="song.mid", label="song.mid") }}"#));
		assert!(body.contains(r#"{{ epub_viewer(src="book.epub", label="book.epub") }}"#));
		assert!(
			body.contains(r#"{{ comic_viewer(src="comic.cbz", kind="cbz", label="comic.cbz") }}"#)
		);
		assert!(body.contains(
			r#"{{ font_preview(src="letters.woff2", label="letters.woff2", family="everpublich-font-font") }}"#
		));
	}

	#[test]
	fn renders_generated_preview_with_original_download() {
		let resources = vec![Resource {
			hash: "ai".into(),
			file_name: "poster.avif".into(),
			original_file_name: Some("poster.ai".into()),
			mime: "image/avif".into(),
			s3_key: None,
			text_preview: None,
			archive_tree: None,
			size_bytes: None,
		}];
		let body = enml_to_zola_body(
			r#"<en-note><en-media type="application/illustrator" hash="ai"/></en-note>"#,
			&resources,
			&HashMap::new(),
		);

		assert!(body.contains(r#"<img src="poster.avif" alt="" loading="lazy">"#));
		assert!(body.contains(r#"href="poster.ai" download"#));
		assert!(body.contains("Download original poster.ai"));
	}

	#[test]
	fn can_disable_generated_attachment_preview_by_file() {
		let mut disabled = HashSet::new();
		disabled.insert("poster.ai".to_string());
		let resources = vec![Resource {
			hash: "ai".into(),
			file_name: "poster.avif".into(),
			original_file_name: Some("poster.ai".into()),
			mime: "image/avif".into(),
			s3_key: None,
			text_preview: None,
			archive_tree: None,
			size_bytes: None,
		}];
		let body = enml_to_zola_body_with_options(
			r#"<en-note><en-media type="application/illustrator" hash="ai"/></en-note>"#,
			&resources,
			&HashMap::new(),
			&EnmlRenderOptions {
				previews_enabled: true,
				disabled_preview_files: &disabled,
			},
		);

		assert_eq!(
			body,
			r#"<a class="attachment" href="poster.ai" download>poster.ai</a>"#
		);
	}

	#[test]
	fn adds_attachment_size_to_download_title() {
		let resources = vec![Resource {
			hash: "exe".into(),
			file_name: "setup.exe".into(),
			original_file_name: None,
			mime: "application/x-msdownload".into(),
			s3_key: None,
			text_preview: None,
			archive_tree: None,
			size_bytes: Some(1_572_864),
		}];
		let body = enml_to_zola_body(
			r#"<en-note><en-media type="application/x-msdownload" hash="exe"/></en-note>"#,
			&resources,
			&HashMap::new(),
		);

		assert_eq!(
			body,
			r#"<a class="attachment" href="setup.exe" download title="Size: 1.5 MiB">setup.exe</a>"#
		);
	}

	#[test]
	fn adds_archive_size_and_file_tree_to_title() {
		let resources = vec![Resource {
			hash: "zip".into(),
			file_name: "archive.zip".into(),
			original_file_name: None,
			mime: "application/zip".into(),
			s3_key: None,
			text_preview: None,
			archive_tree: Some(".\n`-- docs\n    `-- readme.txt".into()),
			size_bytes: Some(2_048),
		}];
		let body = enml_to_zola_body(
			r#"<en-note><en-media type="application/zip" hash="zip"/></en-note>"#,
			&resources,
			&HashMap::new(),
		);

		assert!(body.contains("title=\"Size: 2.0 KiB"));
		assert!(body.contains("Files:"));
		assert!(body.contains("&#96;-- docs"));
		assert!(body.contains("&#96;-- readme.txt"));
		assert!(body.contains(r#"<summary title=""#));
	}
}
