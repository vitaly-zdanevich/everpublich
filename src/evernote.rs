//! Evernote-specific mapping and OAuth helpers.
//!
//! Evernote's public documentation still describes OAuth 1.0a. This module only
//! builds signed OAuth URLs and transforms already-fetched notes. Network calls
//! are intentionally isolated for the Lambda adapter so tests do not need
//! Evernote credentials.

use crate::enml::enml_to_zola_body;
use crate::models::{Note, Post, PostKind};
use crate::slug::slug_from_title_and_tags;
use crate::widgets::expand_bare_links;
use anyhow::{Context, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use sha1::Sha1;
use std::collections::{BTreeMap, HashMap};

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
	/// OAuth callback URL configured for the Lambda endpoint.
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
	let note_slug_by_guid = notes
		.iter()
		.map(|note| {
			(
				note.guid.to_ascii_lowercase(),
				slug_from_title_and_tags(&note.title, &note.tags),
			)
		})
		.collect::<HashMap<_, _>>();

	notes
		.iter()
		.map(|note| note_to_post(note, &note_slug_by_guid, expand_widgets))
		.collect()
}

/// Convert one Evernote note to one generated post/page.
pub fn note_to_post(
	note: &Note,
	note_slug_by_guid: &HashMap<String, String>,
	expand_widgets: bool,
) -> Post {
	let slug = slug_from_title_and_tags(&note.title, &note.tags);
	let kind = post_kind(&note.tags);
	let body = enml_to_zola_body(&note.enml, &note.resources, note_slug_by_guid);
	let body = expand_bare_links(&body, expand_widgets);

	Post {
		guid: note.guid.clone(),
		slug,
		title: clean_title(&note.title),
		date: note.created,
		tags: public_tags(&note.tags),
		body,
		resources: note.resources.clone(),
		kind,
	}
}

fn post_kind(tags: &[String]) -> PostKind {
	if has_tag(tags, "about") {
		PostKind::About
	} else if has_tag(tags, "page") {
		PostKind::Page
	} else {
		PostKind::BlogPost
	}
}

fn public_tags(tags: &[String]) -> Vec<String> {
	tags.iter()
		.filter(|tag| {
			!tag.starts_with("slug:")
				&& !matches!(tag.as_str(), "page" | "about" | "podcast_description")
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
				mime: "audio/mpeg".into(),
				s3_key: None,
			}],
		};
		let post = notes_to_posts(&[note], true).remove(0);

		assert!(post.body.contains(r#"{{ audio(src="episode.mp3") }}"#));
		assert!(post.tags.contains(&"podcast".to_string()));
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
