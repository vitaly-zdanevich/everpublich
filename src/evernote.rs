//! Evernote-specific mapping and OAuth helpers.
//!
//! Evernote's public documentation still describes OAuth 1.0a. This module only
//! builds signed OAuth URLs and transforms already-fetched notes. Network calls
//! stay outside this module so tests do not need Evernote credentials.

use crate::enml::{EnmlRenderOptions, enml_to_zola_body, enml_to_zola_body_with_options};
use crate::models::{Note, Post, PostKind};
use crate::slug::slug_from_title_and_tags;
use crate::widgets::{
	enrich_link_titles, expand_bare_links_with_disabled, link_wikidata_ids,
	normalize_widget_provider_name,
};
use anyhow::{Context, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use html_escape::decode_html_entities;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use regex::Regex;
use sha1::Sha1;
use std::collections::{BTreeMap, HashMap, HashSet};

type HmacSha1 = Hmac<Sha1>;

const OAUTH_ENDPOINT: &str = "https://www.evernote.com/oauth";
const AUTHORIZE_ENDPOINT: &str = "https://www.evernote.com/OAuth.action";

const OAUTH_ENCODE: &AsciiSet = &CONTROLS
	.add(b' ')
	.add(b'!')
	.add(b'"')
	.add(b'#')
	.add(b'$')
	.add(b'%')
	.add(b'&')
	.add(b'\'')
	.add(b'(')
	.add(b')')
	.add(b'*')
	.add(b'+')
	.add(b',')
	.add(b'/')
	.add(b':')
	.add(b';')
	.add(b'<')
	.add(b'=')
	.add(b'>')
	.add(b'?')
	.add(b'@')
	.add(b'[')
	.add(b'\\')
	.add(b']')
	.add(b'^')
	.add(b'`')
	.add(b'{')
	.add(b'|')
	.add(b'}');

#[derive(Debug, Clone, PartialEq, Eq)]
/// Consumer credentials issued by Evernote for OAuth 1.0a.
pub struct OAuthCredentials {
	/// Evernote OAuth consumer key.
	pub consumer_key: String,
	/// Evernote OAuth consumer secret.
	pub consumer_secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Request parameters that change for each temporary-token request.
pub struct OAuthTemporaryRequest {
	/// OAuth callback URL configured for the backend endpoint.
	pub callback_url: String,
	/// OAuth nonce for this request.
	pub nonce: String,
	/// Unix timestamp in seconds used in the OAuth signature.
	pub timestamp: i64,
}

/// Signed URL used to request an Evernote temporary token.
pub fn temporary_token_url(
	credentials: &OAuthCredentials,
	request: &OAuthTemporaryRequest,
) -> Result<String> {
	let mut params = BTreeMap::new();
	params.insert("oauth_callback", request.callback_url.as_str());
	params.insert("oauth_consumer_key", credentials.consumer_key.as_str());
	params.insert("oauth_nonce", request.nonce.as_str());
	params.insert("oauth_signature_method", "HMAC-SHA1");
	let timestamp = request.timestamp.to_string();
	params.insert("oauth_timestamp", timestamp.as_str());
	params.insert("oauth_version", "1.0");

	let signature = oauth_signature(
		"GET",
		OAUTH_ENDPOINT,
		&params,
		&credentials.consumer_secret,
		"",
	)?;
	Ok(format!(
		"{OAUTH_ENDPOINT}?{}&oauth_signature={}",
		normalized_params(&params),
		encode(&signature)
	))
}

/// Build the Evernote authorization URL for a temporary token.
pub fn authorization_url(temporary_token: &str) -> String {
	format!(
		"{AUTHORIZE_ENDPOINT}?oauth_token={}",
		encode(temporary_token)
	)
}

fn oauth_signature(
	method: &str,
	endpoint: &str,
	params: &BTreeMap<&str, &str>,
	consumer_secret: &str,
	token_secret: &str,
) -> Result<String> {
	let base = format!(
		"{}&{}&{}",
		method.to_ascii_uppercase(),
		encode(endpoint),
		encode(&normalized_params(params))
	);
	let key = format!("{}&{}", encode(consumer_secret), encode(token_secret));
	let mut mac = HmacSha1::new_from_slice(key.as_bytes()).context("invalid OAuth HMAC key")?;
	mac.update(base.as_bytes());
	Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

fn normalized_params(params: &BTreeMap<&str, &str>) -> String {
	params
		.iter()
		.map(|(k, v)| format!("{}={}", encode(k), encode(v)))
		.collect::<Vec<_>>()
		.join("&")
}

fn encode(value: &str) -> String {
	utf8_percent_encode(value, OAUTH_ENCODE).to_string()
}

/// Convert fetched Evernote notes to blog posts/pages.
pub fn notes_to_posts(notes: &[Note], expand_widgets: bool) -> Vec<Post> {
	let config = notebook_config(notes, expand_widgets);
	let linkable_notes = notes
		.iter()
		.filter(|note| post_kind(&note.title, &note.tags).is_linkable())
		.collect::<Vec<_>>();
	let note_slug_by_guid = unique_note_slug_map(&linkable_notes);

	notes
		.iter()
		.map(|note| {
			let slug = note_slug_by_guid
				.get(&note.guid.to_ascii_lowercase())
				.cloned()
				.unwrap_or_else(|| slug_from_title_and_tags(&note.title, &note.tags));
			note_to_post_with_slug(note, slug, &note_slug_by_guid, &config)
		})
		.collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NotebookConfig {
	expand_widgets: bool,
	previews_enabled: bool,
	disabled_preview_files: HashSet<String>,
	disabled_widget_providers: HashSet<String>,
}

fn notebook_config(notes: &[Note], expand_widgets: bool) -> NotebookConfig {
	let mut config = NotebookConfig {
		expand_widgets,
		previews_enabled: true,
		disabled_preview_files: HashSet::new(),
		disabled_widget_providers: HashSet::new(),
	};
	for note in notes.iter().filter(|note| is_config_note(&note.title)) {
		apply_config_note(&mut config, &note.enml);
	}
	config
}

fn apply_config_note(config: &mut NotebookConfig, enml: &str) {
	for line in plain_text_from_enml(enml).lines() {
		let Some((key, value)) = line.split_once(':') else {
			continue;
		};
		let key = key.trim();
		let value = value.trim();
		let key_lower = key.to_ascii_lowercase();
		if key.eq_ignore_ascii_case("widgets") {
			if let Some(enabled) = config_bool(value) {
				config.expand_widgets = enabled;
			}
		} else if key.eq_ignore_ascii_case("previews") {
			if let Some(enabled) = config_bool(value) {
				config.previews_enabled = enabled;
			}
		} else if key.eq_ignore_ascii_case("preview") {
			apply_named_config_bool(value, &mut config.disabled_preview_files, |file_name| {
				Some(file_name.trim().to_ascii_lowercase())
			});
		} else if key.eq_ignore_ascii_case("widget") {
			apply_named_config_bool(value, &mut config.disabled_widget_providers, |provider| {
				normalize_widget_provider_name(provider).map(str::to_string)
			});
		} else if let Some(file_name) = key_lower.strip_prefix("preview ") {
			if let Some(enabled) = config_bool(value) {
				set_config_item(
					&mut config.disabled_preview_files,
					file_name.trim().to_ascii_lowercase(),
					enabled,
				);
			}
		} else if let Some(provider) = key_lower.strip_prefix("widget ")
			&& let Some(enabled) = config_bool(value)
			&& let Some(provider) = normalize_widget_provider_name(provider)
		{
			set_config_item(
				&mut config.disabled_widget_providers,
				provider.to_string(),
				enabled,
			);
		}
	}
}

fn config_bool(value: &str) -> Option<bool> {
	match value.trim().to_ascii_lowercase().as_str() {
		"on" | "true" | "yes" | "enabled" => Some(true),
		"off" | "false" | "no" | "disabled" => Some(false),
		_ => None,
	}
}

fn apply_named_config_bool(
	value: &str,
	disabled: &mut HashSet<String>,
	normalize: impl Fn(&str) -> Option<String>,
) {
	let Some((name, enabled)) = split_named_config_bool(value) else {
		return;
	};
	if let Some(name) = normalize(name) {
		set_config_item(disabled, name, enabled);
	}
}

fn split_named_config_bool(value: &str) -> Option<(&str, bool)> {
	let value = value.trim();
	let (name, state) = value.rsplit_once(' ')?;
	config_bool(state).map(|enabled| (name.trim(), enabled))
}

fn set_config_item(disabled: &mut HashSet<String>, item: String, enabled: bool) {
	if item.is_empty() {
		return;
	}
	if enabled {
		disabled.remove(&item);
	} else {
		disabled.insert(item);
	}
}

/// Build the final URL slug for each note GUID, making duplicate title or
/// explicit `slug:` values unique before internal Evernote links are rewritten.
fn unique_note_slug_map(notes: &[&Note]) -> HashMap<String, String> {
	let mut seen_bases = HashSet::<String>::new();
	let mut used_slugs = HashSet::<String>::new();
	let mut slugs = HashMap::new();
	for note in notes {
		let base = note_base_slug(note);
		let candidate = if seen_bases.insert(base.clone()) {
			base.clone()
		} else {
			format!("{base}-{}", note.created.format("%Y%m%d-%H%M%S"))
		};
		let slug = unique_slug(candidate, &mut used_slugs);
		slugs.insert(note.guid.to_ascii_lowercase(), slug);
	}
	slugs
}

/// Return `candidate` unless it was already used; otherwise append a short
/// numeric suffix. This handles notes created in the same second.
fn unique_slug(candidate: String, used_slugs: &mut HashSet<String>) -> String {
	if used_slugs.insert(candidate.clone()) {
		return candidate;
	}
	for index in 2.. {
		let slug = format!("{candidate}-{index}");
		if used_slugs.insert(slug.clone()) {
			return slug;
		}
	}
	unreachable!("unbounded integer sequence must eventually produce a unique slug")
}

/// Convert one Evernote note to one generated post/page.
pub fn note_to_post(
	note: &Note,
	note_slug_by_guid: &HashMap<String, String>,
	expand_widgets: bool,
) -> Post {
	let slug = note_base_slug(note);
	let config = NotebookConfig {
		expand_widgets,
		previews_enabled: true,
		disabled_preview_files: HashSet::new(),
		disabled_widget_providers: HashSet::new(),
	};
	note_to_post_with_slug(note, slug, note_slug_by_guid, &config)
}

fn note_base_slug(note: &Note) -> String {
	if is_about_note(&note.title) {
		"about".to_string()
	} else if is_config_note(&note.title) {
		"everpublich-config".to_string()
	} else if let Some(tag) = everpublish_nav_tag(&note.title) {
		slug_from_title_and_tags(&tag, &[])
	} else if let Some(title) = everpublish_page_title(&note.title) {
		slug_from_title_and_tags(&title, &note.tags)
	} else {
		slug_from_title_and_tags(&note.title, &note.tags)
	}
}

fn note_to_post_with_slug(
	note: &Note,
	slug: String,
	note_slug_by_guid: &HashMap<String, String>,
	config: &NotebookConfig,
) -> Post {
	let kind = post_kind(&note.title, &note.tags);
	let enml_options = EnmlRenderOptions {
		previews_enabled: config.previews_enabled,
		disabled_preview_files: &config.disabled_preview_files,
	};
	let body = enml_to_zola_body_with_options(
		&note.enml,
		&note.resources,
		note_slug_by_guid,
		&enml_options,
	);
	let body = if kind == PostKind::Config {
		body
	} else {
		let body = expand_bare_links_with_disabled(
			&body,
			config.expand_widgets,
			&config.disabled_widget_providers,
		);
		let body = link_wikidata_ids(&body);
		enrich_link_titles(&body)
	};

	Post {
		guid: note.guid.clone(),
		slug,
		title: post_title(&note.title, kind),
		date: note.created,
		tags: if kind == PostKind::NavTag {
			Vec::new()
		} else {
			public_tags(&note.tags)
		},
		body,
		resources: note.resources.clone(),
		kind,
	}
}

fn post_kind(title: &str, tags: &[String]) -> PostKind {
	if is_config_note(title) {
		PostKind::Config
	} else if is_about_note(title) {
		PostKind::About
	} else if everpublish_nav_tag(title).is_some() {
		PostKind::NavTag
	} else if everpublish_page_title(title).is_some() || has_tag(tags, "page") {
		PostKind::Page
	} else {
		PostKind::BlogPost
	}
}

fn is_about_note(title: &str) -> bool {
	matches!(
		everpublish_page_title(title).as_deref(),
		Some(title) if title.eq_ignore_ascii_case("about")
	)
}

fn is_config_note(title: &str) -> bool {
	matches!(
		everpublish_command_title(title).as_deref(),
		Some(title) if title.eq_ignore_ascii_case("config")
	)
}

fn everpublish_nav_tag(title: &str) -> Option<String> {
	everpublish_command_title(title)
		.and_then(|title| title.strip_prefix('#').map(str::trim).map(clean_title))
		.filter(|tag| !tag.is_empty())
}

/// Return a dedicated page title from an Everpublich title command.
fn everpublish_page_title(title: &str) -> Option<String> {
	everpublish_command_title(title).filter(|title| !title.starts_with('#'))
}

fn everpublish_command_title(title: &str) -> Option<String> {
	let trimmed = title.trim();
	let lower = trimmed.to_ascii_lowercase();
	for prefix in ["everpublish:", "everpublich:"] {
		if lower.starts_with(prefix) {
			return trimmed
				.get(prefix.len()..)
				.map(str::trim)
				.filter(|title| !title.is_empty())
				.map(clean_title);
		}
	}
	None
}

fn public_tags(tags: &[String]) -> Vec<String> {
	tags.iter()
		.filter(|tag| {
			!tag.starts_with("slug:") && !matches!(tag.as_str(), "page" | "podcast_description")
		})
		.cloned()
		.collect()
}

fn has_tag(tags: &[String], wanted: &str) -> bool {
	tags.iter().any(|tag| tag.eq_ignore_ascii_case(wanted))
}

fn clean_title(title: &str) -> String {
	title.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn post_title(title: &str, kind: PostKind) -> String {
	if kind == PostKind::NavTag
		&& let Some(tag) = everpublish_nav_tag(title)
	{
		return format!("#{tag}");
	}
	everpublish_page_title(title)
		.map(|title| {
			if kind == PostKind::About {
				"About".to_string()
			} else {
				title
			}
		})
		.unwrap_or_else(|| clean_title(title))
}

fn plain_text_from_enml(enml: &str) -> String {
	let body = enml_to_zola_body(enml, &[], &HashMap::new());
	let with_line_breaks = Regex::new(
		r"(?i)</?(?:address|article|aside|blockquote|br|dd|div|dl|dt|figcaption|figure|footer|h[1-6]|header|hr|li|main|nav|ol|p|pre|section|table|tbody|td|tfoot|th|thead|tr|ul)\b[^>]*>",
	)
	.unwrap()
	.replace_all(&body, "\n");
	let without_tags = Regex::new(r"(?s)<[^>]+>")
		.unwrap()
		.replace_all(&with_line_breaks, " ");
	decode_html_entities(&without_tags)
		.lines()
		.map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
		.filter(|line| !line.is_empty())
		.collect::<Vec<_>>()
		.join("\n")
}

/// Helper for mock fixtures and tests.
/// Build a UTC timestamp from Unix seconds for fixtures and CLI mock data.
pub fn utc(seconds: i64) -> DateTime<Utc> {
	DateTime::from_timestamp(seconds, 0).expect("valid fixture timestamp")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::Resource;

	#[test]
	fn maps_slug_tag_and_page_kind() {
		let note = Note {
			guid: "abc".into(),
			title: "Ignored Title".into(),
			created: utc(1_700_000_000),
			updated: utc(1_700_000_000),
			tags: vec!["slug:my-post".into(), "page".into(), "rust".into()],
			enml: "<en-note>Hello</en-note>".into(),
			resources: vec![],
		};
		let post = notes_to_posts(&[note], true).remove(0);

		assert_eq!(post.slug, "my-post");
		assert_eq!(post.kind, PostKind::Page);
		assert_eq!(post.tags, vec!["rust"]);
	}

	#[test]
	fn maps_title_commands_to_about_page_pages_and_nav_tags() {
		let notes = vec![
			Note {
				guid: "about".into(),
				title: "everpublich:about".into(),
				created: utc(1_700_000_000),
				updated: utc(1_700_000_000),
				tags: vec![],
				enml: "<en-note>About body</en-note>".into(),
				resources: vec![],
			},
			Note {
				guid: "page".into(),
				title: "everpublish:Project status".into(),
				created: utc(1_700_000_001),
				updated: utc(1_700_000_001),
				tags: vec![],
				enml: "<en-note>Page body</en-note>".into(),
				resources: vec![],
			},
			Note {
				guid: "nav".into(),
				title: "everpublich:#postgres".into(),
				created: utc(1_700_000_002),
				updated: utc(1_700_000_002),
				tags: vec![],
				enml: "<en-note>PostgreSQL docs</en-note>".into(),
				resources: vec![],
			},
		];

		let posts = notes_to_posts(&notes, true);

		assert_eq!(posts[0].kind, PostKind::About);
		assert_eq!(posts[0].title, "About");
		assert_eq!(posts[0].slug, "about");
		assert_eq!(posts[1].kind, PostKind::Page);
		assert_eq!(posts[1].title, "Project status");
		assert_eq!(posts[1].slug, "project-status");
		assert_eq!(posts[2].kind, PostKind::NavTag);
		assert_eq!(posts[2].title, "#postgres");
		assert!(posts[2].tags.is_empty());
	}

	#[test]
	fn config_note_can_disable_widgets() {
		let notes = vec![
			Note {
				guid: "config".into(),
				title: "everpublich:config".into(),
				created: utc(1_700_000_000),
				updated: utc(1_700_000_000),
				tags: vec![],
				enml: "<en-note><p>widgets: off</p></en-note>".into(),
				resources: vec![],
			},
			Note {
				guid: "post".into(),
				title: "Post".into(),
				created: utc(1_700_000_001),
				updated: utc(1_700_000_001),
				tags: vec![],
				enml: "<en-note><p>https://youtu.be/dQw4w9WgXcQ</p></en-note>".into(),
				resources: vec![],
			},
		];

		let posts = notes_to_posts(&notes, true);

		assert_eq!(posts[0].kind, PostKind::Config);
		assert_eq!(posts[1].kind, PostKind::BlogPost);
		assert!(posts[1].body.contains("https://youtu.be/dQw4w9WgXcQ"));
		assert!(!posts[1].body.contains("youtube("));
	}

	#[test]
	fn config_note_can_disable_one_widget_provider() {
		let notes = vec![
			Note {
				guid: "config".into(),
				title: "everpublich:config".into(),
				created: utc(1_700_000_000),
				updated: utc(1_700_000_000),
				tags: vec![],
				enml: "<en-note><p>widget: youtube off</p></en-note>".into(),
				resources: vec![],
			},
			Note {
				guid: "post".into(),
				title: "Post".into(),
				created: utc(1_700_000_001),
				updated: utc(1_700_000_001),
				tags: vec![],
				enml: "<en-note><p>https://youtu.be/dQw4w9WgXcQ</p><p>https://open.spotify.com/track/0VjIjW4GlUZAMYd2vXMi3b</p></en-note>".into(),
				resources: vec![],
			},
		];

		let posts = notes_to_posts(&notes, true);

		assert!(posts[1].body.contains("https://youtu.be/dQw4w9WgXcQ"));
		assert!(!posts[1].body.contains("youtube("));
		assert!(posts[1].body.contains("spotify("));
	}

	#[test]
	fn config_note_can_disable_preview_by_file() {
		let notes = vec![
			Note {
				guid: "config".into(),
				title: "everpublich:config".into(),
				created: utc(1_700_000_000),
				updated: utc(1_700_000_000),
				tags: vec![],
				enml: "<en-note><p>preview: poster.psd off</p></en-note>".into(),
				resources: vec![],
			},
			Note {
				guid: "post".into(),
				title: "Post".into(),
				created: utc(1_700_000_001),
				updated: utc(1_700_000_001),
				tags: vec![],
				enml:
					r#"<en-note><en-media type="image/vnd.adobe.photoshop" hash="psd"/></en-note>"#
						.into(),
				resources: vec![Resource {
					hash: "psd".into(),
					file_name: "poster.avif".into(),
					original_file_name: Some("poster.psd".into()),
					mime: "image/avif".into(),
					s3_key: None,
					text_preview: None,
					archive_tree: None,
				}],
			},
		];

		let posts = notes_to_posts(&notes, true);

		assert_eq!(
			posts[1].body,
			r#"<a class="attachment" href="poster.psd" download>poster.psd</a>"#
		);
	}

	#[test]
	fn maps_podcast_audio_to_shortcode() {
		let note = Note {
			guid: "abc".into(),
			title: "Podcast".into(),
			created: utc(1_700_000_000),
			updated: utc(1_700_000_000),
			tags: vec!["podcast".into()],
			enml: r#"<en-note><en-media type="audio/mpeg" hash="abc"/></en-note>"#.into(),
			resources: vec![Resource {
				hash: "abc".into(),
				file_name: "episode.mp3".into(),
				original_file_name: None,
				mime: "audio/mpeg".into(),
				s3_key: None,
				text_preview: None,
				archive_tree: None,
			}],
		};
		let post = notes_to_posts(&[note], true).remove(0);

		assert!(post.body.contains(r#"{{ audio(src="episode.mp3") }}"#));
		assert!(post.tags.contains(&"podcast".to_string()));
	}

	#[test]
	fn deduplicates_duplicate_title_slugs() {
		let target_guid = "22222222-2222-2222-2222-222222222222";
		let notes = vec![
			Note {
				guid: "11111111-1111-1111-1111-111111111111".into(),
				title: "Same title".into(),
				created: utc(1_700_000_000),
				updated: utc(1_700_000_000),
				tags: vec![],
				enml: format!(
					r#"<en-note><a href="evernote:///view/1/s1/{target_guid}/{target_guid}/">next</a></en-note>"#
				),
				resources: vec![],
			},
			Note {
				guid: target_guid.into(),
				title: "Same title".into(),
				created: utc(1_700_000_001),
				updated: utc(1_700_000_001),
				tags: vec![],
				enml: "<en-note>duplicate</en-note>".into(),
				resources: vec![],
			},
		];
		let posts = notes_to_posts(&notes, true);

		assert_eq!(posts[0].slug, "same-title");
		assert_eq!(posts[1].slug, "same-title-20231114-221321");
		assert!(
			posts[0]
				.body
				.contains(r#"/posts/same-title-20231114-221321/"#)
		);
	}

	#[test]
	fn deduplicates_duplicate_explicit_slug_tags() {
		let notes = vec![
			Note {
				guid: "11111111-1111-1111-1111-111111111111".into(),
				title: "First title".into(),
				created: utc(1_700_000_000),
				updated: utc(1_700_000_000),
				tags: vec!["slug:custom".into()],
				enml: "<en-note>first</en-note>".into(),
				resources: vec![],
			},
			Note {
				guid: "22222222-2222-2222-2222-222222222222".into(),
				title: "Second title".into(),
				created: utc(1_700_000_001),
				updated: utc(1_700_000_001),
				tags: vec!["slug:custom".into()],
				enml: "<en-note>second</en-note>".into(),
				resources: vec![],
			},
		];
		let posts = notes_to_posts(&notes, true);

		assert_eq!(posts[0].slug, "custom");
		assert_eq!(posts[1].slug, "custom-20231114-221321");
	}

	#[test]
	fn signs_oauth_temporary_token_url() {
		let url = temporary_token_url(
			&OAuthCredentials {
				consumer_key: "key".into(),
				consumer_secret: "secret".into(),
			},
			&OAuthTemporaryRequest {
				callback_url: "https://example.com/callback".into(),
				nonce: "nonce".into(),
				timestamp: 1_700_000_000,
			},
		)
		.unwrap();

		assert!(url.starts_with("https://www.evernote.com/oauth?"));
		assert!(url.contains("oauth_signature="));
		assert!(url.contains("oauth_callback=https%3A%2F%2Fexample.com%2Fcallback"));
	}
}
