//! Expand supported external links into static-site widgets.

use html_escape::encode_double_quoted_attribute;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::env;
use std::sync::OnceLock;
use std::time::Duration;
use url::Url;

/// Supported external widget providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetProvider {
	/// YouTube video or short.
	YouTube,
	/// Instagram post.
	Instagram,
	/// Pinterest pin.
	Pinterest,
	/// Spotify track, album, playlist, or episode.
	Spotify,
	/// Genius lyrics page.
	Genius,
	/// SoundCloud track or playlist.
	SoundCloud,
	/// Apple Podcasts page.
	ApplePodcasts,
	/// Vimeo video.
	Vimeo,
	/// Rumble video.
	Rumble,
	/// Dailymotion video.
	Dailymotion,
	/// Bilibili video.
	Bilibili,
	/// Odysee video.
	Odysee,
	/// Yandex Music track or album.
	YandexMusic,
	/// Bandcamp track or album.
	Bandcamp,
	/// TikTok video.
	TikTok,
	/// Twitch clip or video.
	Twitch,
	/// Mixcloud show.
	Mixcloud,
	/// Internet Archive item.
	InternetArchive,
	/// GitHub Gist.
	GitHubGist,
	/// CodePen pen.
	CodePen,
	/// Figma file.
	Figma,
	/// Google Maps place.
	GoogleMaps,
	/// Reddit post.
	Reddit,
	/// Mastodon post.
	Mastodon,
	/// Bluesky post.
	Bluesky,
	/// Telegram public post.
	Telegram,
}

impl WidgetProvider {
	/// Human-readable provider label.
	pub fn label(self) -> &'static str {
		match self {
			Self::YouTube => "YouTube",
			Self::Instagram => "Instagram",
			Self::Pinterest => "Pinterest",
			Self::Spotify => "Spotify",
			Self::Genius => "Genius",
			Self::SoundCloud => "SoundCloud",
			Self::ApplePodcasts => "Apple Podcasts",
			Self::Vimeo => "Vimeo",
			Self::Rumble => "Rumble",
			Self::Dailymotion => "Dailymotion",
			Self::Bilibili => "Bilibili",
			Self::Odysee => "Odysee",
			Self::YandexMusic => "Yandex Music",
			Self::Bandcamp => "Bandcamp",
			Self::TikTok => "TikTok",
			Self::Twitch => "Twitch",
			Self::Mixcloud => "Mixcloud",
			Self::InternetArchive => "Internet Archive",
			Self::GitHubGist => "GitHub Gist",
			Self::CodePen => "CodePen",
			Self::Figma => "Figma",
			Self::GoogleMaps => "Google Maps",
			Self::Reddit => "Reddit",
			Self::Mastodon => "Mastodon",
			Self::Bluesky => "Bluesky",
			Self::Telegram => "Telegram",
		}
	}
}

/// A detected widget expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widget {
	/// Provider matched from the URL.
	pub provider: WidgetProvider,
	/// Original note URL.
	pub original_url: String,
	/// Zola shortcode or fallback HTML.
	pub shortcode: String,
}

/// Human-readable names of all supported and planned widget providers.
pub fn supported_widget_names() -> Vec<&'static str> {
	[
		WidgetProvider::YouTube,
		WidgetProvider::Instagram,
		WidgetProvider::Pinterest,
		WidgetProvider::Spotify,
		WidgetProvider::Genius,
		WidgetProvider::SoundCloud,
		WidgetProvider::ApplePodcasts,
		WidgetProvider::Vimeo,
		WidgetProvider::Rumble,
		WidgetProvider::Dailymotion,
		WidgetProvider::Bilibili,
		WidgetProvider::Odysee,
		WidgetProvider::YandexMusic,
		WidgetProvider::Bandcamp,
		WidgetProvider::TikTok,
		WidgetProvider::Twitch,
		WidgetProvider::Mixcloud,
		WidgetProvider::InternetArchive,
		WidgetProvider::GitHubGist,
		WidgetProvider::CodePen,
		WidgetProvider::Figma,
		WidgetProvider::GoogleMaps,
		WidgetProvider::Reddit,
		WidgetProvider::Mastodon,
		WidgetProvider::Bluesky,
		WidgetProvider::Telegram,
	]
	.into_iter()
	.map(WidgetProvider::label)
	.collect()
}

