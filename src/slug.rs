//! URL slug generation for site names and note titles.

use sha2::{Digest, Sha256};

/// Make a stable, URL-friendly slug. Unicode letters are preserved because Zola
/// can serve them, while whitespace and punctuation collapse to one dash.
pub fn slugify(input: &str) -> String {
	let mut out = String::new();
	let mut dash = false;

	for c in input.trim().chars() {
		let c = c.to_lowercase().next().unwrap_or(c);
		if c.is_alphanumeric() {
			out.push(c);
			dash = false;
		} else if !dash && !out.is_empty() {
			out.push('-');
			dash = true;
		}
	}

	while out.ends_with('-') {
		out.pop();
	}

	if out.is_empty() {
		let digest = Sha256::digest(input.as_bytes());
		format!("note-{}", hex_bytes(&digest[..4]))
	} else {
		out
	}
}

/// Prefer an explicit `slug:...` tag, otherwise derive from the note title.
pub fn slug_from_title_and_tags(title: &str, tags: &[String]) -> String {
	tags.iter()
		.find_map(|tag| tag.strip_prefix("slug:"))
		.map(slugify)
		.filter(|slug| !slug.is_empty())
		.unwrap_or_else(|| slugify(title))
}

fn hex_bytes(bytes: &[u8]) -> String {
	bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn slug_collapses_punctuation() {
		assert_eq!(slugify(" Hello, Evernote blog! "), "hello-evernote-blog");
	}

	#[test]
	fn slug_tag_wins() {
		assert_eq!(
			slug_from_title_and_tags("Title", &[String::from("slug:my-url")]),
			"my-url"
		);
	}

	#[test]
	fn empty_title_gets_stable_fallback() {
		assert!(slugify("!!!").starts_with("note-"));
	}
}
