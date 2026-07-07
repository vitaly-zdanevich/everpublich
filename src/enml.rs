//! Evernote ENML conversion.
//!
//! The MVP preserves Evernote formatting as HTML instead of trying to downgrade
//! fonts, sizes, colors, and tables to Markdown. Zola allows raw HTML in page
//! bodies, so this keeps rich notes faithful while still letting us rewrite
//! Evernote-specific tags and internal links.

use crate::models::Resource;
use chrono::{TimeZone, Utc};
use html_escape::{decode_html_entities, encode_double_quoted_attribute, encode_text};
use regex::{Captures, Regex};
use serde_json::Value;
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
	out = preserve_adjacent_inline_spacing(&out);

	rewrite_internal_links(&out, note_slug_by_guid)
}

fn preserve_adjacent_inline_spacing(html: &str) -> String {
	let adjacent_inline_words = Regex::new(
		r#"(?is)([\p{L}\p{N}])</(b|code|em|i|s|span|strong|u)>\s+<(b|code|em|i|s|span|strong|u)([^>]*)>([\p{L}\p{N}])"#,
	)
	.unwrap();
	adjacent_inline_words
		.replace_all(html, "$1</$2>&nbsp;<$3$4>$5")
		.into_owned()
}

/// Convert Evernote desktop rich blocks that are stored as CSS custom
/// properties into semantic HTML understood by browsers.
fn normalize_evernote_rich_blocks(html: &str) -> String {
	let out = convert_evernote_todo_lists(html);
	let out = convert_evernote_toggles(&out);
	let out = convert_evernote_callouts(&out);
	convert_evernote_comments(&out)
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

/// Render Evernote's current comments metadata, which is stored as JSON inside
/// a CSS custom property on an otherwise empty element.
fn convert_evernote_comments(html: &str) -> String {
	let mut out = html.to_string();
	let mut threads = Vec::new();
	for tag in ["div", "span"] {
		let comment_marker = Regex::new(&format!(
			r#"(?is)<{tag}\b(?P<attrs>[^>]*--en-threads\s*:[^>]*)>\s*(?:<br\s*/?>)?\s*</{tag}>"#
		))
		.unwrap();
		out = comment_marker
			.replace_all(&out, |caps: &Captures| {
				let attrs = caps.name("attrs").map(|m| m.as_str()).unwrap_or_default();
				let Some(found) = evernote_threads_from_attrs(attrs) else {
					return caps.get(0).unwrap().as_str().to_string();
				};
				threads.extend(found);
				String::new()
			})
			.into_owned();
	}
	attach_evernote_comments(&out, &threads)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EvernoteThread {
	comments: Vec<EvernoteComment>,
	ranges: Vec<EvernoteRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EvernoteRange {
	from: usize,
	to: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EvernoteComment {
	content: String,
	author: Option<String>,
	created_at_millis: Option<i64>,
	edited: bool,
}

fn evernote_threads_from_attrs(attrs: &str) -> Option<Vec<EvernoteThread>> {
	let style = style_attr(attrs)?;
	let raw = evernote_style_raw_value(&style, "--en-threads")?;
	let decoded = decode_evernote_style_json(&raw)?;
	let value: Value = serde_json::from_str(&decoded).ok()?;
	let threads = parse_evernote_threads(&value);
	threads
		.iter()
		.any(|thread| !thread.comments.is_empty())
		.then_some(threads)
}

fn evernote_style_raw_value(style: &str, property: &str) -> Option<String> {
	let lower = style.to_ascii_lowercase();
	let start = lower.find(&property.to_ascii_lowercase())?;
	let after_property = &style[start + property.len()..];
	let colon = after_property.find(':')?;
	let after_colon = after_property[colon + 1..].trim_start();
	let next_property = Regex::new(r#"(?i);\s*--en-[a-z0-9-]+\s*:"#).unwrap();
	let end = next_property
		.find(after_colon)
		.map(|matched| matched.start())
		.unwrap_or(after_colon.len());
	Some(after_colon[..end].trim().to_string())
}

fn decode_evernote_style_json(raw: &str) -> Option<String> {
	let with_backslashes = raw.replace("&bsol;", "\\");
	let decoded = decode_html_entities(&with_backslashes);
	let decoded = decoded.trim();
	if decoded.starts_with('"') {
		serde_json::from_str::<String>(decoded).ok()
	} else {
		Some(decoded.to_string())
	}
}

fn parse_evernote_threads(value: &Value) -> Vec<EvernoteThread> {
	let Some(threads) = value.as_array() else {
		return Vec::new();
	};
	threads
		.iter()
		.filter_map(|thread| {
			let comments = thread
				.get("comments")
				.and_then(Value::as_array)
				.into_iter()
				.flatten()
				.filter_map(parse_evernote_comment)
				.collect::<Vec<_>>();
			let ranges = thread
				.get("ranges")
				.and_then(Value::as_array)
				.into_iter()
				.flatten()
				.filter_map(parse_evernote_range)
				.collect::<Vec<_>>();
			(!comments.is_empty()).then_some(EvernoteThread { comments, ranges })
		})
		.collect()
}

fn parse_evernote_range(value: &Value) -> Option<EvernoteRange> {
	let from = json_usize(value, &["from", "start"])?;
	let to = json_usize(value, &["to", "end"])?;
	(to > from).then_some(EvernoteRange { from, to })
}

fn parse_evernote_comment(value: &Value) -> Option<EvernoteComment> {
	let content = json_string(value, &["content", "body", "text"])?;
	let author = json_string(
		value,
		&[
			"authorName",
			"creatorName",
			"createdBy",
			"userName",
			"author",
		],
	);
	Some(EvernoteComment {
		content,
		author,
		created_at_millis: json_i64(value, &["createdAt", "created", "created_at"]),
		edited: value
			.get("hasBeenEdited")
			.or_else(|| value.get("edited"))
			.and_then(Value::as_bool)
			.unwrap_or(false),
	})
}

fn json_string(value: &Value, keys: &[&str]) -> Option<String> {
	keys.iter()
		.find_map(|key| value.get(*key).and_then(Value::as_str))
		.map(str::trim)
		.filter(|value| !value.is_empty())
		.map(str::to_string)
}

fn json_i64(value: &Value, keys: &[&str]) -> Option<i64> {
	keys.iter()
		.find_map(|key| value.get(*key).and_then(Value::as_i64))
}

fn json_usize(value: &Value, keys: &[&str]) -> Option<usize> {
	keys.iter()
		.find_map(|key| value.get(*key).and_then(Value::as_u64))
		.and_then(|value| usize::try_from(value).ok())
}

fn attach_evernote_comments(html: &str, threads: &[EvernoteThread]) -> String {
	let mut placements = threads
		.iter()
		.enumerate()
		.filter_map(|(index, thread)| {
			let range = thread.ranges.first()?;
			let (start, end) = evernote_comment_range_bytes(html, *range)?;
			(end > start).then_some((start, end, index))
		})
		.collect::<Vec<_>>();
	placements.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));

	let mut out = html.to_string();
	let mut placed = HashSet::new();
	let mut cursor = 0;
	while cursor < placements.len() {
		let (start, end, _) = placements[cursor];
		let mut group = Vec::new();
		while cursor < placements.len()
			&& placements[cursor].0 == start
			&& placements[cursor].1 == end
		{
			let index = placements[cursor].2;
			group.push(threads[index].clone());
			placed.insert(index);
			cursor += 1;
		}
		let comment = render_evernote_comments(&group);
		let selected = out[start..end].to_string();
		if selected.contains('<') {
			let insertion = evernote_comment_insertion_point(&out, end);
			out.insert_str(insertion, &comment);
		} else {
			let target = format!(
				r#"<span class="evernote-comment-target">{}</span>"#,
				selected
			);
			out.replace_range(start..end, &target);
			let insertion = evernote_comment_insertion_point(&out, start + target.len());
			out.insert_str(insertion, &comment);
		}
	}

	let fallback = threads
		.iter()
		.enumerate()
		.filter(|(index, _)| !placed.contains(index))
		.map(|(_, thread)| thread)
		.cloned()
		.collect::<Vec<_>>();
	if !fallback.is_empty() {
		out.push_str(&render_evernote_comments(&fallback));
	}
	out
}

fn evernote_comment_range_bytes(html: &str, range: EvernoteRange) -> Option<(usize, usize)> {
	let mut best = None;
	let mut fallback = None;
	for profile in [
		OffsetProfile::Default,
		OffsetProfile::TableCompact,
		OffsetProfile::TableRich,
	] {
		let Some(start) =
			html_byte_for_evernote_offset(html, range.from, OffsetBoundary::Start, profile)
		else {
			continue;
		};
		let Some(end) = html_byte_for_evernote_offset(html, range.to, OffsetBoundary::End, profile)
		else {
			continue;
		};
		if end <= start {
			continue;
		}
		if let Some(score) = evernote_comment_selection_score(html, start, end) {
			if best.is_none_or(|(_, _, best_score)| score > best_score) {
				best = Some((start, end, score));
			}
			continue;
		}
		fallback.get_or_insert((start, end));
	}
	if let Some((start, end, score)) = best {
		if score < 15
			&& let Some(word) = nearby_whole_word_selection(html, start, end, range.to - range.from)
		{
			return Some(word);
		}
		return Some((start, end));
	}
	fallback
}

fn evernote_comment_selection_score(html: &str, start: usize, end: usize) -> Option<i32> {
	let selection = &html[start..end];
	if selection.contains('<') {
		return None;
	}
	let decoded = decode_html_entities(selection);
	let trimmed = decoded.trim();
	if trimmed.is_empty() {
		return None;
	}

	let mut score = 10;
	if trimmed.len() == decoded.len() {
		score += 2;
	} else {
		score -= 2;
	}
	if decoded.chars().any(char::is_whitespace) {
		score -= 2;
	}
	let first = trimmed.chars().next()?;
	let last = trimmed.chars().next_back()?;
	if first.is_alphanumeric()
		&& previous_visible_char(html, start).is_some_and(char::is_alphanumeric)
	{
		score -= 5;
	} else {
		score += 3;
	}
	if last.is_alphanumeric() && next_visible_char(html, end).is_some_and(char::is_alphanumeric) {
		score -= 5;
	} else {
		score += 3;
	}
	Some(score)
}

fn previous_visible_char(html: &str, index: usize) -> Option<char> {
	let mut end = index;
	while end > 0 {
		let before = &html[..end];
		let character = before.chars().next_back()?;
		if character == '>' {
			end = before.rfind('<')?;
			continue;
		}
		if character == ';'
			&& let Some(entity_start) = before.rfind('&')
			&& end - entity_start <= 64
		{
			let decoded = decode_html_entities(&html[entity_start..end]);
			return decoded.chars().next_back();
		}
		return Some(character);
	}
	None
}

fn next_visible_char(html: &str, index: usize) -> Option<char> {
	let mut start = index;
	while start < html.len() {
		if html[start..].starts_with('<') {
			start = html[start..]
				.find('>')
				.map(|position| start + position + 1)?;
			continue;
		}
		if html[start..].starts_with('&')
			&& let Some(semicolon) = html[start..].find(';').filter(|position| *position <= 64)
		{
			let end = start + semicolon + 1;
			let decoded = decode_html_entities(&html[start..end]);
			return decoded.chars().next();
		}
		return html[start..].chars().next();
	}
	None
}

fn nearby_whole_word_selection(
	html: &str,
	start: usize,
	end: usize,
	target_chars: usize,
) -> Option<(usize, usize)> {
	let center = start + (end - start) / 2;
	let mut best = None;
	let mut current = None::<VisibleWord>;
	let mut index = 0usize;
	while index < html.len() {
		if html[index..].starts_with('<') {
			finish_visible_word(&mut current, &mut best, center, target_chars);
			index = html[index..]
				.find('>')
				.map(|position| index + position + 1)?;
			continue;
		}

		let (next_index, character) = if html[index..].starts_with('&') {
			if let Some(semicolon) = html[index..].find(';').filter(|position| *position <= 64) {
				let next_index = index + semicolon + 1;
				let decoded = decode_html_entities(&html[index..next_index]);
				(next_index, decoded.chars().next().unwrap_or(' '))
			} else {
				let character = html[index..].chars().next()?;
				(index + character.len_utf8(), character)
			}
		} else {
			let character = html[index..].chars().next()?;
			(index + character.len_utf8(), character)
		};

		if character.is_alphanumeric() {
			match &mut current {
				Some(word) => {
					word.end = next_index;
					word.characters += 1;
				}
				None => {
					current = Some(VisibleWord {
						start: index,
						end: next_index,
						characters: 1,
					});
				}
			}
		} else {
			finish_visible_word(&mut current, &mut best, center, target_chars);
		}
		index = next_index;
	}
	finish_visible_word(&mut current, &mut best, center, target_chars);
	best.map(|candidate: WordCandidate| (candidate.start, candidate.end))
}

#[derive(Debug, Clone, Copy)]
struct VisibleWord {
	start: usize,
	end: usize,
	characters: usize,
}

#[derive(Debug, Clone, Copy)]
struct WordCandidate {
	start: usize,
	end: usize,
	distance: usize,
}

fn finish_visible_word(
	current: &mut Option<VisibleWord>,
	best: &mut Option<WordCandidate>,
	center: usize,
	target_chars: usize,
) {
	let Some(word) = current.take() else {
		return;
	};
	if word.characters != target_chars {
		return;
	}
	let word_center = word.start + (word.end - word.start) / 2;
	let distance = word_center.abs_diff(center);
	if distance > 160 {
		return;
	}
	if best.is_none_or(|candidate| distance < candidate.distance) {
		*best = Some(WordCandidate {
			start: word.start,
			end: word.end,
			distance,
		});
	}
}

fn evernote_comment_insertion_point(html: &str, from: usize) -> usize {
	let block_end =
		Regex::new(r#"(?is)</(?:blockquote|div|h1|h2|h3|h4|h5|h6|li|p|pre|td|th|tr)>"#).unwrap();
	block_end
		.find(&html[from..])
		.map(|matched| from + matched.end())
		.unwrap_or(html.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OffsetBoundary {
	Start,
	End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OffsetProfile {
	Default,
	TableCompact,
	TableRich,
}

impl OffsetProfile {
	fn hr_units(self) -> usize {
		match self {
			Self::Default | Self::TableCompact => 0,
			Self::TableRich => 2,
		}
	}

	fn table_cell_units(self) -> usize {
		match self {
			Self::Default => 0,
			Self::TableCompact | Self::TableRich => 3,
		}
	}

	fn table_units(self) -> usize {
		match self {
			Self::Default => 0,
			Self::TableCompact | Self::TableRich => 3,
		}
	}

	fn table_section_units(self) -> usize {
		match self {
			Self::Default | Self::TableCompact => 0,
			Self::TableRich => 2,
		}
	}

	fn counts_table_structure(self) -> bool {
		!matches!(self, Self::Default)
	}
}

/// Map Evernote comment offsets to byte positions in the rendered HTML.
///
/// Current Evernote comments use rich-text document offsets where normal
/// characters count as one unit and block separators behave like CRLF, taking
/// two units. The source HTML only has tags, so this scanner recreates that
/// lightweight text model without stripping the markup we need to preserve.
fn html_byte_for_evernote_offset(
	html: &str,
	target: usize,
	boundary: OffsetBoundary,
	profile: OffsetProfile,
) -> Option<usize> {
	let mut offset = 0usize;
	let mut index = 0usize;
	let mut pending_br_line_break = false;
	while index < html.len() {
		if html[index..].starts_with('<') {
			let end = html[index..]
				.find('>')
				.map(|position| index + position + 1)?;
			let tag = &html[index + 1..end - 1];
			if let Some(block_end) = everpublich_source_url_block_end(html, index, tag) {
				index = block_end;
				continue;
			}
			let mut increment = evernote_tag_offset_units(tag, profile);
			if pending_br_line_break && evernote_is_offset_block_closer(tag, profile) {
				increment = 0;
				pending_br_line_break = false;
			} else if evernote_is_br_tag(tag) {
				pending_br_line_break = true;
			}
			if increment == 0 {
				index = end;
				continue;
			}
			if !evernote_is_br_tag(tag) {
				pending_br_line_break = false;
			}
			match boundary {
				OffsetBoundary::Start if target <= offset => return Some(index),
				OffsetBoundary::End if target <= offset => return Some(index),
				OffsetBoundary::Start if target < offset + increment => return Some(index),
				OffsetBoundary::End if target <= offset + increment => return Some(end),
				_ => {}
			}
			offset += increment;
			index = end;
			continue;
		}

		let (next_index, increment) = html_text_token(html, index)?;
		pending_br_line_break = false;
		match boundary {
			OffsetBoundary::Start if target <= offset => return Some(index),
			OffsetBoundary::End if target <= offset => return Some(index),
			OffsetBoundary::Start if target < offset + increment => return Some(index),
			OffsetBoundary::End if target <= offset + increment => return Some(next_index),
			_ => {}
		}
		offset += increment;
		index = next_index;
	}
	(target <= offset).then_some(html.len())
}

fn everpublich_source_url_block_end(html: &str, index: usize, tag: &str) -> Option<usize> {
	let (name, closing) = evernote_tag_name(tag)?;
	if closing || name != "p" || !tag.contains("data-everpublich-source-url") {
		return None;
	}
	html[index..]
		.to_ascii_lowercase()
		.find("</p>")
		.map(|position| index + position + "</p>".len())
}

fn evernote_tag_offset_units(tag: &str, profile: OffsetProfile) -> usize {
	let Some((name, closing)) = evernote_tag_name(tag) else {
		return 0;
	};
	if name == "br" {
		return 2;
	}
	if name == "hr" {
		return profile.hr_units();
	}
	if !closing {
		return 0;
	}
	match name.as_str() {
		"blockquote" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "p" | "pre"
		| "tr" => 2,
		"td" | "th" => profile.table_cell_units(),
		"table" => profile.table_units(),
		"tbody" | "thead" | "tfoot" => profile.table_section_units(),
		_ => 0,
	}
}

fn evernote_is_br_tag(tag: &str) -> bool {
	evernote_tag_name(tag).is_some_and(|(name, _)| name == "br")
}

fn evernote_is_closing_block_tag(tag: &str) -> bool {
	evernote_tag_name(tag).is_some_and(|(name, closing)| {
		closing
			&& matches!(
				name.as_str(),
				"blockquote"
					| "div" | "h1" | "h2"
					| "h3" | "h4" | "h5"
					| "h6" | "li" | "p"
					| "pre" | "tr"
			)
	})
}

fn evernote_is_offset_block_closer(tag: &str, profile: OffsetProfile) -> bool {
	evernote_tag_name(tag).is_some_and(|(name, closing)| {
		closing
			&& (evernote_is_closing_block_tag(tag)
				|| profile.counts_table_structure()
					&& matches!(
						name.as_str(),
						"td" | "th" | "table" | "tbody" | "thead" | "tfoot"
					))
	})
}

fn evernote_tag_name(tag: &str) -> Option<(String, bool)> {
	let trimmed = tag.trim_start();
	if trimmed.starts_with("!--") {
		return None;
	}
	let closing = trimmed.starts_with('/');
	let name_start = usize::from(closing);
	let name = trimmed[name_start..]
		.split(|character: char| {
			character.is_ascii_whitespace() || character == '/' || character == '>'
		})
		.next()
		.unwrap_or_default()
		.to_ascii_lowercase();
	(!name.is_empty()).then_some((name, closing))
}

fn html_text_token(html: &str, index: usize) -> Option<(usize, usize)> {
	let rest = &html[index..];
	if rest.starts_with('&')
		&& let Some(semicolon) = rest.find(';').filter(|position| *position <= 64)
	{
		let end = index + semicolon + 1;
		let decoded = decode_html_entities(&html[index..end]);
		let units = decoded.chars().count().max(1);
		return Some((end, units));
	}
	let character = rest.chars().next()?;
	Some((index + character.len_utf8(), 1))
}

fn render_evernote_comments(threads: &[EvernoteThread]) -> String {
	let mut out = String::from(
		r#"<aside class="evernote-comments" aria-label="Evernote comments"><p class="evernote-comments__title">Comment</p><ol>"#,
	);
	for thread in threads {
		for comment in &thread.comments {
			out.push_str(r#"<li class="evernote-comments__item">"#);
			out.push_str(&format!(
				r#"<div class="evernote-comments__body">{}</div>"#,
				comment_html(&comment.content)
			));
			let meta = comment_metadata(comment);
			if !meta.is_empty() {
				out.push_str(&format!(
					r#"<p class="evernote-comments__meta">{}</p>"#,
					meta.join(" · ")
				));
			}
			out.push_str("</li>");
		}
	}
	out.push_str("</ol></aside>");
	out
}

fn comment_html(content: &str) -> String {
	let mut out = String::new();
	for (index, line) in content.lines().enumerate() {
		if index > 0 {
			out.push_str("<br>");
		}
		out.push_str(&encode_text(line));
	}
	if out.is_empty() {
		encode_text(content).to_string()
	} else {
		out
	}
}

fn comment_metadata(comment: &EvernoteComment) -> Vec<String> {
	let mut parts = Vec::new();
	if let Some(author) = &comment.author {
		parts.push(encode_text(author).to_string());
	}
	if let Some(created_at) = comment.created_at_millis.and_then(format_comment_time) {
		parts.push(created_at);
	}
	if comment.edited {
		parts.push("edited".to_string());
	}
	parts
}

fn format_comment_time(millis: i64) -> Option<String> {
	Utc.timestamp_millis_opt(millis)
		.single()
		.map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
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
	fn renders_evernote_comments_metadata() {
		let style = r#"--en-threads:&quot;[{\&quot;comments\&quot;:[{\&quot;content\&quot;:\&quot;Hi, this is my comment\&quot;,\&quot;createdAt\&quot;:1783311195586,\&quot;hasBeenEdited\&quot;:true}],\&quot;ranges\&quot;:[{\&quot;from\&quot;:3,\&quot;to\&quot;:11}]}]&quot;;--en-requiredFeatures:&quot;[\&quot;evernoteComments\&quot;]&quot;;"#;
		let raw = evernote_style_raw_value(style, "--en-threads").unwrap();
		let decoded = decode_evernote_style_json(&raw).unwrap_or_else(|| panic!("{raw}"));
		assert!(decoded.contains(r#""comments""#), "{decoded}");

		let body = enml_to_zola_body(
			r#"<en-note><div style="--en-threads:&quot;[{\&quot;comments\&quot;:[{\&quot;content\&quot;:\&quot;Hi, this is my comment\&quot;,\&quot;createdAt\&quot;:1783311195586,\&quot;hasBeenEdited\&quot;:true}],\&quot;ranges\&quot;:[{\&quot;from\&quot;:3,\&quot;to\&quot;:11}]}]&quot;;--en-requiredFeatures:&quot;[\&quot;evernoteComments\&quot;]&quot;;"></div><div>A</div><div>comments.</div></en-note>"#,
			&[],
			&HashMap::new(),
		);

		assert!(
			body.contains(r#"<span class="evernote-comment-target">comments</span>"#),
			"{body}"
		);
		assert!(
			body.contains(
				r#"<div><span class="evernote-comment-target">comments</span>.</div><aside class="evernote-comments""#
			),
			"{body}"
		);
		assert!(
			body.contains(r#"<aside class="evernote-comments" aria-label="Evernote comments">"#),
			"{body}"
		);
		assert!(body.contains("Hi, this is my comment"));
		assert!(body.contains("UTC"), "{body}");
		assert!(body.contains("edited"));
		assert!(!body.contains("--en-threads"));
		let target_at = body.find("evernote-comment-target").unwrap();
		let comment_at = body.find("Hi, this is my comment").unwrap();
		assert!(comment_at > target_at);

		let body_with_source = enml_to_zola_body(
			r#"<en-note><p data-everpublich-source-url="true"><a href="https://example.com/source">https://example.com/source</a></p><div style="--en-threads:&quot;[{\&quot;comments\&quot;:[{\&quot;content\&quot;:\&quot;Hi, this is my comment\&quot;}],\&quot;ranges\&quot;:[{\&quot;from\&quot;:3,\&quot;to\&quot;:11}]}]&quot;;--en-requiredFeatures:&quot;[\&quot;evernoteComments\&quot;]&quot;;"></div><div>A</div><div>comments.</div></en-note>"#,
			&[],
			&HashMap::new(),
		);

		assert!(
			body_with_source.contains(r#"<span class="evernote-comment-target">comments</span>"#),
			"{body_with_source}"
		);
		assert!(
			body_with_source.contains(
				r#"<div><span class="evernote-comment-target">comments</span>.</div><aside class="evernote-comments""#
			),
			"{body_with_source}"
		);
	}

	#[test]
	fn maps_evernote_comment_offsets_near_real_rich_text_marker() {
		let body = enml_to_zola_body(
			r#"<en-note><p data-everpublich-source-url="true"><a href="https://archive.org/details/indiegamewebsite-com--dump">https://archive.org/details/indiegamewebsite-com--dump</a></p><div><br/></div><div><br/></div><div style="--en-threads:&quot;[{\&quot;comments\&quot;:[{\&quot;content\&quot;:\&quot;Hi, this is my comment\&quot;}],\&quot;ranges\&quot;:[{\&quot;from\&quot;:267,\&quot;to\&quot;:275}]}]&quot;;--en-requiredFeatures:&quot;[\&quot;evernoteComments\&quot;]&quot;;"></div><div><a href="https://genius.com/Bonnie-tyler-total-eclipse-of-the-heart-lyrics">https://genius.com/Bonnie-tyler-total-eclipse-of-the-heart-lyrics</a></div><div><br/></div><hr/><div><br/></div><table><tbody><tr><td><div>aa</div></td><td><div>bb</div></td></tr><tr><td><div>xx</div></td><td><div>yy</div></td></tr></tbody></table><div><br/></div><div><br/></div><div><span style="color:red">My</span> <span style="color:blue">colorful</span> <span style="color:green">text</span></div><div><span><span style="--en-markholder:true;"><br/></span></span></div><h2>My<span> h2 and green text</span></h2><div><br/></div><div><b>Bold</b> <i>italic </i><u>under</u><s> strike</s></div><div><s><span style="--en-markholder:true;"><br/></span></s></div><div>Part of this text is <span style="--en-highlight:yellow;background-color: #fdf3d0;">highlighted</span>, should be nice.</div><div><br/></div><div>On this like we are testing comments.</div><div><br/></div><div><br/></div><div><br/></div><div>Quote:</div></en-note>"#,
			&[],
			&HashMap::new(),
		);

		assert!(
			body.contains(r#"<span class="evernote-comment-target">comments</span>"#),
			"{body}"
		);
		assert!(
			body.contains(r#"<div>On this like we are testing <span class="evernote-comment-target">comments</span>.</div><aside class="evernote-comments""#),
			"{body}"
		);
	}

	#[test]
	fn comment_offset_fallback_prefers_nearby_whole_word() {
		let html = "<div>On this like we are testing comments.</div>";
		let start = html.find("g comments").unwrap();
		let end = start + "g commen".len();
		let (word_start, word_end) = nearby_whole_word_selection(html, start, end, 8).unwrap();

		assert_eq!(&html[word_start..word_end], "comments");
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