/// Detect whether a URL has a known embeddable representation.
pub fn detect(url: &str) -> Option<Widget> {
	let parsed = Url::parse(url).ok()?;
	let host = parsed
		.host_str()?
		.trim_start_matches("www.")
		.to_ascii_lowercase();
	let provider = match host.as_str() {
		"youtu.be" | "youtube.com" | "music.youtube.com" => WidgetProvider::YouTube,
		"instagram.com" => WidgetProvider::Instagram,
		"pinterest.com" | "pin.it" => WidgetProvider::Pinterest,
		"open.spotify.com" => WidgetProvider::Spotify,
		"genius.com" => WidgetProvider::Genius,
		"soundcloud.com" => WidgetProvider::SoundCloud,
		"podcasts.apple.com" => WidgetProvider::ApplePodcasts,
		"vimeo.com" | "player.vimeo.com" => WidgetProvider::Vimeo,
		"rumble.com" => WidgetProvider::Rumble,
		"dailymotion.com" | "dai.ly" => WidgetProvider::Dailymotion,
		"bilibili.com" | "b23.tv" => WidgetProvider::Bilibili,
		"odysee.com" => WidgetProvider::Odysee,
		"music.yandex.ru" | "music.yandex.com" => WidgetProvider::YandexMusic,
		h if h.ends_with(".bandcamp.com") || h == "bandcamp.com" => WidgetProvider::Bandcamp,
		"tiktok.com" => WidgetProvider::TikTok,
		"twitch.tv" => WidgetProvider::Twitch,
		"mixcloud.com" => WidgetProvider::Mixcloud,
		"archive.org" => WidgetProvider::InternetArchive,
		"gist.github.com" => WidgetProvider::GitHubGist,
		"codepen.io" => WidgetProvider::CodePen,
		"figma.com" => WidgetProvider::Figma,
		"maps.google.com" | "google.com" => WidgetProvider::GoogleMaps,
		"reddit.com" | "old.reddit.com" => WidgetProvider::Reddit,
		"bsky.app" => WidgetProvider::Bluesky,
		"t.me" | "telegram.me" | "telegram.dog" => WidgetProvider::Telegram,
		_ if looks_like_mastodon(&host) => WidgetProvider::Mastodon,
		_ => return None,
	};

	Some(Widget {
		provider,
		original_url: url.to_string(),
		shortcode: shortcode(provider, url, &parsed),
	})
}

fn looks_like_mastodon(host: &str) -> bool {
	matches!(
		host,
		"mastodon.social" | "fosstodon.org" | "hachyderm.io" | "mstdn.social" | "piaille.fr"
	)
}

fn shortcode(provider: WidgetProvider, original: &str, parsed: &Url) -> String {
	match provider {
		WidgetProvider::YouTube => youtube_id(parsed)
			.map(|id| format!(r#"{{{{ youtube(id="{id}") }}}}"#))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::Vimeo => parsed
			.path_segments()
			.and_then(|mut s| s.find(|part| part.chars().all(|c| c.is_ascii_digit())))
			.map(|id| format!(r#"{{{{ vimeo(id="{id}") }}}}"#))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::Spotify => spotify_embed(parsed)
			.map(|url| format!(r#"{{{{ spotify(url="{}") }}}}"#, shortcode_arg(&url)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::ApplePodcasts => {
			let url = apple_podcast_embed(original);
			format!(r#"{{{{ apple_podcast(url="{}") }}}}"#, shortcode_arg(&url))
		}
		WidgetProvider::YandexMusic => yandex_music_embed(parsed)
			.map(|url| format!(r#"{{{{ yandex_music(url="{}") }}}}"#, shortcode_arg(&url)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::Instagram => format!(r#"{{{{ instagram(url="{original}") }}}}"#),
		WidgetProvider::Pinterest => pinterest_pin(parsed)
			.map(|url| format!(r#"{{{{ pinterest(url="{}") }}}}"#, shortcode_arg(&url)))
			.unwrap_or_else(|| format!(r#"{{{{ pinterest(url="{original}") }}}}"#)),
		WidgetProvider::Genius => genius_shortcode(original),
		_ => generic_embed(provider, original),
	}
}

fn generic_embed(provider: WidgetProvider, original: &str) -> String {
	format!(
		r#"<p class="embed-link"><a href="{}" rel="noopener">{}</a></p>"#,
		encode_double_quoted_attribute(original),
		provider.label()
	)
}

fn youtube_id(url: &Url) -> Option<String> {
	if url.host_str().is_some_and(|h| h.ends_with("youtu.be")) {
		return url.path_segments()?.next().map(str::to_string);
	}
	url.query_pairs()
		.find(|(key, _)| key == "v")
		.map(|(_, value)| value.into_owned())
		.or_else(|| {
			let parts = url.path_segments()?.collect::<Vec<_>>();
			parts
				.windows(2)
				.find(|w| matches!(w[0], "embed" | "shorts"))
				.map(|w| w[1].to_string())
		})
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GeniusResolution {
	song_id: Option<String>,
	youtube_id: Option<String>,
}

fn genius_shortcode(original: &str) -> String {
	resolve_genius(original)
		.filter(|resolution| resolution.song_id.is_some() || resolution.youtube_id.is_some())
		.map(|resolution| genius_shortcodes(original, &resolution))
		.unwrap_or_else(|| generic_embed(WidgetProvider::Genius, original))
}

fn resolve_genius(url: &str) -> Option<GeniusResolution> {
	genius_override(url)
		.or_else(|| genius_api(url))
		.or_else(|| genius_page(url))
}

fn genius_override(url: &str) -> Option<GeniusResolution> {
	let wanted = normalize_genius_url(url);
	env::var("GENIUS_EMBED_OVERRIDES")
		.ok()?
		.split(';')
		.filter_map(|entry| entry.split_once('='))
		.find_map(|(entry_url, ids)| {
			(normalize_genius_url(entry_url.trim()) == wanted)
				.then(|| {
					let mut parts = ids
						.split(',')
						.map(str::trim)
						.filter(|part| !part.is_empty());
					GeniusResolution {
						song_id: parts.next().map(str::to_string),
						youtube_id: parts.next().map(str::to_string),
					}
				})
				.filter(|resolution| {
					resolution.song_id.is_some() || resolution.youtube_id.is_some()
				})
		})
}

fn genius_api(url: &str) -> Option<GeniusResolution> {
	let token = env::var("GENIUS_TOKEN")
		.ok()
		.map(|token| token.trim().to_string())
		.filter(|token| !token.is_empty())?;
	let client = genius_client()?;
	let query = genius_slug_to_query(url);
	let search: Value = client
		.get("https://api.genius.com/search")
		.query(&[("q", query)])
		.bearer_auth(&token)
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	let id = search["response"]["hits"]
		.as_array()?
		.iter()
		.find_map(|hit| {
			let result = &hit["result"];
			let result_url = result["url"].as_str()?;
			same_genius_song(result_url, url)
				.then(|| result["id"].as_u64())
				.flatten()
		})?;
	let song: Value = client
		.get(format!("https://api.genius.com/songs/{id}"))
		.bearer_auth(token)
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	let youtube_id = song["response"]["song"]["media"]
		.as_array()
		.and_then(|items| {
			items.iter().find_map(|item| {
				(item["provider"].as_str()? == "youtube")
					.then(|| item["url"].as_str())
					.flatten()
					.and_then(youtube_id_from_str)
			})
		});
	Some(GeniusResolution {
		song_id: Some(id.to_string()),
		youtube_id,
	})
}

fn genius_page(url: &str) -> Option<GeniusResolution> {
	let html = genius_client()?
		.get(url)
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()?;
	parse_genius_html(&html)
}

fn genius_client() -> Option<Client> {
	Client::builder()
		.timeout(Duration::from_secs(10))
		.user_agent("Mozilla/5.0 (compatible; Everpublich/0.2)")
		.build()
		.ok()
}

fn parse_genius_html(html: &str) -> Option<GeniusResolution> {
	let song_id = genius_song_id_regex()
		.captures(html)
		.and_then(|caps| caps.get(1))
		.map(|m| m.as_str().to_string());
	let youtube_id = youtube_id_from_str(html);
	(song_id.is_some() || youtube_id.is_some()).then_some(GeniusResolution {
		song_id,
		youtube_id,
	})
}

fn genius_song_id_regex() -> &'static Regex {
	static REGEX: OnceLock<Regex> = OnceLock::new();
	REGEX.get_or_init(|| Regex::new(r#"(?:songs/|song:)(\d+)(?:/embed)?"#).unwrap())
}

fn genius_slug_to_query(url: &str) -> String {
	let slug = url
		.trim_end_matches('/')
		.rsplit('/')
		.next()
		.unwrap_or_default();
	slug.strip_suffix("-lyrics")
		.unwrap_or(slug)
		.replace('-', " ")
}

fn same_genius_song(a: &str, b: &str) -> bool {
	normalize_genius_url(a) == normalize_genius_url(b)
}

fn normalize_genius_url(url: &str) -> String {
	url.trim()
		.trim_end_matches('/')
		.replacen("http://", "https://", 1)
}

fn genius_shortcodes(original: &str, resolution: &GeniusResolution) -> String {
	let mut out = String::new();
	if let Some(id) = &resolution.youtube_id {
		out.push_str(&format!(r#"{{{{ youtube(id="{}") }}}}"#, shortcode_arg(id)));
	}
	if let Some(song_id) = &resolution.song_id {
		if !out.is_empty() {
			out.push_str("\n\n");
		}
		out.push_str(&format!(
			r#"{{{{ genius(song_id="{}", url="{}") }}}}"#,
			shortcode_arg(song_id),
			shortcode_arg(original)
		));
	}
	out
}

fn youtube_id_from_str(text: &str) -> Option<String> {
	youtube_url_regex()
		.captures(text)
		.and_then(|caps| caps.get(1))
		.map(|m| m.as_str().to_string())
}

fn youtube_url_regex() -> &'static Regex {
	static REGEX: OnceLock<Regex> = OnceLock::new();
	REGEX.get_or_init(|| {
		Regex::new(
			r#"(?i)(?:youtube(?:-nocookie)?\.com/(?:watch\?v=|embed/|shorts/|live/|v/)|youtu\.be/)([A-Za-z0-9_-]{6,})"#,
		)
		.unwrap()
	})
}

fn spotify_embed(url: &Url) -> Option<String> {
	let mut parts = url.path_segments()?;
	let kind = parts.next()?;
	let id = parts.next()?;
	matches!(kind, "track" | "album" | "playlist" | "episode" | "show")
		.then(|| format!("https://open.spotify.com/embed/{kind}/{id}"))
}

fn apple_podcast_embed(url: &str) -> String {
	if url.contains("//embed.podcasts.apple.com/") {
		url.to_string()
	} else {
		url.replacen("//podcasts.apple.com/", "//embed.podcasts.apple.com/", 1)
	}
}

fn yandex_music_embed(url: &Url) -> Option<String> {
	let parts = url.path_segments()?.collect::<Vec<_>>();
	parts
		.windows(3)
		.find(|window| window[0] == "album" && window[2] == "track")
		.and_then(|window| {
			let album = window[1];
			let track = parts.get(
				parts
					.iter()
					.position(|part| *part == "track")
					.unwrap_or_default()
					+ 1,
			)?;
			Some(format!(
				"https://music.yandex.ru/iframe/#track/{track}/{album}"
			))
		})
}

fn pinterest_pin(url: &Url) -> Option<String> {
	let parts = url.path_segments()?.collect::<Vec<_>>();
	parts
		.windows(2)
		.find(|window| window[0] == "pin" && window[1].chars().all(|c| c.is_ascii_digit()))
		.map(|window| format!("https://www.pinterest.com/pin/{}/", window[1]))
}

fn shortcode_arg(value: &str) -> String {
	value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Expand bare URL lines into widget shortcodes; Markdown links are left alone.
pub fn expand_bare_links(markdown: &str, enabled: bool) -> String {
	if !enabled {
		return markdown.to_string();
	}
	let paragraph_url = Regex::new(r#"(?is)<p>\s*(https?://[^<\s]+)\s*</p>"#).unwrap();
	let markdown = paragraph_url
		.replace_all(markdown, |caps: &regex::Captures| {
			let url = clean_url(caps.get(1).unwrap().as_str());
			detect(url)
				.map(|w| w.shortcode)
				.unwrap_or_else(|| caps.get(0).unwrap().as_str().to_string())
		})
		.into_owned();
	let url_line = Regex::new(r"^\s*(https?://\S+)\s*$").unwrap();
	markdown
		.lines()
		.map(|line| {
			if let Some(caps) = url_line.captures(line) {
				let url = clean_url(caps.get(1).unwrap().as_str());
				detect(url)
					.map(|w| w.shortcode)
					.unwrap_or_else(|| line.to_string())
			} else {
				line.to_string()
			}
		})
		.collect::<Vec<_>>()
		.join("\n")
}

fn clean_url(url: &str) -> &str {
	url.trim_end_matches(['.', ',', ')'])
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn detects_requested_widgets() {
		assert_eq!(
			detect("https://www.youtube.com/watch?v=abc")
				.unwrap()
				.provider,
			WidgetProvider::YouTube
		);
		assert_eq!(
			detect("https://open.spotify.com/track/123")
				.unwrap()
				.provider,
			WidgetProvider::Spotify
		);
		assert_eq!(
			detect("https://music.yandex.ru/album/1/track/2")
				.unwrap()
				.provider,
			WidgetProvider::YandexMusic
		);
	}

	#[test]
	fn expands_only_bare_links() {
		let md = "hello\nhttps://youtu.be/abc\n[link](https://youtu.be/abc)";
		let out = expand_bare_links(md, true);

		assert!(out.contains(r#"{{ youtube(id="abc") }}"#), "{out}");
		assert!(out.contains("[link](https://youtu.be/abc)"), "{out}");
	}

	#[test]
	fn normalizes_telegram_project_embed_urls() {
		assert_eq!(
			detect("https://open.spotify.com/track/1ZKipeRdw2frIZBd6Y0wNZ?si=x")
				.unwrap()
				.shortcode,
			r#"{{ spotify(url="https://open.spotify.com/embed/track/1ZKipeRdw2frIZBd6Y0wNZ") }}"#
		);
		assert_eq!(
			detect("https://podcasts.apple.com/us/podcast/x/id123?i=456")
				.unwrap()
				.shortcode,
			r#"{{ apple_podcast(url="https://embed.podcasts.apple.com/us/podcast/x/id123?i=456") }}"#
		);
		assert_eq!(
			detect("https://music.yandex.ru/album/22206733/track/103670414")
				.unwrap()
				.shortcode,
			r#"{{ yandex_music(url="https://music.yandex.ru/iframe/#track/103670414/22206733") }}"#
		);
		assert_eq!(
			detect("https://www.pinterest.com/pin/1234567890/sent/")
				.unwrap()
				.shortcode,
			r#"{{ pinterest(url="https://www.pinterest.com/pin/1234567890/") }}"#
		);
	}

	#[test]
	fn parses_genius_song_and_youtube_ids() {
		let html = r#"<div id="live_performance:song:122476"></div>
<iframe src="https://www.youtube-nocookie.com/embed/9z-Mh9Qeinw"></iframe>"#;
		let resolution = parse_genius_html(html).unwrap();

		assert_eq!(resolution.song_id.as_deref(), Some("122476"));
		assert_eq!(resolution.youtube_id.as_deref(), Some("9z-Mh9Qeinw"));
	}

	#[test]
	fn genius_resolution_renders_youtube_and_lyrics_widgets() {
		let shortcodes = genius_shortcodes(
			"https://genius.com/Bonnie-tyler-total-eclipse-of-the-heart-lyrics",
			&GeniusResolution {
				song_id: Some("122476".into()),
				youtube_id: Some("9z-Mh9Qeinw".into()),
			},
		);

		assert!(shortcodes.contains(r#"{{ youtube(id="9z-Mh9Qeinw") }}"#));
		assert!(shortcodes.contains(r#"{{ genius(song_id="122476""#));
	}

	#[test]
	fn lists_extra_widget_ideas() {
		assert!(supported_widget_names().contains(&"Bandcamp"));
		assert!(supported_widget_names().contains(&"TikTok"));
		assert!(supported_widget_names().contains(&"Internet Archive"));
	}
}
