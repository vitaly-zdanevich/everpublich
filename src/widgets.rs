//! Expand supported external links into static-site widgets.

use chrono::{DateTime, Utc};
use html_escape::{decode_html_entities, encode_double_quoted_attribute, encode_text};
use percent_encoding::{
	AsciiSet, CONTROLS, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode,
};
use regex::Regex;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::redirect::Policy;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use url::{Host, Url};

const PATH_SEGMENT_ENCODE: &AsciiSet = &CONTROLS
	.add(b' ')
	.add(b'"')
	.add(b'#')
	.add(b'%')
	.add(b'<')
	.add(b'>')
	.add(b'?')
	.add(b'`')
	.add(b'{')
	.add(b'}')
	.add(b'/');

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
	/// Steam store app page.
	Steam,
	/// VK audio playlist.
	VkPlaylist,
	/// OK.ru video.
	OkRu,
	/// my.mail.ru video.
	MyMailRu,
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
			Self::Steam => "Steam",
			Self::VkPlaylist => "VK playlist",
			Self::OkRu => "OK.ru",
			Self::MyMailRu => "my.mail.ru",
		}
	}

	/// Stable lowercase key used by `everpublich:config`.
	pub fn config_key(self) -> &'static str {
		match self {
			Self::YouTube => "youtube",
			Self::Instagram => "instagram",
			Self::Pinterest => "pinterest",
			Self::Spotify => "spotify",
			Self::Genius => "genius",
			Self::SoundCloud => "soundcloud",
			Self::ApplePodcasts => "apple-podcasts",
			Self::Vimeo => "vimeo",
			Self::Rumble => "rumble",
			Self::Dailymotion => "dailymotion",
			Self::Bilibili => "bilibili",
			Self::Odysee => "odysee",
			Self::YandexMusic => "yandex-music",
			Self::Bandcamp => "bandcamp",
			Self::TikTok => "tiktok",
			Self::Twitch => "twitch",
			Self::Mixcloud => "mixcloud",
			Self::InternetArchive => "internet-archive",
			Self::GitHubGist => "github-gist",
			Self::CodePen => "codepen",
			Self::Figma => "figma",
			Self::GoogleMaps => "google-maps",
			Self::Reddit => "reddit",
			Self::Mastodon => "mastodon",
			Self::Bluesky => "bluesky",
			Self::Telegram => "telegram",
			Self::Steam => "steam",
			Self::VkPlaylist => "vk-playlist",
			Self::OkRu => "ok-ru",
			Self::MyMailRu => "my-mail-ru",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectMediaKind {
	Audio,
	Video,
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
		WidgetProvider::Steam,
		WidgetProvider::VkPlaylist,
		WidgetProvider::OkRu,
		WidgetProvider::MyMailRu,
	]
	.into_iter()
	.map(WidgetProvider::label)
	.collect()
}

/// Normalize a config note provider name to a supported widget key.
pub fn normalize_widget_provider_name(name: &str) -> Option<&'static str> {
	let wanted = normalize_widget_key(name);
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
		WidgetProvider::Steam,
		WidgetProvider::VkPlaylist,
		WidgetProvider::OkRu,
		WidgetProvider::MyMailRu,
	]
	.into_iter()
	.find(|provider| {
		wanted == provider.config_key() || wanted == normalize_widget_key(provider.label())
	})
	.map(WidgetProvider::config_key)
}

fn normalize_widget_key(value: &str) -> String {
	let mut out = String::new();
	let mut previous_dash = false;
	for ch in value.chars().flat_map(char::to_lowercase) {
		if ch.is_ascii_alphanumeric() {
			out.push(ch);
			previous_dash = false;
		} else if !previous_dash && !out.is_empty() {
			out.push('-');
			previous_dash = true;
		}
	}
	out.trim_matches('-').to_string()
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
		"dailymotion.com" | "geo.dailymotion.com" | "dai.ly" => WidgetProvider::Dailymotion,
		"bilibili.com" | "b23.tv" | "bilibili.tv" => WidgetProvider::Bilibili,
		"odysee.com" => WidgetProvider::Odysee,
		"music.yandex.ru" | "music.yandex.com" => WidgetProvider::YandexMusic,
		h if h.ends_with(".bandcamp.com") || h == "bandcamp.com" => WidgetProvider::Bandcamp,
		"tiktok.com" => WidgetProvider::TikTok,
		"ok.ru" | "m.ok.ru" => WidgetProvider::OkRu,
		"my.mail.ru" | "m.my.mail.ru" => WidgetProvider::MyMailRu,
		"twitch.tv" => WidgetProvider::Twitch,
		"mixcloud.com" => WidgetProvider::Mixcloud,
		"archive.org" => WidgetProvider::InternetArchive,
		"gist.github.com" => WidgetProvider::GitHubGist,
		"codepen.io" => WidgetProvider::CodePen,
		"figma.com" => WidgetProvider::Figma,
		"maps.google.com" | "google.com" => WidgetProvider::GoogleMaps,
		"reddit.com" | "old.reddit.com" | "new.reddit.com" => WidgetProvider::Reddit,
		"bsky.app" => WidgetProvider::Bluesky,
		"t.me" | "telegram.me" | "telegram.dog" => WidgetProvider::Telegram,
		"store.steampowered.com" => WidgetProvider::Steam,
		h if matches!(h, "vk.com" | "m.vk.com") && vk_audio_playlist(&parsed).is_some() => {
			WidgetProvider::VkPlaylist
		}
		_ if mastodon_supported_url(&parsed)
			&& (looks_like_mastodon(&host) || mastodon_embed(&parsed).is_some()) =>
		{
			WidgetProvider::Mastodon
		}
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
		WidgetProvider::Vimeo => vimeo_video_id(parsed)
			.map(|id| format!(r#"{{{{ vimeo(id="{id}") }}}}"#))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::Spotify => spotify_embed(parsed)
			.map(|url| format!(r#"{{{{ spotify(url="{}") }}}}"#, shortcode_arg(&url)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::SoundCloud => soundcloud_embed(parsed)
			.map(|url| format!(r#"{{{{ soundcloud(url="{}") }}}}"#, shortcode_arg(&url)))
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
		WidgetProvider::Rumble => rumble_embed(parsed)
			.map(|url| format!(r#"{{{{ rumble(url="{}") }}}}"#, shortcode_arg(&url)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::Dailymotion => dailymotion_video_id(parsed)
			.map(|id| format!(r#"{{{{ dailymotion(id="{}") }}}}"#, shortcode_arg(&id)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::Odysee => odysee_embed(parsed)
			.map(|url| format!(r#"{{{{ odysee(url="{}") }}}}"#, shortcode_arg(&url)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::Bilibili => bilibili_embed(parsed)
			.map(|url| format!(r#"{{{{ bilibili(url="{}") }}}}"#, shortcode_arg(&url)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::TikTok => tiktok_video_id(parsed)
			.map(|id| {
				format!(
					r#"{{{{ tiktok(url="{}", id="{}") }}}}"#,
					shortcode_arg(original),
					shortcode_arg(&id)
				)
			})
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::OkRu => ok_ru_video_id(parsed)
			.map(|id| format!(r#"{{{{ ok_ru_video(id="{}") }}}}"#, shortcode_arg(&id)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::MyMailRu => my_mail_ru_embed_id(parsed)
			.map(|id| format!(r#"{{{{ my_mail_ru_video(id="{}") }}}}"#, shortcode_arg(&id)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::InternetArchive => archive_embed_identifier(parsed)
			.map(|id| format!(r#"{{{{ archive_org(id="{}") }}}}"#, shortcode_arg(&id)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::Steam => steam_app_id(parsed)
			.map(|id| format!(r#"{{{{ steam(app_id="{}") }}}}"#, shortcode_arg(&id)))
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::VkPlaylist => vk_audio_playlist(parsed)
			.map(|(oid, pid)| {
				format!(
					r#"{{{{ vk_playlist(oid="{}", pid="{}") }}}}"#,
					shortcode_arg(&oid),
					shortcode_arg(&pid)
				)
			})
			.unwrap_or_else(|| generic_embed(provider, original)),
		WidgetProvider::Reddit => {
			reddit_shortcode(original, parsed).unwrap_or_else(|| generic_embed(provider, original))
		}
		WidgetProvider::Mastodon => {
			mastodon_embed(parsed).unwrap_or_else(|| generic_embed(provider, original))
		}
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

fn vimeo_video_id(url: &Url) -> Option<&str> {
	let host = normalized_host(url)?;
	if !matches!(host.as_str(), "vimeo.com" | "player.vimeo.com") {
		return None;
	}
	url.path_segments()?
		.find(|part| part.chars().all(|c| c.is_ascii_digit()))
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
	let client = genius_client()?;
	let html = safe_get(&client, url)?
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
		.redirect(Policy::none())
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

fn soundcloud_embed(url: &Url) -> Option<String> {
	soundcloud_url(url).then(|| {
		format!(
			"https://w.soundcloud.com/player/?url={}",
			utf8_percent_encode(url.as_str(), NON_ALPHANUMERIC)
		)
	})
}

fn soundcloud_url(url: &Url) -> bool {
	normalized_host(url).as_deref() == Some("soundcloud.com") && url.path() != "/"
}

fn apple_podcast_embed(url: &str) -> String {
	if url.contains("//embed.podcasts.apple.com/") {
		url.to_string()
	} else {
		url.replacen("//podcasts.apple.com/", "//embed.podcasts.apple.com/", 1)
	}
}

fn apple_podcast_id(url: &Url) -> Option<String> {
	(normalized_host(url).as_deref() == Some("podcasts.apple.com")).then_some(())?;
	decoded_path_segments(url).into_iter().find_map(|segment| {
		segment
			.strip_prefix("id")
			.filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
			.map(str::to_string)
	})
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

fn rumble_embed(url: &Url) -> Option<String> {
	let parts = decoded_path_segments(url);
	if parts.first().is_some_and(|part| part == "embed") {
		return Some(url.as_str().to_string());
	}
	rumble_oembed_embed_url(url)
}

fn rumble_oembed_embed_url(url: &Url) -> Option<String> {
	ssrf_safe_url(url).then_some(())?;
	cached_widget_html(&format!("rumble-oembed:{}", url.as_str()), || {
		let client = metadata_client()?;
		let response: Value = safe_get(&client, "https://rumble.com/api/Media/oembed.json")?
			.query(&[("url", url.as_str())])
			.send()
			.ok()?
			.error_for_status()
			.ok()?
			.text()
			.ok()
			.and_then(|body| serde_json::from_str(&body).ok())?;
		rumble_embed_url_from_oembed(&response)
	})
}

fn rumble_embed_url_from_oembed(response: &Value) -> Option<String> {
	let html = response["html"].as_str()?;
	let url = first_capture(html, r#"(?is)\bsrc\s*=\s*(?:"([^"]+)"|'([^']+)')"#)?;
	let parsed = Url::parse(&decode_html_entities(&url)).ok()?;
	(normalized_host(&parsed).as_deref() == Some("rumble.com")
		&& decoded_path_segments(&parsed)
			.first()
			.is_some_and(|part| part == "embed")
		&& ssrf_safe_url(&parsed))
	.then(|| parsed.to_string())
}

/// Extract a Dailymotion video id from public watch, short, or player URLs.
fn dailymotion_video_id(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if host == "dai.ly" {
		return decoded_path_segments(url)
			.into_iter()
			.next()
			.filter(|id| dailymotion_id(id));
	}
	if !matches!(host.as_str(), "dailymotion.com" | "geo.dailymotion.com") {
		return None;
	}
	url.query_pairs()
		.find(|(key, _)| key == "video")
		.map(|(_, value)| value.into_owned())
		.filter(|id| dailymotion_id(id))
		.or_else(|| {
			let parts = decoded_path_segments(url);
			parts.windows(2).find_map(|window| {
				matches!(window[0].as_str(), "video" | "embed")
					.then(|| window[1].clone())
					.filter(|id| dailymotion_id(id))
			})
		})
}

fn dailymotion_id(id: &str) -> bool {
	!id.is_empty() && id.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn odysee_embed(url: &Url) -> Option<String> {
	let path = url.path().trim_start_matches('/');
	if path.is_empty() {
		return None;
	}
	if path.starts_with("$/embed/") {
		return Some(format!("https://odysee.com/{path}"));
	}
	Some(format!("https://odysee.com/$/embed/{path}"))
}

fn bilibili_embed(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if host == "bilibili.tv" {
		return Some(url.as_str().to_string());
	}
	if let Some(bvid) = bilibili_bvid(url) {
		return Some(format!(
			"https://player.bilibili.com/player.html?bvid={bvid}"
		));
	}
	let aid = decoded_path_segments(url)
		.into_iter()
		.find_map(|part| part.strip_prefix("av").map(str::to_string))
		.filter(|id| id.chars().all(|c| c.is_ascii_digit()))?;
	Some(format!("https://player.bilibili.com/player.html?aid={aid}"))
}

fn bilibili_bvid(url: &Url) -> Option<String> {
	let regex = Regex::new(r#"(?i)\bBV[0-9A-Za-z]{8,}\b"#).unwrap();
	regex
		.find(url.as_str())
		.map(|value| value.as_str().to_string())
}

fn tiktok_video_id(url: &Url) -> Option<String> {
	let parts = decoded_path_segments(url);
	parts.windows(2).find_map(|window| {
		(window[0] == "video" && window[1].chars().all(|c| c.is_ascii_digit()))
			.then(|| window[1].clone())
	})
}

fn ok_ru_video_id(url: &Url) -> Option<String> {
	let parts = decoded_path_segments(url);
	(parts.first().is_some_and(|part| part == "video"))
		.then(|| parts.get(1).cloned())
		.flatten()
		.filter(|id| id.chars().all(|c| c.is_ascii_digit()))
}

/// Resolve the numeric my.mail.ru embed id from its public video page HTML.
fn my_mail_ru_embed_id(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	matches!(host.as_str(), "my.mail.ru" | "m.my.mail.ru").then_some(())?;
	cached_widget_html(&format!("my-mail-ru:{}", url.as_str()), || {
		let client = metadata_client()?;
		let html = safe_get(&client, url.as_str())?
			.send()
			.ok()?
			.error_for_status()
			.ok()?
			.text()
			.ok()?;
		my_mail_ru_embed_id_from_html(&html)
	})
}

/// Parse a my.mail.ru video page and return the id accepted by `/video/embed/`.
fn my_mail_ru_embed_id_from_html(html: &str) -> Option<String> {
	first_capture(
		html,
		r#"(?is)\b(?:href|content)\s*=\s*(?:"[^"]*/\+/video/url/[^"]*/([0-9]+)"|'[^']*/\+/video/url/[^']*/([0-9]+)')"#,
	)
	.or_else(|| first_capture(html, r#"(?is)my\.mail\.ru/video/embed/([0-9]+)"#))
}

/// Return an Archive.org item id for URLs that should use the item player.
fn archive_embed_identifier(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if host != "archive.org" {
		return None;
	}
	let parts = decoded_path_segments(url);
	let marker = parts.first()?;
	matches!(marker.as_str(), "details" | "embed" | "stream")
		.then(|| parts.get(1).cloned())
		.flatten()
		.filter(|identifier| !identifier.is_empty())
}

fn steam_app_id(url: &Url) -> Option<String> {
	let parts = decoded_path_segments(url);
	(parts.first().is_some_and(|part| part == "app"))
		.then(|| parts.get(1).cloned())
		.flatten()
		.filter(|id| id.chars().all(|c| c.is_ascii_digit()))
}

fn vk_audio_playlist(url: &Url) -> Option<(String, String)> {
	let segment = decoded_path_segments(url).into_iter().next()?;
	let ids = segment.strip_prefix("audio_playlist")?;
	let (oid, pid) = ids.split_once('_')?;
	let oid_digits = oid.strip_prefix('-').unwrap_or(oid);
	(!oid_digits.is_empty()
		&& oid_digits.chars().all(|c| c.is_ascii_digit())
		&& !pid.is_empty()
		&& pid.chars().all(|c| c.is_ascii_digit()))
	.then(|| (oid.to_string(), pid.to_string()))
}

fn reddit_shortcode(original: &str, url: &Url) -> Option<String> {
	if reddit_subreddit(url).is_some() {
		return reddit_subreddit_widget(url).or_else(|| reddit_subreddit_card(url));
	}
	reddit_post_json_url(url)?;
	cached_widget_html(&format!("reddit-post:{}", url.as_str()), || {
		let alive = cached_probe_bool(&format!("reddit-post-alive:{}", url.as_str()), || {
			reddit_post_alive(url)
		})?;
		alive.then(|| reddit_embed_html(original))
	})
}

fn reddit_subreddit_widget(url: &Url) -> Option<String> {
	let subreddit = reddit_subreddit(url)?;
	let embed_url = reddit_subreddit_embed_url(&subreddit);
	let available = cached_probe_bool(&format!("reddit-subreddit-widget:{subreddit}"), || {
		reddit_subreddit_widget_available(&embed_url)
	})?;
	available.then(|| reddit_subreddit_widget_html(&subreddit, &embed_url))
}

fn reddit_subreddit_widget_html(subreddit: &str, embed_url: &str) -> String {
	format!(
		r#"<div class="embed embed-reddit-subreddit" data-subreddit="{}"><script src="{}" charset="UTF-8"></script><p><a href="https://www.reddit.com/r/{}/" rel="noopener">Open r/{}</a></p></div>"#,
		encode_double_quoted_attribute(subreddit),
		encode_double_quoted_attribute(embed_url),
		encode_double_quoted_attribute(subreddit),
		encode_text(subreddit)
	)
}

fn reddit_subreddit_embed_url(subreddit: &str) -> String {
	format!("https://www.reddit.com/r/{subreddit}/.embed?limit=5")
}

fn reddit_subreddit_widget_available(embed_url: &str) -> Option<bool> {
	let client = metadata_client()?;
	let response = safe_get(&client, embed_url)?.send().ok()?;
	Some(response.status().is_success())
}

fn reddit_subreddit_card(url: &Url) -> Option<String> {
	cached_widget_html(&format!("reddit-subreddit:{}", url.as_str()), || {
		let data = reddit_subreddit_about(url)?;
		reddit_subreddit_card_from_data(&data)
	})
}

fn reddit_subreddit_about(url: &Url) -> Option<Value> {
	let subreddit = reddit_subreddit(url)?;
	metadata_client()?
		.get(format!("https://www.reddit.com/r/{subreddit}/about.json"))
		.query(&[("raw_json", "1")])
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())
}

fn reddit_subreddit_card_from_data(data: &Value) -> Option<String> {
	let subreddit = json_string(&data["data"]["display_name_prefixed"])
		.or_else(|| json_string(&data["data"]["display_name"]).map(|name| format!("r/{name}")))?;
	let url = json_string(&data["data"]["url"])
		.map(|path| format!("https://www.reddit.com{}", path.trim_end_matches('/')))
		.unwrap_or_else(|| format!("https://www.reddit.com/{subreddit}"));
	let title = json_string(&data["data"]["title"]).unwrap_or_else(|| subreddit.clone());
	let description = json_string(&data["data"]["public_description"])
		.or_else(|| json_string(&data["data"]["description"]))
		.map(|description| strip_html(&description))
		.and_then(|description| compact_title_text(&description));
	let subscribers = json_u64(&data["data"]["subscribers"]);
	let active = json_u64(&data["data"]["active_user_count"]);
	let icon = json_string(&data["data"]["community_icon"])
		.or_else(|| json_string(&data["data"]["icon_img"]))
		.and_then(|icon| icon.split('?').next().map(str::to_string));

	let mut html = String::new();
	html.push_str(r#"<div class="embed reddit-subreddit-card">"#);
	if let Some(icon) = icon.filter(|icon| !icon.is_empty()) {
		html.push_str(&format!(
			r#"<a class="reddit-subreddit-card__icon" href="{}" rel="noopener"><img src="{}" alt=""></a>"#,
			encode_double_quoted_attribute(&url),
			encode_double_quoted_attribute(&icon)
		));
	}
	html.push_str(r#"<div class="reddit-subreddit-card__body">"#);
	html.push_str(&format!(
		r#"<a class="reddit-subreddit-card__name" href="{}" rel="noopener"><strong>{}</strong><span>{}</span></a>"#,
		encode_double_quoted_attribute(&url),
		encode_text(&title),
		encode_text(&subreddit)
	));
	if let Some(description) = description {
		html.push_str(&format!(
			r#"<p class="reddit-subreddit-card__description">{}</p>"#,
			encode_text(&description)
		));
	}
	html.push_str(r#"<dl class="reddit-subreddit-card__stats">"#);
	push_reddit_stat(&mut html, "Members", subscribers);
	push_reddit_stat(&mut html, "Online", active);
	html.push_str("</dl></div></div>");
	Some(html)
}

fn push_reddit_stat(html: &mut String, label: &str, value: Option<u64>) {
	if let Some(value) = value {
		html.push_str(&format!(
			r#"<div><dt>{}</dt><dd>{}</dd></div>"#,
			encode_text(label),
			format_count(value)
		));
	}
}

fn reddit_embed_html(url: &str) -> String {
	format!(
		r#"<div class="embed embed-reddit"><blockquote class="reddit-embed-bq"><a href="{}" rel="noopener">View Reddit post</a></blockquote><script async src="https://embed.reddit.com/widgets.js" charset="UTF-8"></script></div>"#,
		encode_double_quoted_attribute(url)
	)
}

fn reddit_post_alive(url: &Url) -> Option<bool> {
	let status = reddit_post_oembed_status(url)?;
	if matches!(status, 200..=299) {
		return Some(true);
	}
	if definitely_broken_status(status) {
		return Some(false);
	}
	reddit_post_alive_from_json(url)
}

fn reddit_post_oembed_status(url: &Url) -> Option<u16> {
	reddit_post_json_url(url)?;
	Some(
		metadata_client()?
			.get("https://www.reddit.com/oembed")
			.query(&[("url", url.as_str())])
			.send()
			.ok()?
			.status()
			.as_u16(),
	)
}

fn reddit_post_alive_from_json(url: &Url) -> Option<bool> {
	let json_url = reddit_post_json_url(url)?;
	let client = metadata_client()?;
	let response: Value = safe_get(&client, &json_url)?
		.query(&[("raw_json", "1")])
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	let post = response
		.as_array()?
		.first()?
		.get("data")?
		.get("children")?
		.as_array()?
		.first()?
		.get("data")?;
	let title_exists = json_string(&post["title"]).is_some();
	let author = json_string(&post["author"]).unwrap_or_default();
	let removed = post["removed_by_category"].is_string()
		|| post["banned_by"].is_string()
		|| author == "[deleted]";
	Some(title_exists && !removed)
}

fn reddit_post_json_url(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if !matches!(
		host.as_str(),
		"reddit.com" | "old.reddit.com" | "new.reddit.com"
	) {
		return None;
	}
	let parts = decoded_path_segments(url);
	let comments_index = parts.iter().position(|part| part == "comments")?;
	let id = parts.get(comments_index + 1)?;
	if !is_reddit_id(id) {
		return None;
	}
	if comments_index >= 2 && parts.first().is_some_and(|part| part == "r") {
		let subreddit = parts.get(1)?;
		if !is_reddit_subreddit(subreddit) {
			return None;
		}
		return Some(format!(
			"https://www.reddit.com/r/{subreddit}/comments/{id}.json"
		));
	}
	Some(format!("https://www.reddit.com/comments/{id}.json"))
}

fn reddit_subreddit(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if !matches!(
		host.as_str(),
		"reddit.com" | "old.reddit.com" | "new.reddit.com"
	) {
		return None;
	}
	let parts = decoded_path_segments(url)
		.into_iter()
		.filter(|part| !part.is_empty())
		.collect::<Vec<_>>();
	if parts.first().map(String::as_str) != Some("r") || parts.len() != 2 {
		return None;
	}
	let subreddit = parts.get(1)?;
	is_reddit_subreddit(subreddit).then(|| subreddit.to_string())
}

fn is_reddit_id(value: &str) -> bool {
	!value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_reddit_subreddit(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 21
		&& value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn mastodon_embed(url: &Url) -> Option<String> {
	ssrf_safe_url(url).then_some(())?;
	if mastodon_status_url(url) {
		return mastodon_post_embed(url);
	}
	mastodon_profile_account(url).and_then(|acct| mastodon_profile_card(url, &acct))
}

fn mastodon_supported_url(url: &Url) -> bool {
	mastodon_status_url(url) || mastodon_profile_account(url).is_some()
}

fn mastodon_status_url(url: &Url) -> bool {
	let parts = decoded_path_segments(url);
	matches!(
		parts.as_slice(),
		[first, second]
			if first.starts_with('@') && second.chars().all(|c| c.is_ascii_digit())
	) || matches!(
		parts.as_slice(),
		[first, _, third, status]
			if first == "users" && third == "statuses" && status.chars().all(|c| c.is_ascii_digit())
	)
}

fn mastodon_profile_account(url: &Url) -> Option<String> {
	let parts = decoded_path_segments(url);
	let account = match parts.as_slice() {
		[account] => account.strip_prefix('@')?,
		[first, account] if first == "users" => account,
		_ => return None,
	};
	(!account.is_empty()
		&& account
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@')))
	.then(|| account.to_string())
}

fn mastodon_post_embed(url: &Url) -> Option<String> {
	let embed_url = cached_widget_html(&format!("mastodon-post:{}", url.as_str()), || {
		mastodon_oembed_embed_url(url).or_else(|| mastodon_status_embed_url(url))
	})?;
	Some(format!(
		r#"<div class="embed embed-mastodon-post"><iframe src="{}" title="Mastodon post" loading="lazy"></iframe><a href="{}" rel="noopener">View on Mastodon</a></div>"#,
		encode_double_quoted_attribute(&embed_url),
		encode_double_quoted_attribute(url.as_str())
	))
}

fn mastodon_oembed_embed_url(url: &Url) -> Option<String> {
	let response = mastodon_oembed_json(url)?;
	let html = response["html"].as_str()?;
	mastodon_embed_url_from_oembed_html(url, html).or_else(|| mastodon_status_embed_url(url))
}

fn mastodon_embed_url_from_oembed_html(url: &Url, html: &str) -> Option<String> {
	let embed_url = first_capture(
		html,
		r#"(?is)\bdata-embed-url\s*=\s*(?:"([^"]+)"|'([^']+)')"#,
	)
	.map(|value| decode_html_entities(&value).to_string())?;
	same_host_url(url, &embed_url)
}

fn mastodon_status_embed_url(url: &Url) -> Option<String> {
	let mut embed = url.clone();
	embed.set_query(None);
	embed.set_fragment(None);
	let path = format!("{}/embed", embed.path().trim_end_matches('/'));
	embed.set_path(&path);
	Some(embed.to_string())
}

fn mastodon_oembed_json(url: &Url) -> Option<Value> {
	let endpoint = mastodon_api_endpoint(url, "/api/oembed")?;
	let client = metadata_client()?;
	safe_get(&client, &endpoint)?
		.query(&[("url", url.as_str())])
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())
}

fn mastodon_profile_card(url: &Url, acct: &str) -> Option<String> {
	cached_widget_html(&format!("mastodon-profile:{}", url.as_str()), || {
		let account = mastodon_account(url, acct)?;
		mastodon_profile_card_from_account(url.as_str(), &account)
	})
}

fn mastodon_account(url: &Url, acct: &str) -> Option<Value> {
	let endpoint = mastodon_api_endpoint(url, "/api/v1/accounts/lookup")?;
	let client = metadata_client()?;
	safe_get(&client, &endpoint)?
		.query(&[("acct", acct)])
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())
}

fn mastodon_profile_card_from_account(fallback_url: &str, account: &Value) -> Option<String> {
	let url = json_string(&account["url"]).unwrap_or_else(|| fallback_url.to_string());
	let display_name = json_string(&account["display_name"])
		.filter(|name| !name.trim().is_empty())
		.or_else(|| json_string(&account["username"]))
		.or_else(|| json_string(&account["acct"]))?;
	let handle = mastodon_handle(&url, account)?;
	let avatar = json_string(&account["avatar_static"]).or_else(|| json_string(&account["avatar"]));
	let note =
		json_string(&account["note"]).and_then(|note| compact_title_text(&strip_html(&note)));
	let statuses = json_u64(&account["statuses_count"]);
	let followers = json_u64(&account["followers_count"]);
	let following = json_u64(&account["following_count"]);
	let mut html = String::new();
	html.push_str(r#"<div class="embed mastodon-profile-card">"#);
	if let Some(avatar) = avatar {
		html.push_str(&format!(
			r#"<a class="mastodon-profile-card__avatar" href="{}" rel="noopener"><img src="{}" alt=""></a>"#,
			encode_double_quoted_attribute(&url),
			encode_double_quoted_attribute(&avatar)
		));
	}
	html.push_str(r#"<div class="mastodon-profile-card__body">"#);
	html.push_str(&format!(
		r#"<a class="mastodon-profile-card__name" href="{}" rel="noopener"><strong>{}</strong><span>{}</span></a>"#,
		encode_double_quoted_attribute(&url),
		encode_text(&display_name),
		encode_text(&handle)
	));
	if let Some(note) = note {
		html.push_str(&format!(
			r#"<p class="mastodon-profile-card__note">{}</p>"#,
			encode_text(&note)
		));
	}
	html.push_str(r#"<dl class="mastodon-profile-card__stats">"#);
	push_mastodon_stat(&mut html, "Posts", statuses);
	push_mastodon_stat(&mut html, "Followers", followers);
	push_mastodon_stat(&mut html, "Following", following);
	html.push_str("</dl></div></div>");
	Some(html)
}

fn mastodon_handle(url: &str, account: &Value) -> Option<String> {
	let acct = json_string(&account["acct"]).or_else(|| json_string(&account["username"]))?;
	if acct.contains('@') {
		return Some(format!("@{acct}"));
	}
	let host = Url::parse(url).ok().and_then(|url| normalized_host(&url));
	Some(
		host.map(|host| format!("@{acct}@{host}"))
			.unwrap_or_else(|| format!("@{acct}")),
	)
}

fn push_mastodon_stat(html: &mut String, label: &str, value: Option<u64>) {
	if let Some(value) = value {
		html.push_str(&format!(
			r#"<div><dt>{}</dt><dd>{}</dd></div>"#,
			encode_text(label),
			format_count(value)
		));
	}
}

fn format_count(value: u64) -> String {
	if value >= 1_000_000 {
		format!("{:.1}M", value as f64 / 1_000_000.0)
	} else if value >= 1_000 {
		format!("{:.1}K", value as f64 / 1_000.0)
	} else {
		value.to_string()
	}
}

fn mastodon_api_endpoint(url: &Url, path: &str) -> Option<String> {
	Some(format!("https://{}{}", mastodon_host(url)?, path))
}

fn mastodon_host(url: &Url) -> Option<String> {
	Some(
		url.host_str()?
			.trim_start_matches("www.")
			.to_ascii_lowercase(),
	)
}

fn same_host_url(original: &Url, candidate: &str) -> Option<String> {
	let parsed = Url::parse(candidate).ok()?;
	(matches!(parsed.scheme(), "http" | "https")
		&& ssrf_safe_url(&parsed)
		&& normalized_host(&parsed) == normalized_host(original))
	.then(|| parsed.to_string())
}

fn shortcode_arg(value: &str) -> String {
	value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Expand bare URL lines into widget shortcodes; Markdown links are left alone.
pub fn expand_bare_links(markdown: &str, enabled: bool) -> String {
	expand_bare_links_with_disabled(markdown, enabled, &Default::default())
}

/// Expand bare URL lines while skipping widget providers disabled by config.
pub fn expand_bare_links_with_disabled(
	markdown: &str,
	enabled: bool,
	disabled_providers: &std::collections::HashSet<String>,
) -> String {
	expand_bare_links_with(markdown, enabled, |url, fallback| {
		expand_standalone_url_with_disabled(url, fallback, disabled_providers)
	})
}

fn expand_bare_links_with<F>(markdown: &str, enabled: bool, expand_url: F) -> String
where
	F: Fn(&str, &str) -> String,
{
	if !enabled {
		return markdown.to_string();
	}
	let markdown = expand_rich_link_blocks_with(markdown, &expand_url);
	let paragraph_url = Regex::new(r#"(?is)<p>\s*(https?://[^<\s]+)\s*</p>"#).unwrap();
	let markdown = paragraph_url
		.replace_all(&markdown, |caps: &regex::Captures| {
			let url = clean_url(caps.get(1).unwrap().as_str());
			expand_url_and_keep_source(url, caps.get(0).unwrap().as_str(), &expand_url)
		})
		.into_owned();
	let url_line = Regex::new(r"^\s*(https?://\S+)\s*$").unwrap();
	markdown
		.lines()
		.map(|line| {
			if let Some(caps) = url_line.captures(line) {
				let url = clean_url(caps.get(1).unwrap().as_str());
				expand_url_and_keep_source(url, line, &expand_url)
			} else {
				line.to_string()
			}
		})
		.collect::<Vec<_>>()
		.join("\n")
}

fn expand_url_and_keep_source<F>(url: &str, fallback: &str, expand_url: &F) -> String
where
	F: Fn(&str, &str) -> String,
{
	let expanded = expand_url(url, fallback);
	if widget_replaced_source(&expanded, fallback) {
		format!("{expanded}\n{}", original_link_html(url))
	} else {
		expanded
	}
}

fn widget_replaced_source(expanded: &str, fallback: &str) -> bool {
	expanded != fallback
		&& !expanded.contains("class=\"embed-link\"")
		&& !expanded.contains("class=embed-link")
		&& !expanded.contains("class=\"broken-link\"")
		&& !expanded.contains("class=broken-link")
}

fn original_link_html(url: &str) -> String {
	format!(
		r#"<p class="embed-link embed-link--source"><a href="{}" rel="noopener">Original link</a></p>"#,
		encode_double_quoted_attribute(url)
	)
}

fn expand_standalone_url_with_disabled(
	url: &str,
	fallback: &str,
	disabled_providers: &std::collections::HashSet<String>,
) -> String {
	if let Ok(parsed) = Url::parse(url)
		&& let Some(problem) = link_problem(&parsed)
	{
		return broken_link_html(url, &problem);
	}
	Url::parse(url)
		.ok()
		.and_then(|parsed| direct_media_embed(&parsed))
		.or_else(|| {
			detect(url).and_then(|widget| {
				(!disabled_providers.contains(widget.provider.config_key()))
					.then_some(widget.shortcode)
			})
		})
		.unwrap_or_else(|| fallback.to_string())
}

fn direct_media_embed(url: &Url) -> Option<String> {
	if commons_file_page_title(url).is_some() {
		return commons_file_direct_media_embed(url);
	}
	let kind = direct_media_kind_from_url(url)?;
	Some(direct_media_embed_html(url.as_str(), kind))
}

/// Classify direct media URLs by browser-playable filename extension.
fn direct_media_kind_from_url(url: &Url) -> Option<DirectMediaKind> {
	direct_media_kind_from_filename(url.path())
}

/// Classify a filename or wiki file title by browser-playable media extension.
fn direct_media_kind_from_filename(filename: &str) -> Option<DirectMediaKind> {
	let filename = filename.to_ascii_lowercase();
	let extension = filename.rsplit_once('.')?.1;
	match extension {
		"aac" | "flac" | "m4a" | "mid" | "midi" | "mp3" | "oga" | "ogg" | "opus" | "wav"
		| "weba" => Some(DirectMediaKind::Audio),
		"m4v" | "mkv" | "mov" | "mp4" | "ogv" | "webm" => Some(DirectMediaKind::Video),
		_ => None,
	}
}

fn direct_media_embed_html(url: &str, kind: DirectMediaKind) -> String {
	let url_attr = encode_double_quoted_attribute(url);
	let label = match kind {
		DirectMediaKind::Audio => "Open audio",
		DirectMediaKind::Video => "Open video",
	};
	let fallback = format!(
		r#"<a href="{url_attr}" rel="noopener">{}</a>"#,
		encode_text(label)
	);
	match kind {
		DirectMediaKind::Audio => format!(
			r#"<div class="embed embed-direct-media"><audio controls preload="metadata"><source src="{url_attr}">{fallback}</audio></div>"#
		),
		DirectMediaKind::Video => format!(
			r#"<div class="embed embed-direct-media"><video controls preload="metadata" playsinline><source src="{url_attr}">{fallback}</video></div>"#
		),
	}
}

/// Link plain Wikidata item ids like `Q42` to Wikidata before title enrichment.
pub fn link_wikidata_ids(html: &str) -> String {
	let tag = Regex::new(r#"(?is)<[^>]+>"#).unwrap();
	let mut out = String::new();
	let mut last = 0;
	let mut skip_depth = 0usize;

	for matched in tag.find_iter(html) {
		let text = &html[last..matched.start()];
		if skip_depth == 0 {
			out.push_str(&link_wikidata_ids_in_text(text));
		} else {
			out.push_str(text);
		}

		let tag_text = matched.as_str();
		out.push_str(tag_text);
		update_wikidata_link_skip_depth(tag_text, &mut skip_depth);
		last = matched.end();
	}

	let text = &html[last..];
	if skip_depth == 0 {
		out.push_str(&link_wikidata_ids_in_text(text));
	} else {
		out.push_str(text);
	}
	out
}

fn link_wikidata_ids_in_text(text: &str) -> String {
	let shortcode = Regex::new(r#"(?s)\{\{.*?\}\}"#).unwrap();
	let mut out = String::new();
	let mut last = 0;
	for matched in shortcode.find_iter(text) {
		out.push_str(&link_wikidata_ids_in_plain_text(
			&text[last..matched.start()],
		));
		out.push_str(matched.as_str());
		last = matched.end();
	}
	out.push_str(&link_wikidata_ids_in_plain_text(&text[last..]));
	out
}

fn link_wikidata_ids_in_plain_text(text: &str) -> String {
	let id = Regex::new(r#"Q[1-9][0-9]*"#).unwrap();
	let mut out = String::new();
	let mut last = 0;

	for matched in id.find_iter(text) {
		if !wikidata_id_has_text_boundaries(text.as_bytes(), matched.start(), matched.end()) {
			continue;
		}
		out.push_str(&text[last..matched.start()]);
		let id = matched.as_str();
		out.push_str(&format!(
			r#"<a href="https://www.wikidata.org/wiki/{id}">{id}</a>"#
		));
		last = matched.end();
	}

	if last == 0 {
		return text.to_string();
	}
	out.push_str(&text[last..]);
	out
}

fn wikidata_id_has_text_boundaries(bytes: &[u8], start: usize, end: usize) -> bool {
	let before = start
		.checked_sub(1)
		.and_then(|index| bytes.get(index))
		.is_none_or(|byte| wikidata_id_boundary(*byte));
	let after = bytes
		.get(end)
		.is_none_or(|byte| wikidata_id_boundary(*byte));
	before && after
}

fn wikidata_id_boundary(byte: u8) -> bool {
	!byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'/' | b':' | b'-')
}

fn update_wikidata_link_skip_depth(tag: &str, skip_depth: &mut usize) {
	let Some(name) = html_tag_name(tag) else {
		return;
	};
	if !matches!(
		name.as_str(),
		"a" | "code" | "pre" | "script" | "style" | "textarea"
	) {
		return;
	}
	if html_closing_tag(tag) {
		*skip_depth = skip_depth.saturating_sub(1);
	} else if !html_self_closing_tag(tag) {
		*skip_depth += 1;
	}
}

fn html_tag_name(tag: &str) -> Option<String> {
	let inner = tag.trim_start_matches('<').trim_end_matches('>').trim();
	let inner = inner.strip_prefix('/').unwrap_or(inner).trim_start();
	if inner.starts_with(['!', '?']) {
		return None;
	}
	let end = inner
		.find(|c: char| c.is_whitespace() || c == '/')
		.unwrap_or(inner.len());
	let name = &inner[..end];
	(!name.is_empty()).then(|| name.to_ascii_lowercase())
}

fn html_closing_tag(tag: &str) -> bool {
	tag.trim_start_matches('<').trim_start().starts_with('/')
}

fn html_self_closing_tag(tag: &str) -> bool {
	tag.trim_end().ends_with("/>")
}

/// Add hover titles with fetched metadata to supported external links.
pub fn enrich_link_titles(html: &str) -> String {
	enrich_link_titles_with_status(html, link_title, link_problem)
}

#[cfg(test)]
fn enrich_link_titles_with<F>(html: &str, title_for_url: F) -> String
where
	F: Fn(&Url) -> Option<String>,
{
	enrich_link_titles_with_status(html, title_for_url, |_| None)
}

fn enrich_link_titles_with_status<F, G>(html: &str, title_for_url: F, problem_for_url: G) -> String
where
	F: Fn(&Url) -> Option<String>,
	G: Fn(&Url) -> Option<String>,
{
	let link = Regex::new(r#"(?is)<a\b([^>]*)>"#).unwrap();
	link.replace_all(html, |caps: &regex::Captures| {
		let attrs = caps.get(1).unwrap().as_str();
		let Some(href) = href_attr(attrs) else {
			return caps.get(0).unwrap().as_str().to_string();
		};
		let Ok(url) = Url::parse(&href) else {
			return caps.get(0).unwrap().as_str().to_string();
		};
		let problem = problem_for_url(&url);
		let mut attrs = problem
			.as_ref()
			.map(|_| add_class_attr(attrs, "broken-link"))
			.unwrap_or_else(|| attrs.to_string());
		if let Some(title) = problem.or_else(|| title_for_url(&url)) {
			attrs = set_title_attr(&attrs, &title);
		}
		format!("<a{attrs}>")
	})
	.into_owned()
}

fn link_title(url: &Url) -> Option<String> {
	if is_wikipedia_article_url(url) {
		return cached_url_title(url.as_str(), || wikipedia_summary(url));
	}
	if archive_identifier(url).is_some() {
		return cached_url_title(url.as_str(), || archive_org_title(url));
	}
	if musicbrainz_entity(url).is_some() {
		return cached_url_title(url.as_str(), || musicbrainz_title(url));
	}
	if github_file_raw_url(url).is_some() {
		return cached_url_title(url.as_str(), || github_file_title(url));
	}
	if gitlab_file_raw_url(url).is_some() {
		return cached_url_title(url.as_str(), || gitlab_file_title(url));
	}
	if github_repo(url).is_some() {
		return cached_url_title(url.as_str(), || github_repo_title(url));
	}
	if gitlab_repo(url).is_some() {
		return cached_url_title(url.as_str(), || gitlab_repo_title(url));
	}
	if wikidata_entity_id(url).is_some() {
		return cached_url_title(url.as_str(), || wikidata_title(url));
	}
	if rutracker_topic_id(url).is_some() {
		return cached_url_title(url.as_str(), || rutracker_title(url));
	}
	if commons_page_title(url).is_some() {
		return cached_url_title(url.as_str(), || commons_title(url));
	}
	if gentoo_package_atom(url).is_some() {
		return cached_url_title(url.as_str(), || gentoo_package_title(url));
	}
	if lastfm_track(url).is_some() {
		return cached_url_title(url.as_str(), || lastfm_title(url));
	}
	if mdn_doc(url) {
		return cached_url_title(url.as_str(), || mdn_title(url));
	}
	if livejournal_post(url) {
		return cached_url_title(url.as_str(), || livejournal_title(url));
	}
	if habr_user_profile(url).is_some() {
		return cached_url_title(url.as_str(), || habr_user_title(url));
	}
	if telegram_channel(url).is_some() {
		return cached_url_title(url.as_str(), || telegram_channel_title(url));
	}
	if google_drive_folder_id(url).is_some() {
		return cached_url_title(url.as_str(), || google_drive_folder_title(url));
	}
	if stackoverflow_question_id(url).is_some() {
		return cached_url_title(url.as_str(), || stackoverflow_title(url));
	}
	None
}

fn cached_url_title<F>(url: &str, fetch: F) -> Option<String>
where
	F: FnOnce() -> Option<String>,
{
	static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
	let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
	if let Some(cached) = cache.lock().ok()?.get(url).cloned() {
		return cached;
	}
	let title = fetch();
	cache.lock().ok()?.insert(url.to_string(), title.clone());
	title
}

fn cached_widget_html<F>(key: &str, fetch: F) -> Option<String>
where
	F: FnOnce() -> Option<String>,
{
	static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
	let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
	if let Some(cached) = cache.lock().ok()?.get(key).cloned() {
		return cached;
	}
	let html = fetch();
	cache.lock().ok()?.insert(key.to_string(), html.clone());
	html
}

fn link_problem(url: &Url) -> Option<String> {
	if !matches!(url.scheme(), "http" | "https") {
		return None;
	}
	if let Some(problem) = embeddable_link_problem(url) {
		return Some(problem);
	}
	let status = cached_link_status(url)?;
	definitely_broken_status(status).then(|| format!("Broken link: HTTP {status}"))
}

fn cached_link_status(url: &Url) -> Option<u16> {
	cached_probe_status(url.as_str(), || fetch_link_status(url))
}

fn cached_probe_status<F>(key: &str, fetch: F) -> Option<u16>
where
	F: FnOnce() -> Option<u16>,
{
	static CACHE: OnceLock<Mutex<HashMap<String, Option<u16>>>> = OnceLock::new();
	let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
	if let Some(cached) = cache.lock().ok()?.get(key).cloned() {
		return cached;
	}
	let status = fetch();
	cache.lock().ok()?.insert(key.to_string(), status);
	status
}

fn cached_probe_bool<F>(key: &str, fetch: F) -> Option<bool>
where
	F: FnOnce() -> Option<bool>,
{
	static CACHE: OnceLock<Mutex<HashMap<String, Option<bool>>>> = OnceLock::new();
	let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
	if let Some(cached) = cache.lock().ok()?.get(key).cloned() {
		return cached;
	}
	let value = fetch();
	cache.lock().ok()?.insert(key.to_string(), value);
	value
}

fn fetch_link_status(url: &Url) -> Option<u16> {
	let client = metadata_client()?;
	let response = safe_head(&client, url)?.send().ok();
	if let Some(response) = response {
		let status = response.status();
		if !matches!(status.as_u16(), 405 | 403 | 429 | 500..=599) {
			return Some(status.as_u16());
		}
	}
	safe_get(&client, url.as_str())?
		.header(reqwest::header::RANGE, "bytes=0-0")
		.send()
		.ok()
		.map(|response| response.status().as_u16())
}

fn definitely_broken_status(status: u16) -> bool {
	matches!(status, 400 | 404 | 410 | 451)
}

fn embeddable_link_problem(url: &Url) -> Option<String> {
	vimeo_link_problem(url)
		.or_else(|| spotify_link_problem(url))
		.or_else(|| soundcloud_link_problem(url))
		.or_else(|| apple_podcast_link_problem(url))
		.or_else(|| reddit_link_problem(url))
		.or_else(|| mastodon_link_problem(url))
}

fn vimeo_link_problem(url: &Url) -> Option<String> {
	vimeo_video_id(url)?;
	let status = oembed_status(
		&format!("vimeo-oembed:{}", url.as_str()),
		"https://vimeo.com/api/oembed.json",
		&[("url", url.as_str())],
	)?;
	provider_status_problem("Vimeo video", status)
}

fn spotify_link_problem(url: &Url) -> Option<String> {
	spotify_embed(url)?;
	let status = oembed_status(
		&format!("spotify-oembed:{}", url.as_str()),
		"https://open.spotify.com/oembed",
		&[("url", url.as_str())],
	)?;
	provider_status_problem("Spotify link", status)
}

fn soundcloud_link_problem(url: &Url) -> Option<String> {
	soundcloud_url(url).then_some(())?;
	let status = oembed_status(
		&format!("soundcloud-oembed:{}", url.as_str()),
		"https://soundcloud.com/oembed",
		&[("format", "json"), ("url", url.as_str())],
	)?;
	provider_status_problem("SoundCloud link", status)
}

fn apple_podcast_link_problem(url: &Url) -> Option<String> {
	let id = apple_podcast_id(url)?;
	let removed = cached_probe_bool(&format!("apple-podcast:{id}"), || {
		apple_podcast_removed(&id)
	})?;
	removed.then(|| "Broken Apple Podcasts link: podcast not found".to_string())
}

fn mastodon_link_problem(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if !looks_like_mastodon(&host) || !mastodon_status_url(url) || !ssrf_safe_url(url) {
		return None;
	}
	let endpoint = mastodon_api_endpoint(url, "/api/oembed")?;
	let status = oembed_status(
		&format!("mastodon-oembed:{}", url.as_str()),
		&endpoint,
		&[("url", url.as_str())],
	)?;
	provider_status_problem("Mastodon post", status)
}

fn oembed_status(cache_key: &str, endpoint: &str, query: &[(&str, &str)]) -> Option<u16> {
	cached_probe_status(cache_key, || {
		let client = metadata_client()?;
		safe_get(&client, endpoint)?
			.query(query)
			.send()
			.ok()
			.map(|response| response.status().as_u16())
	})
}

fn provider_status_problem(provider: &str, status: u16) -> Option<String> {
	definitely_broken_status(status).then(|| format!("Broken {provider}: HTTP {status}"))
}

fn apple_podcast_removed(id: &str) -> Option<bool> {
	let response: Value = metadata_client()?
		.get("https://itunes.apple.com/lookup")
		.query(&[("id", id)])
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	json_u64(&response["resultCount"]).map(|count| count == 0)
}

fn reddit_link_problem(url: &Url) -> Option<String> {
	reddit_post_json_url(url)?;
	let alive = cached_probe_bool(&format!("reddit-post-alive:{}", url.as_str()), || {
		reddit_post_alive(url)
	})?;
	(!alive).then(|| "Broken Reddit post: deleted, removed, or not public".to_string())
}

fn broken_link_html(url: &str, problem: &str) -> String {
	format!(
		r#"<p><a class="broken-link" href="{}" title="{}">{}</a></p>"#,
		encode_double_quoted_attribute(url),
		encode_double_quoted_attribute(problem),
		encode_text(url)
	)
}

fn wikipedia_summary(url: &Url) -> Option<String> {
	let title = wikipedia_article_title(url)?;
	let client = metadata_client()?;
	let api_host = wikipedia_api_host(url)?;
	let response: Value = client
		.get(format!("https://{api_host}/w/api.php"))
		.query(&[
			("action", "query"),
			("prop", "extracts|revisions"),
			("exintro", "1"),
			("explaintext", "1"),
			("rvdir", "newer"),
			("rvlimit", "1"),
			("rvprop", "timestamp"),
			("redirects", "1"),
			("format", "json"),
			("formatversion", "2"),
			("titles", title.as_str()),
		])
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	response["query"]["pages"]
		.as_array()?
		.iter()
		.find_map(|page| {
			if page["missing"].as_bool().unwrap_or_default() {
				return None;
			}
			wikipedia_title_from_page(page)
		})
}

fn wikipedia_title_from_page(page: &Value) -> Option<String> {
	let mut lines = Vec::new();
	if let Some(summary) = page["extract"].as_str().and_then(compact_title_text) {
		lines.push(summary);
	}
	if let Some(created) = page["revisions"].as_array().and_then(|revisions| {
		revisions
			.first()
			.and_then(|revision| json_string(&revision["timestamp"]))
	}) {
		if !lines.is_empty() {
			lines.push(String::new());
		}
		lines.push(format!("Page created: {created}"));
	}
	(!lines.is_empty()).then(|| lines.join("\n"))
}

fn wikipedia_api_host(url: &Url) -> Option<String> {
	let host = url
		.host_str()?
		.trim_start_matches("www.")
		.to_ascii_lowercase();
	if let Some(language) = host.strip_suffix(".m.wikipedia.org") {
		return Some(format!("{language}.wikipedia.org"));
	}
	(host.ends_with(".wikipedia.org")).then_some(host)
}

fn is_wikipedia_article_url(url: &Url) -> bool {
	wikipedia_article_title(url).is_some()
}

fn wikipedia_article_title(url: &Url) -> Option<String> {
	wikipedia_api_host(url)?;
	let path = url.path();
	let encoded_title = path.strip_prefix("/wiki/")?;
	if encoded_title.is_empty() {
		return None;
	}
	let title = percent_decode_str(encoded_title)
		.decode_utf8()
		.ok()?
		.replace('_', " ");
	if commons_namespace_is_skipped(&title) {
		return None;
	}
	Some(title)
}

fn commons_namespace_is_skipped(title: &str) -> bool {
	let Some((namespace, _)) = title.split_once(':') else {
		return false;
	};
	matches!(
		namespace.to_ascii_lowercase().as_str(),
		"special" | "help" | "talk" | "template" | "user" | "commons" | "module" | "timedtext"
	)
}

fn commons_title(url: &Url) -> Option<String> {
	let title = commons_page_title(url)?;
	if commons_file_title_prefix(&title) {
		return commons_file_title(&title);
	}
	commons_regular_page_title(&title)
}

/// Extract the Commons `File:` page title for file pages only.
fn commons_file_page_title(url: &Url) -> Option<String> {
	let title = commons_page_title(url)?;
	commons_file_title_prefix(&title).then_some(title)
}

fn commons_file_title_prefix(title: &str) -> bool {
	title.starts_with("File:")
}

fn commons_page_title(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if host != "commons.wikimedia.org" {
		return None;
	}
	let encoded_title = url.path().strip_prefix("/wiki/")?;
	if encoded_title.is_empty() {
		return None;
	}
	let title = percent_decode_str(encoded_title)
		.decode_utf8()
		.ok()?
		.replace('_', " ");
	if commons_namespace_is_skipped(&title) {
		return None;
	}
	Some(title)
}

fn commons_file_title(title: &str) -> Option<String> {
	let client = metadata_client()?;
	let response: Value = client
		.get("https://commons.wikimedia.org/w/api.php")
		.query(&[
			("action", "query"),
			("prop", "imageinfo"),
			("iiprop", "extmetadata|mime|size"),
			("format", "json"),
			("formatversion", "2"),
			("titles", title),
		])
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	let page = response["query"]["pages"].as_array()?.first()?;
	commons_file_title_from_page(title, page)
}

/// Convert a Wikimedia Commons `File:` page URL into a browser-playable media embed.
fn commons_file_direct_media_embed(url: &Url) -> Option<String> {
	let title = commons_file_page_title(url)?;
	let kind = direct_media_kind_from_filename(&title)?;
	let media_url = commons_file_media_url(&title)?;
	Some(direct_media_embed_html(&media_url, kind))
}

/// Fetch the original media URL for a Commons `File:` title through the API.
fn commons_file_media_url(title: &str) -> Option<String> {
	let client = metadata_client()?;
	let response: Value = client
		.get("https://commons.wikimedia.org/w/api.php")
		.query(&[
			("action", "query"),
			("prop", "imageinfo"),
			("iiprop", "url"),
			("format", "json"),
			("formatversion", "2"),
			("titles", title),
		])
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	let page = response["query"]["pages"].as_array()?.first()?;
	let url = commons_file_media_url_from_page(page)?;
	let parsed = Url::parse(&url).ok()?;
	ssrf_safe_url(&parsed).then(|| parsed.to_string())
}

/// Extract the API `imageinfo.url` field for a Commons file page.
fn commons_file_media_url_from_page(page: &Value) -> Option<String> {
	page["imageinfo"]
		.as_array()?
		.first()
		.and_then(|info| json_string(&info["url"]))
}

fn commons_file_title_from_page(title: &str, page: &Value) -> Option<String> {
	let info = page["imageinfo"].as_array()?.first()?;
	let metadata = &info["extmetadata"];
	let mut lines = vec![format!("Wikimedia Commons file: {title}")];
	if let Some(description) = commons_metadata_value(metadata, "ImageDescription") {
		lines.push(format!("Description: {description}"));
	}
	if let Some(author) = commons_metadata_value(metadata, "Artist") {
		lines.push(format!("Author: {author}"));
	}
	if let Some(date) = commons_metadata_value(metadata, "DateTimeOriginal")
		.or_else(|| commons_metadata_value(metadata, "DateTime"))
	{
		lines.push(format!("Date: {date}"));
	}
	if let Some(license) = commons_metadata_value(metadata, "LicenseShortName")
		.or_else(|| commons_metadata_value(metadata, "UsageTerms"))
	{
		lines.push(format!("License: {license}"));
	}
	if let (Some(width), Some(height)) = (json_u64(&info["width"]), json_u64(&info["height"])) {
		lines.push(format!("Dimensions: {width} x {height}"));
	}
	if let Some(mime) = json_string(&info["mime"]) {
		lines.push(format!("MIME: {mime}"));
	}
	if let Some(size) = json_u64(&info["size"]) {
		lines.push(format!("Size: {}", format_bytes(size)));
	}
	Some(lines.join("\n"))
}

fn commons_metadata_value(metadata: &Value, key: &str) -> Option<String> {
	json_string(&metadata[key]["value"])
		.map(|value| strip_html(&value))
		.and_then(|value| compact_title_text(&value))
}

fn commons_regular_page_title(title: &str) -> Option<String> {
	let client = metadata_client()?;
	let response: Value = client
		.get("https://commons.wikimedia.org/w/api.php")
		.query(&[
			("action", "query"),
			("prop", "extracts|categoryinfo"),
			("exintro", "1"),
			("explaintext", "1"),
			("format", "json"),
			("formatversion", "2"),
			("titles", title),
		])
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	let page = response["query"]["pages"].as_array()?.first()?;
	commons_regular_page_title_from_page(title, page)
}

fn commons_regular_page_title_from_page(title: &str, page: &Value) -> Option<String> {
	let mut lines = vec![format!("Wikimedia Commons: {title}")];
	if let Some(extract) = page["extract"].as_str().and_then(compact_title_text) {
		lines.push(extract);
	}
	let category = &page["categoryinfo"];
	if let Some(files) = json_u64(&category["files"]) {
		lines.push(format!("Files: {files}"));
	}
	if let Some(pages) = json_u64(&category["pages"]) {
		lines.push(format!("Pages: {pages}"));
	}
	if let Some(subcategories) = json_u64(&category["subcats"]) {
		lines.push(format!("Subcategories: {subcategories}"));
	}
	Some(lines.join("\n"))
}

fn archive_org_title(url: &Url) -> Option<String> {
	let identifier = archive_identifier(url)?;
	let client = metadata_client()?;
	let response: Value = client
		.get(format!("https://archive.org/metadata/{identifier}"))
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	archive_org_title_from_metadata(url, &response)
}

fn archive_org_title_from_metadata(url: &Url, metadata: &Value) -> Option<String> {
	let mut lines = Vec::new();
	if let Some(size) =
		archive_file_size(url, metadata).or_else(|| json_u64(&metadata["item_size"]))
	{
		lines.push(format!("Size: {}", format_bytes(size)));
	}
	if let Some(date) = json_string(&metadata["metadata"]["publicdate"])
		.or_else(|| json_string(&metadata["metadata"]["date"]))
		.or_else(|| json_string(&metadata["metadata"]["addeddate"]))
		.or_else(|| json_u64(&metadata["created"]).map(|timestamp| timestamp.to_string()))
	{
		lines.push(format!("Publication date: {date}"));
	}
	if let Some(uploader) = json_string(&metadata["metadata"]["uploader"])
		.or_else(|| json_string(&metadata["metadata"]["creator"]))
	{
		lines.push(format!("Uploaded by: {uploader}"));
	}
	(!lines.is_empty()).then(|| lines.join("\n"))
}

fn archive_file_size(url: &Url, metadata: &Value) -> Option<u64> {
	let file_name = archive_file_name(url)?;
	metadata["files"].as_array()?.iter().find_map(|file| {
		(file["name"].as_str()? == file_name)
			.then(|| json_u64(&file["size"]))
			.flatten()
	})
}

fn archive_identifier(url: &Url) -> Option<String> {
	let host = url
		.host_str()?
		.trim_start_matches("www.")
		.to_ascii_lowercase();
	if host != "archive.org" {
		return None;
	}
	let parts = decoded_path_segments(url);
	let marker = parts.first()?;
	matches!(
		marker.as_str(),
		"details" | "download" | "embed" | "stream" | "metadata"
	)
	.then(|| parts.get(1).cloned())
	.flatten()
	.filter(|identifier| !identifier.is_empty())
}

fn archive_file_name(url: &Url) -> Option<String> {
	let parts = decoded_path_segments(url);
	(parts.first().is_some_and(|part| part == "download") && parts.len() > 2)
		.then(|| parts[2..].join("/"))
}

fn decoded_path_segments(url: &Url) -> Vec<String> {
	url.path_segments()
		.into_iter()
		.flatten()
		.filter_map(|segment| percent_decode_str(segment).decode_utf8().ok())
		.map(|segment| segment.to_string())
		.collect()
}

fn json_string(value: &Value) -> Option<String> {
	match value {
		Value::String(value) => Some(value.trim().to_string()).filter(|value| !value.is_empty()),
		Value::Array(values) => values.iter().find_map(json_string),
		_ => None,
	}
}

fn json_u64(value: &Value) -> Option<u64> {
	match value {
		Value::Number(number) => number.as_u64(),
		Value::String(value) => value.parse().ok(),
		_ => None,
	}
}

fn json_scalar_text(value: &Value) -> Option<String> {
	match value {
		Value::Number(number) => Some(number.to_string()),
		Value::String(value) => compact_title_text(value),
		_ => None,
	}
	.filter(|value| !value.is_empty())
}

fn compact_title_text(text: &str) -> Option<String> {
	let mut compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
	if compact.is_empty() {
		return None;
	}
	const MAX_CHARS: usize = 700;
	if compact.chars().count() > MAX_CHARS {
		let mut trimmed = compact.chars().take(MAX_CHARS - 3).collect::<String>();
		trimmed = trimmed.trim_end().to_string();
		trimmed.push_str("...");
		compact = trimmed;
	}
	Some(compact)
}

fn format_bytes(size: u64) -> String {
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

fn metadata_client() -> Option<Client> {
	Client::builder()
		.timeout(Duration::from_secs(10))
		.redirect(Policy::none())
		.user_agent(
			"Everpublich/0.4 (https://github.com/vitaly-zdanevich/everpublich; zdanevich.vitaly@ya.ru)",
		)
		.build()
		.ok()
}

/// Build a GET request only after the URL passes the outbound SSRF guard.
fn safe_get(client: &Client, url: &str) -> Option<RequestBuilder> {
	let parsed = Url::parse(url).ok()?;
	ssrf_safe_url(&parsed).then(|| client.get(url))
}

/// Build a HEAD request only after the URL passes the outbound SSRF guard.
fn safe_head(client: &Client, url: &Url) -> Option<RequestBuilder> {
	ssrf_safe_url(url).then(|| client.head(url.as_str()))
}

/// Keep server-side metadata fetches away from local, private, and cloud
/// metadata addresses. Notes are user-controlled, so link probes must fail
/// closed unless the target is a public HTTP(S) endpoint.
fn ssrf_safe_url(url: &Url) -> bool {
	if !matches!(url.scheme(), "http" | "https") {
		return false;
	}
	let Some(host) = url.host() else {
		return false;
	};
	let host = match host {
		Host::Ipv4(ip) => return public_ipv4_for_outbound(ip),
		Host::Ipv6(ip) => return public_ipv6_for_outbound(ip),
		Host::Domain(host) => host,
	};
	if local_host_name(host) {
		return false;
	}
	let Some(port) = url.port_or_known_default() else {
		return false;
	};
	let Ok(addresses) = (host, port).to_socket_addrs() else {
		return false;
	};
	let addresses = addresses.collect::<Vec<_>>();
	!addresses.is_empty()
		&& addresses
			.iter()
			.all(|address| public_ip_for_outbound(address.ip()))
}

fn local_host_name(host: &str) -> bool {
	let host = host.trim_end_matches('.').to_ascii_lowercase();
	matches!(host.as_str(), "localhost" | "localhost.localdomain") || host.ends_with(".localhost")
}

fn public_ip_for_outbound(ip: IpAddr) -> bool {
	match ip {
		IpAddr::V4(ip) => public_ipv4_for_outbound(ip),
		IpAddr::V6(ip) => public_ipv6_for_outbound(ip),
	}
}

fn public_ipv4_for_outbound(ip: Ipv4Addr) -> bool {
	let [a, b, c, d] = ip.octets();
	if a == 0
		|| a == 10
		|| a == 127
		|| a >= 224
		|| (a == 100 && (64..=127).contains(&b))
		|| (a == 169 && b == 254)
		|| (a == 172 && (16..=31).contains(&b))
		|| (a == 192 && b == 168)
		|| (a == 192 && b == 0 && c == 0)
		|| (a == 192 && b == 0 && c == 2)
		|| (a == 198 && (b == 18 || b == 19))
		|| (a == 198 && b == 51 && c == 100)
		|| (a == 203 && b == 0 && c == 113)
	{
		return false;
	}
	!(a == 255 && b == 255 && c == 255 && d == 255)
}

fn public_ipv6_for_outbound(ip: Ipv6Addr) -> bool {
	if let Some(mapped) = ipv4_mapped_ipv6(ip) {
		return public_ipv4_for_outbound(mapped);
	}
	let segments = ip.segments();
	if ip.is_unspecified() || ip.is_loopback() {
		return false;
	}
	if segments[0] & 0xfe00 == 0xfc00 {
		return false;
	}
	if segments[0] & 0xffc0 == 0xfe80 {
		return false;
	}
	if segments[0] & 0xff00 == 0xff00 {
		return false;
	}
	if segments[0] == 0x2001 && segments[1] == 0x0db8 {
		return false;
	}
	if segments[0] == 0x2002 {
		return false;
	}
	if segments[0] == 0x0100 && segments[1] == 0 {
		return false;
	}
	true
}

fn ipv4_mapped_ipv6(ip: Ipv6Addr) -> Option<Ipv4Addr> {
	let segments = ip.segments();
	if segments[..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff {
		Some(Ipv4Addr::new(
			(segments[6] >> 8) as u8,
			segments[6] as u8,
			(segments[7] >> 8) as u8,
			segments[7] as u8,
		))
	} else {
		None
	}
}

fn musicbrainz_title(url: &Url) -> Option<String> {
	let (entity, mbid) = musicbrainz_entity(url)?;
	let client = metadata_client()?;
	musicbrainz_wait_for_rate_limit();
	let response: Value = client
		.get(format!("https://musicbrainz.org/ws/2/{entity}/{mbid}"))
		.query(&musicbrainz_query(&entity))
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	musicbrainz_title_from_metadata(&entity, &response)
}

fn musicbrainz_query(entity: &str) -> Vec<(&'static str, &'static str)> {
	let inc = match entity {
		"artist" => Some("genres"),
		"release" => Some("artist-credits+labels+media+release-groups"),
		"release-group" => Some("artist-credits+genres"),
		"recording" => Some("artist-credits+releases+isrcs"),
		"label" | "work" => Some("genres"),
		_ => None,
	};
	let mut query = vec![("fmt", "json")];
	if let Some(inc) = inc {
		query.push(("inc", inc));
	}
	query
}

fn musicbrainz_wait_for_rate_limit() {
	static LAST_REQUEST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
	let lock = LAST_REQUEST.get_or_init(|| Mutex::new(None));
	let Ok(mut last_request) = lock.lock() else {
		return;
	};
	if let Some(last) = *last_request {
		let elapsed = last.elapsed();
		if elapsed < Duration::from_secs(1) {
			thread::sleep(Duration::from_secs(1) - elapsed);
		}
	}
	*last_request = Some(Instant::now());
}

fn musicbrainz_entity(url: &Url) -> Option<(String, String)> {
	let host = url
		.host_str()?
		.trim_start_matches("www.")
		.to_ascii_lowercase();
	if host != "musicbrainz.org" {
		return None;
	}
	let parts = decoded_path_segments(url);
	let entity = parts.first()?.to_ascii_lowercase();
	if !matches!(
		entity.as_str(),
		"area"
			| "artist"
			| "event" | "instrument"
			| "label" | "place"
			| "recording"
			| "release"
			| "release-group"
			| "series"
			| "work"
	) {
		return None;
	}
	let mbid = parts.get(1)?.to_ascii_lowercase();
	is_mbid(&mbid).then_some((entity, mbid))
}

fn is_mbid(value: &str) -> bool {
	static REGEX: OnceLock<Regex> = OnceLock::new();
	REGEX
		.get_or_init(|| {
			Regex::new(
				r#"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"#,
			)
			.unwrap()
		})
		.is_match(value)
}

fn musicbrainz_title_from_metadata(entity: &str, metadata: &Value) -> Option<String> {
	let mut lines = Vec::new();
	let label = musicbrainz_entity_label(entity);
	let primary = json_string(&metadata["title"]).or_else(|| json_string(&metadata["name"]))?;
	lines.push(format!("{label}: {primary}"));
	if let Some(artist) = musicbrainz_artist_credit(metadata) {
		lines.push(format!("Artist: {artist}"));
	}
	if let Some(entity_type) =
		json_string(&metadata["type"]).or_else(|| json_string(&metadata["primary-type"]))
	{
		lines.push(format!("Type: {entity_type}"));
	}
	if let Some(date) = json_string(&metadata["first-release-date"])
		.or_else(|| json_string(&metadata["date"]))
		.or_else(|| json_string(&metadata["life-span"]["begin"]))
		.or_else(|| json_string(&metadata["begin-date"]))
	{
		lines.push(format!("Date: {date}"));
	}
	if let Some(country) = json_string(&metadata["country"]) {
		lines.push(format!("Country: {country}"));
	}
	if let Some(status) = json_string(&metadata["status"]) {
		lines.push(format!("Status: {status}"));
	}
	if let Some(label) = musicbrainz_release_label(metadata) {
		lines.push(format!("Label: {label}"));
	}
	if let Some(track_count) = musicbrainz_track_count(metadata) {
		lines.push(format!("Tracks: {track_count}"));
	}
	if let Some(length) = json_u64(&metadata["length"]) {
		lines.push(format!("Length: {}", format_duration(length)));
	}
	if let Some(genres) = musicbrainz_genres(metadata) {
		lines.push(format!("Genres: {genres}"));
	}
	if let Some(disambiguation) = json_string(&metadata["disambiguation"]) {
		lines.push(format!("Disambiguation: {disambiguation}"));
	}
	Some(lines.join("\n"))
}

fn musicbrainz_entity_label(entity: &str) -> &'static str {
	match entity {
		"area" => "MusicBrainz area",
		"artist" => "MusicBrainz artist",
		"event" => "MusicBrainz event",
		"instrument" => "MusicBrainz instrument",
		"label" => "MusicBrainz label",
		"place" => "MusicBrainz place",
		"recording" => "MusicBrainz recording",
		"release" => "MusicBrainz release",
		"release-group" => "MusicBrainz release group",
		"series" => "MusicBrainz series",
		"work" => "MusicBrainz work",
		_ => "MusicBrainz item",
	}
}

fn musicbrainz_artist_credit(metadata: &Value) -> Option<String> {
	let credits = metadata["artist-credit"].as_array()?;
	let mut out = String::new();
	for credit in credits {
		let name =
			json_string(&credit["name"]).or_else(|| json_string(&credit["artist"]["name"]))?;
		out.push_str(&name);
		if let Some(joinphrase) = json_string(&credit["joinphrase"]) {
			out.push_str(&joinphrase);
		}
	}
	(!out.is_empty()).then_some(out)
}

fn musicbrainz_release_label(metadata: &Value) -> Option<String> {
	metadata["label-info"].as_array()?.iter().find_map(|info| {
		json_string(&info["label"]["name"]).or_else(|| json_string(&info["catalog-number"]))
	})
}

fn musicbrainz_track_count(metadata: &Value) -> Option<u64> {
	let media = metadata["media"].as_array()?;
	let count = media
		.iter()
		.filter_map(|medium| {
			json_u64(&medium["track-count"]).or_else(|| {
				medium["tracks"]
					.as_array()
					.and_then(|tracks| u64::try_from(tracks.len()).ok())
			})
		})
		.sum::<u64>();
	(count > 0).then_some(count)
}

fn musicbrainz_genres(metadata: &Value) -> Option<String> {
	let genres = metadata["genres"]
		.as_array()?
		.iter()
		.filter_map(|genre| json_string(&genre["name"]))
		.take(4)
		.collect::<Vec<_>>();
	(!genres.is_empty()).then(|| genres.join(", "))
}

fn format_duration(milliseconds: u64) -> String {
	let total_seconds = milliseconds / 1000;
	let minutes = total_seconds / 60;
	let seconds = total_seconds % 60;
	format!("{minutes}:{seconds:02}")
}

fn github_repo_title(url: &Url) -> Option<String> {
	let (owner, repo) = github_repo(url)?;
	let client = metadata_client()?;
	let response: Value = client
		.get(format!("https://api.github.com/repos/{owner}/{repo}"))
		.header("Accept", "application/vnd.github+json")
		.header("X-GitHub-Api-Version", "2026-03-10")
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	repo_title_from_metadata("GitHub", &response)
}

fn github_file_title(url: &Url) -> Option<String> {
	let (label, raw_url) = github_file_raw_url(url)?;
	source_file_title("GitHub file", &label, &raw_url)
}

fn github_file_raw_url(url: &Url) -> Option<(String, String)> {
	let host = normalized_host(url)?;
	let parts = decoded_path_segments(url);
	if host == "raw.githubusercontent.com" {
		let owner = parts.first()?;
		let repo = parts.get(1)?;
		let revision = parts.get(2)?;
		let file_parts = parts.get(3..)?;
		if file_parts.is_empty() {
			return None;
		}
		let file_path = file_parts.join("/");
		return Some((
			format!("{owner}/{repo}/{file_path}"),
			format!(
				"https://raw.githubusercontent.com/{}/{}/{}/{}",
				owner,
				repo,
				revision,
				file_parts
					.iter()
					.map(|part| utf8_percent_encode(part, PATH_SEGMENT_ENCODE).to_string())
					.collect::<Vec<_>>()
					.join("/")
			),
		));
	}
	if host != "github.com" {
		return None;
	}
	let owner = parts.first()?;
	let repo = parts.get(1)?;
	if parts.get(2).map(String::as_str) != Some("blob") {
		return None;
	}
	let revision = parts.get(3)?;
	let file_parts = parts.get(4..)?;
	if file_parts.is_empty() {
		return None;
	}
	let file_path = file_parts.join("/");
	Some((
		format!("{owner}/{repo}/{file_path}"),
		format!(
			"https://raw.githubusercontent.com/{}/{}/{}/{}",
			owner,
			repo,
			revision,
			file_parts
				.iter()
				.map(|part| utf8_percent_encode(part, PATH_SEGMENT_ENCODE).to_string())
				.collect::<Vec<_>>()
				.join("/")
		),
	))
}

fn github_repo(url: &Url) -> Option<(String, String)> {
	let host = normalized_host(url)?;
	if host != "github.com" {
		return None;
	}
	let parts = decoded_path_segments(url);
	let owner = parts.first()?.clone();
	let repo = parts.get(1)?.trim_end_matches(".git").to_string();
	is_repo_path_part(&owner).then_some(())?;
	is_repo_path_part(&repo).then_some(())?;
	Some((owner, repo))
}

fn source_file_title(provider: &str, label: &str, raw_url: &str) -> Option<String> {
	let client = metadata_client()?;
	let body = safe_get(&client, raw_url)?
		.header(reqwest::header::RANGE, "bytes=0-65535")
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()?;
	let preview = first_source_lines(&body, 100)?;
	Some(format!("{provider}: {label}\n\n{preview}"))
}

fn first_source_lines(body: &str, line_limit: usize) -> Option<String> {
	let mut preview = body.lines().take(line_limit).collect::<Vec<_>>().join("\n");
	if preview.trim().is_empty() {
		return None;
	}
	const MAX_CHARS: usize = 12_000;
	if preview.chars().count() > MAX_CHARS {
		let mut truncated = preview.chars().take(MAX_CHARS - 3).collect::<String>();
		truncated = truncated.trim_end().to_string();
		truncated.push_str("...");
		preview = truncated;
	}
	Some(preview)
}

fn gitlab_repo_title(url: &Url) -> Option<String> {
	let path = gitlab_repo(url)?;
	let client = metadata_client()?;
	let encoded_path = utf8_percent_encode(&path, PATH_SEGMENT_ENCODE).to_string();
	let response: Value = client
		.get(format!("https://gitlab.com/api/v4/projects/{encoded_path}"))
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	repo_title_from_metadata("GitLab", &response)
}

fn gitlab_file_title(url: &Url) -> Option<String> {
	let (label, raw_url) = gitlab_file_raw_url(url)?;
	source_file_title("GitLab file", &label, &raw_url)
}

fn gitlab_file_raw_url(url: &Url) -> Option<(String, String)> {
	let host = normalized_host(url)?;
	if host != "gitlab.com" {
		return None;
	}
	let parts = decoded_path_segments(url);
	let dash_index = parts.iter().position(|part| part == "-")?;
	let action = parts.get(dash_index + 1)?;
	if !matches!(action.as_str(), "blob" | "raw") {
		return None;
	}
	let revision = parts.get(dash_index + 2)?;
	let file_parts = parts.get(dash_index + 3..)?;
	if dash_index < 2 || file_parts.is_empty() {
		return None;
	}
	let project = parts[..dash_index].join("/");
	let file_path = file_parts.join("/");
	Some((
		format!("{project}/{file_path}"),
		format!(
			"https://gitlab.com/{}/-/raw/{}/{}",
			parts[..dash_index]
				.iter()
				.map(|part| utf8_percent_encode(part, PATH_SEGMENT_ENCODE).to_string())
				.collect::<Vec<_>>()
				.join("/"),
			revision,
			file_parts
				.iter()
				.map(|part| utf8_percent_encode(part, PATH_SEGMENT_ENCODE).to_string())
				.collect::<Vec<_>>()
				.join("/")
		),
	))
}

fn gitlab_repo(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if host != "gitlab.com" {
		return None;
	}
	let parts = decoded_path_segments(url);
	if parts.len() < 2 {
		return None;
	}
	let mut repo_parts = Vec::new();
	for part in parts {
		if matches!(
			part.as_str(),
			"-" | "tree" | "blob" | "commit" | "commits" | "issues" | "merge_requests" | "releases"
		) {
			break;
		}
		if !is_repo_path_part(&part) {
			return None;
		}
		repo_parts.push(part);
	}
	(repo_parts.len() >= 2).then(|| repo_parts.join("/"))
}

fn is_repo_path_part(part: &str) -> bool {
	!part.is_empty()
		&& part != "."
		&& part != ".."
		&& part
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn repo_title_from_metadata(provider: &str, metadata: &Value) -> Option<String> {
	let name = json_string(&metadata["full_name"])
		.or_else(|| json_string(&metadata["path_with_namespace"]))
		.or_else(|| json_string(&metadata["name_with_namespace"]))
		.or_else(|| json_string(&metadata["name"]))?;
	let mut lines = vec![format!("{provider} repository: {name}")];
	if let Some(description) = json_string(&metadata["description"]) {
		lines.push(format!("Description: {description}"));
	}
	if let Some(stars) =
		json_u64(&metadata["stargazers_count"]).or_else(|| json_u64(&metadata["star_count"]))
	{
		lines.push(format!("Stars: {stars}"));
	}
	if let Some(forks) = json_u64(&metadata["forks_count"]) {
		lines.push(format!("Forks: {forks}"));
	}
	if let Some(issues) = json_u64(&metadata["open_issues_count"]) {
		lines.push(format!("Open issues: {issues}"));
	}
	if let Some(language) = json_string(&metadata["language"]) {
		lines.push(format!("Language: {language}"));
	}
	if let Some(license) = json_string(&metadata["license"]["spdx_id"])
		.or_else(|| json_string(&metadata["license"]["name"]))
	{
		lines.push(format!("License: {license}"));
	}
	if let Some(updated) =
		json_string(&metadata["pushed_at"]).or_else(|| json_string(&metadata["last_activity_at"]))
	{
		lines.push(format!("Updated: {updated}"));
	}
	Some(lines.join("\n"))
}

fn wikidata_title(url: &Url) -> Option<String> {
	let id = wikidata_entity_id(url)?;
	let client = metadata_client()?;
	let response: Value = client
		.get(format!(
			"https://www.wikidata.org/wiki/Special:EntityData/{id}.json"
		))
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	let entity = &response["entities"][id.as_str()];
	let labels = wikidata_label_map(&client, entity);
	wikidata_title_from_entity(&id, entity, &labels)
}

fn wikidata_entity_id(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if host != "wikidata.org" {
		return None;
	}
	let parts = decoded_path_segments(url);
	if parts.first()? != "wiki" {
		return None;
	}
	let id = parts.get(1)?.to_ascii_uppercase();
	is_wikidata_entity_id(&id).then_some(id)
}

fn is_wikidata_entity_id(id: &str) -> bool {
	static REGEX: OnceLock<Regex> = OnceLock::new();
	REGEX
		.get_or_init(|| Regex::new(r#"^[QP][1-9][0-9]*$"#).unwrap())
		.is_match(id)
}

fn wikidata_label_map(client: &Client, entity: &Value) -> HashMap<String, String> {
	let ids = wikidata_statement_ids(entity);
	if ids.is_empty() {
		return HashMap::new();
	}
	let ids_param = ids.join("|");
	let response: Option<Value> = client
		.get("https://www.wikidata.org/w/api.php")
		.query(&[
			("action", "wbgetentities"),
			("ids", ids_param.as_str()),
			("props", "labels"),
			("languages", "en"),
			("format", "json"),
		])
		.send()
		.ok()
		.and_then(|response| response.error_for_status().ok())
		.and_then(|response| response.text().ok())
		.and_then(|body| serde_json::from_str(&body).ok());
	response
		.as_ref()
		.and_then(|value| value["entities"].as_object())
		.map(|entities| {
			entities
				.iter()
				.filter_map(|(id, entity)| {
					json_string(&entity["labels"]["en"]["value"]).map(|label| (id.clone(), label))
				})
				.collect()
		})
		.unwrap_or_default()
}

fn wikidata_statement_ids(entity: &Value) -> Vec<String> {
	let mut ids = Vec::new();
	let Some(claims) = entity["claims"].as_object() else {
		return ids;
	};
	for (property, statements) in claims {
		push_unique_id(&mut ids, property);
		for statement in statements.as_array().into_iter().flatten().take(2) {
			if let Some(value_id) = wikidata_statement_entity_id(statement) {
				push_unique_id(&mut ids, &value_id);
			}
		}
		if ids.len() >= 40 {
			break;
		}
	}
	ids
}

fn push_unique_id(ids: &mut Vec<String>, id: &str) {
	if !ids.iter().any(|seen| seen == id) {
		ids.push(id.to_string());
	}
}

fn wikidata_title_from_entity(
	id: &str,
	entity: &Value,
	labels: &HashMap<String, String>,
) -> Option<String> {
	let label = json_string(&entity["labels"]["en"]["value"]).unwrap_or_else(|| id.to_string());
	let mut lines = vec![format!("Wikidata: {label}")];
	let Some(claims) = entity["claims"].as_object() else {
		return Some(lines.join("\n"));
	};
	for (property, statements) in claims {
		let Some(statement) = statements.as_array().and_then(|items| items.first()) else {
			continue;
		};
		let Some(value) = wikidata_statement_value(statement, labels) else {
			continue;
		};
		let property_label = labels
			.get(property)
			.cloned()
			.unwrap_or_else(|| property.to_string());
		lines.push(format!("{property_label}: {value}"));
		if lines.len() >= 11 {
			break;
		}
	}
	Some(lines.join("\n"))
}

fn wikidata_statement_entity_id(statement: &Value) -> Option<String> {
	let value = &statement["mainsnak"]["datavalue"]["value"];
	let entity_type = value["entity-type"].as_str()?;
	if !matches!(entity_type, "item" | "property") {
		return None;
	}
	json_string(&value["id"])
}

fn wikidata_statement_value(statement: &Value, labels: &HashMap<String, String>) -> Option<String> {
	let datavalue = &statement["mainsnak"]["datavalue"];
	let datatype = datavalue["type"].as_str()?;
	let value = &datavalue["value"];
	match datatype {
		"wikibase-entityid" => {
			let id = json_string(&value["id"])?;
			Some(labels.get(&id).cloned().unwrap_or(id))
		}
		"string" | "external-id" | "url" => json_string(value),
		"monolingualtext" => json_string(&value["text"]),
		"time" => json_string(&value["time"]).map(|time| time.trim_start_matches('+').to_string()),
		"quantity" => {
			json_string(&value["amount"]).map(|amount| amount.trim_start_matches('+').to_string())
		}
		"globecoordinate" => {
			let latitude = value["latitude"].as_f64()?;
			let longitude = value["longitude"].as_f64()?;
			Some(format!("{latitude:.5}, {longitude:.5}"))
		}
		_ => None,
	}
}

fn rutracker_title(url: &Url) -> Option<String> {
	rutracker_topic_id(url)?;
	let client = metadata_client()?;
	let html = safe_get(&client, url.as_str())?
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()?;
	rutracker_title_from_html(&html)
}

fn rutracker_topic_id(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if !matches!(host.as_str(), "rutracker.org" | "rutracker.me") {
		return None;
	}
	(url.path() == "/forum/viewtopic.php").then_some(())?;
	url.query_pairs()
		.find(|(key, _)| key == "t")
		.map(|(_, value)| value.into_owned())
		.filter(|topic_id| topic_id.chars().all(|c| c.is_ascii_digit()))
}

fn rutracker_title_from_html(html: &str) -> Option<String> {
	let mut lines = Vec::new();
	if let Some(title) = rutracker_topic_title(html) {
		lines.push(format!("RuTracker topic: {title}"));
	}
	if let Some(description) = rutracker_topic_description(html) {
		lines.push(format!("Description: {description}"));
	}
	if let Some(comments) = rutracker_comment_count(html) {
		lines.push(format!("Comments: {comments}"));
	}
	(!lines.is_empty()).then(|| lines.join("\n"))
}

fn rutracker_topic_title(html: &str) -> Option<String> {
	first_capture(
		html,
		r#"(?is)<h1[^>]*>(.*?)</h1>|<title[^>]*>(.*?)</title>"#,
	)
	.and_then(|title| {
		let title = strip_html(&title)
			.replace(":: RuTracker.org", "")
			.replace(":: RuTracker.me", "");
		compact_title_text(&title)
	})
}

fn rutracker_topic_description(html: &str) -> Option<String> {
	first_capture(
		html,
		r#"(?is)<div[^>]*class=["'][^"']*\bpost_body\b[^"']*["'][^>]*>(.*?)</div>"#,
	)
	.or_else(|| {
		first_capture(
			html,
			r#"(?is)<meta\s+[^>]*name=["']description["'][^>]*content=["']([^"']+)["'][^>]*>"#,
		)
	})
	.map(|description| strip_html(&description))
	.and_then(|description| compact_title_text(&description))
}

fn rutracker_comment_count(html: &str) -> Option<u64> {
	first_capture(
		html,
		r#"(?is)(?:Ответ(?:ов|а)?|Комментарии|Comments|Replies)\s*[:：]\s*([0-9][0-9\s]*)"#,
	)
	.and_then(|count| count.replace(' ', "").parse().ok())
}

fn gentoo_package_title(url: &Url) -> Option<String> {
	let atom = gentoo_package_atom(url)?;
	let client = metadata_client()?;
	let html = safe_get(&client, url.as_str())?
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()?;
	gentoo_package_title_from_html(&atom, &html)
}

fn gentoo_package_atom(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if host != "packages.gentoo.org" {
		return None;
	}
	let parts = decoded_path_segments(url);
	if parts.first()? != "packages" {
		return None;
	}
	let category = parts.get(1)?;
	let package = parts.get(2)?;
	(!category.is_empty() && !package.is_empty()).then(|| format!("{category}/{package}"))
}

fn gentoo_package_title_from_html(atom: &str, html: &str) -> Option<String> {
	let mut lines = vec![format!("Gentoo package: {atom}")];
	if let Some(description) = gentoo_package_description(html) {
		lines.push(format!("Description: {description}"));
	}
	if let Some(license) = gentoo_package_row_value(html, "License") {
		lines.push(format!("License: {license}"));
	}
	if let Some(maintainer) = gentoo_package_row_value(html, "Maintainer(s)") {
		lines.push(format!("Maintainer: {maintainer}"));
	}
	(lines.len() > 1).then(|| lines.join("\n"))
}

fn gentoo_package_description(html: &str) -> Option<String> {
	first_capture(
		html,
		r#"(?is)<p[^>]*class=["'][^"']*\bkk-package-maindesc\b[^"']*["'][^>]*>(.*?)</p>"#,
	)
	.map(|description| strip_html(&description))
	.and_then(|description| compact_title_text(&description))
}

fn gentoo_package_row_value(html: &str, label: &str) -> Option<String> {
	let pattern = format!(
		r#"(?is)<span[^>]*></span>\s*{}\s*</div>\s*<div[^>]*>(.*?)</div>"#,
		regex::escape(label)
	);
	let row = first_capture(html, &pattern)?;
	first_capture(&row, r#"(?is)<a[^>]*title=["']([^"']+)["']"#)
		.or_else(|| Some(strip_html(&row)))
		.and_then(|value| compact_title_text(&value))
}

fn lastfm_title(url: &Url) -> Option<String> {
	let (artist, track) = lastfm_track(url)?;
	if let Some(token) = env::var("LASTFM_API_KEY")
		.ok()
		.map(|token| token.trim().to_string())
		.filter(|token| !token.is_empty())
	{
		let response: Option<Value> = metadata_client()?
			.get("https://ws.audioscrobbler.com/2.0/")
			.query(&[
				("method", "track.getInfo"),
				("api_key", token.as_str()),
				("artist", artist.as_str()),
				("track", track.as_str()),
				("format", "json"),
				("autocorrect", "1"),
			])
			.send()
			.ok()
			.and_then(|response| response.error_for_status().ok())
			.and_then(|response| response.text().ok())
			.and_then(|body| serde_json::from_str(&body).ok());
		if let Some(title) = response.as_ref().and_then(lastfm_title_from_api) {
			return Some(title);
		}
	}
	let client = metadata_client()?;
	let html = safe_get(&client, url.as_str())?
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()?;
	lastfm_title_from_html(&html)
}

fn lastfm_track(url: &Url) -> Option<(String, String)> {
	let host = normalized_host(url)?;
	if host != "last.fm" {
		return None;
	}
	let parts = decoded_path_segments(url);
	let music_index = parts.iter().position(|part| part == "music")?;
	let artist = parts.get(music_index + 1)?;
	let marker = parts.get(music_index + 2)?;
	if marker != "_" {
		return None;
	}
	let track = parts.get(music_index + 3)?;
	Some((lastfm_path_label(artist), lastfm_path_label(track)))
}

fn lastfm_path_label(value: &str) -> String {
	value.replace('+', " ")
}

fn lastfm_title_from_api(metadata: &Value) -> Option<String> {
	let track = &metadata["track"];
	let track_name = json_string(&track["name"])?;
	let artist = json_string(&track["artist"]["name"]).unwrap_or_default();
	let primary = if artist.is_empty() {
		track_name
	} else {
		format!("{artist} - {track_name}")
	};
	let mut lines = vec![format!("Last.fm track: {primary}")];
	if let Some(album) = json_string(&track["album"]["title"]) {
		lines.push(format!("Album: {album}"));
	}
	if let Some(duration) = json_u64(&track["duration"]).filter(|duration| *duration > 0) {
		lines.push(format!("Length: {}", format_duration(duration)));
	}
	if let Some(listeners) = json_string(&track["listeners"]) {
		lines.push(format!("Listeners: {listeners}"));
	}
	if let Some(playcount) = json_string(&track["playcount"]) {
		lines.push(format!("Scrobbles: {playcount}"));
	}
	let tags = track["toptags"]["tag"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|tag| json_string(&tag["name"]))
		.take(4)
		.collect::<Vec<_>>();
	if !tags.is_empty() {
		lines.push(format!("Tags: {}", tags.join(", ")));
	}
	if let Some(summary) = json_string(&track["wiki"]["summary"])
		.map(|summary| strip_html(&summary))
		.and_then(|summary| compact_title_text(&summary))
	{
		lines.push(format!("Summary: {summary}"));
	}
	Some(lines.join("\n"))
}

fn lastfm_title_from_html(html: &str) -> Option<String> {
	let title = meta_content(html, "og:title").or_else(|| html_title_text(html))?;
	let mut lines = vec![format!("Last.fm track: {title}")];
	if let Some(description) = meta_content(html, "description") {
		lines.push(format!("Description: {description}"));
	}
	if let Some(length) = lastfm_catalogue_value(html, "Length") {
		lines.push(format!("Length: {length}"));
	}
	if let Some(listeners) = lastfm_stat(html, "Listeners") {
		lines.push(format!("Listeners: {listeners}"));
	}
	if let Some(scrobbles) = lastfm_stat(html, "Scrobbles") {
		lines.push(format!("Scrobbles: {scrobbles}"));
	}
	Some(lines.join("\n"))
}

fn lastfm_stat(html: &str, label: &str) -> Option<String> {
	let pattern = format!(
		r#"(?is)<h4[^>]*class=["'][^"']*\bheader-metadata-tnew-title\b[^"']*["'][^>]*>\s*{}\s*</h4>.*?<abbr[^>]*title=["']([^"']+)["']"#,
		regex::escape(label)
	);
	first_capture(html, &pattern).and_then(|value| compact_title_text(&value))
}

fn lastfm_catalogue_value(html: &str, label: &str) -> Option<String> {
	let pattern = format!(
		r#"(?is)<dt[^>]*>\s*{}\s*</dt>\s*<dd[^>]*>(.*?)</dd>"#,
		regex::escape(label)
	);
	first_capture(html, &pattern)
		.map(|value| strip_html(&value))
		.and_then(|value| compact_title_text(&value))
}

fn mdn_title(url: &Url) -> Option<String> {
	mdn_doc(url).then_some(())?;
	let client = metadata_client()?;
	let html = safe_get(&client, url.as_str())?
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()?;
	mdn_title_from_html(&html)
}

fn mdn_doc(url: &Url) -> bool {
	normalized_host(url).as_deref() == Some("developer.mozilla.org")
		&& url.path().contains("/docs/")
}

fn mdn_title_from_html(html: &str) -> Option<String> {
	let title = meta_content(html, "og:title").or_else(|| html_title_text(html))?;
	let mut lines = vec![format!("MDN: {title}")];
	if let Some(summary) = meta_content(html, "description")
		.or_else(|| meta_content(html, "og:description"))
		.or_else(|| first_article_paragraph(html))
	{
		lines.push(format!("Summary: {summary}"));
	}
	Some(lines.join("\n"))
}

fn first_article_paragraph(html: &str) -> Option<String> {
	first_capture(
		html,
		r#"(?is)<main[^>]*>.*?<p[^>]*>(.*?)</p>|<article[^>]*>.*?<p[^>]*>(.*?)</p>"#,
	)
	.map(|paragraph| strip_html(&paragraph))
	.and_then(|paragraph| compact_title_text(&paragraph))
}

fn livejournal_title(url: &Url) -> Option<String> {
	livejournal_post(url).then_some(())?;
	let client = metadata_client()?;
	let html = safe_get(&client, url.as_str())?
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()?;
	livejournal_title_from_html(&html)
}

fn livejournal_post(url: &Url) -> bool {
	let Some(host) = normalized_host(url) else {
		return false;
	};
	if host != "livejournal.com" && !host.ends_with(".livejournal.com") {
		return false;
	}
	decoded_path_segments(url).iter().any(|part| {
		let id = part.strip_suffix(".html").unwrap_or(part);
		!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())
	})
}

fn livejournal_title_from_html(html: &str) -> Option<String> {
	let title = meta_content(html, "og:title").or_else(|| html_title_text(html));
	let text = meta_content(html, "description")
		.or_else(|| meta_content(html, "og:description"))
		.or_else(|| livejournal_post_text(html));
	let mut lines = Vec::new();
	if let Some(title) = title {
		lines.push(format!("LiveJournal: {title}"));
	}
	if let Some(text) = text {
		lines.push(format!("Text: {text}"));
	}
	if let Some(comments) = livejournal_comment_count(html) {
		lines.push(format!("Comments: {comments}"));
	}
	(!lines.is_empty()).then(|| lines.join("\n"))
}

fn livejournal_post_text(html: &str) -> Option<String> {
	first_capture(
		html,
		r#"(?is)<article[^>]*>(.*?)</article>|<div[^>]*class=["'][^"']*\bentry-content\b[^"']*["'][^>]*>(.*?)</div>"#,
	)
	.map(|text| strip_html(&text))
	.and_then(|text| compact_title_text(&text))
}

fn livejournal_comment_count(html: &str) -> Option<u64> {
	first_capture(
		html,
		r#"(?is)(?:comments?|комментар(?:ии|иев|ия|ий)|комменты)\D{0,40}([0-9][0-9\s]*)|([0-9][0-9\s]*)\s*(?:comments?|комментар(?:ии|иев|ия|ий)|комменты)"#,
	)
	.and_then(|count| count.replace(' ', "").parse().ok())
}

fn habr_user_title(url: &Url) -> Option<String> {
	let (alias, language) = habr_user_profile(url)?;
	let endpoint = format!(
		"https://habr.com/kek/v2/users/{}/card",
		utf8_percent_encode(&alias, PATH_SEGMENT_ENCODE)
	);
	let client = metadata_client()?;
	let response: Value = safe_get(&client, &endpoint)?
		.query(&[("hl", language)])
		.header("Accept", "application/json")
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	habr_user_title_from_card(&response)
}

fn habr_user_profile(url: &Url) -> Option<(String, &'static str)> {
	let host = normalized_host(url)?;
	if host != "habr.com" {
		return None;
	}
	let parts = decoded_path_segments(url);
	let (users_index, language) = match parts.first().map(String::as_str) {
		Some("ru") => (1, "ru"),
		Some("en") => (1, "en"),
		_ => (0, "en"),
	};
	(parts.get(users_index)? == "users").then_some(())?;
	let alias = parts.get(users_index + 1)?;
	is_habr_user_alias(alias).then(|| (alias.to_string(), language))
}

fn is_habr_user_alias(value: &str) -> bool {
	!value.is_empty()
		&& value
			.chars()
			.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

fn habr_user_title_from_card(card: &Value) -> Option<String> {
	let alias = json_string(&card["alias"])?;
	let primary = json_string(&card["fullname"])
		.map(|name| format!("Habr user: {name} (@{alias})"))
		.unwrap_or_else(|| format!("Habr user: @{alias}"));
	let mut lines = vec![primary];
	let publication_stats = &card["counterStats"]["publicationStats"];
	if let Some(count) = json_u64(&publication_stats["articleCount"]) {
		lines.push(format!("Articles: {count}"));
	}
	if let Some(count) = json_u64(&publication_stats["postCount"]) {
		lines.push(format!("Posts: {count}"));
	}
	if let Some(count) = json_u64(&publication_stats["newsCount"]) {
		lines.push(format!("News: {count}"));
	}
	if let Some(count) = json_u64(&card["counterStats"]["commentCount"]) {
		lines.push(format!("Comments: {count}"));
	}
	if let Some(date) = json_string(&card["registerDateTime"]) {
		lines.push(format!("Registration date: {date}"));
	}
	if let Some(rating) = json_scalar_text(&card["rating"]) {
		lines.push(format!("Rating: {rating}"));
	}
	(lines.len() > 1).then(|| lines.join("\n"))
}

fn telegram_channel_title(url: &Url) -> Option<String> {
	let channel = telegram_channel(url)?;
	let html = metadata_client()?
		.get(format!(
			"https://t.me/{}",
			utf8_percent_encode(&channel, PATH_SEGMENT_ENCODE)
		))
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()?;
	telegram_channel_title_from_html(&html)
}

fn telegram_channel(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if !matches!(host.as_str(), "t.me" | "telegram.me" | "telegram.dog") {
		return None;
	}
	let parts = decoded_path_segments(url)
		.into_iter()
		.filter(|part| !part.is_empty())
		.collect::<Vec<_>>();
	let channel = match parts.as_slice() {
		[channel] => channel,
		[first, channel] if first == "s" => channel,
		_ => return None,
	};
	is_telegram_channel_name(channel).then(|| channel.to_string())
}

fn is_telegram_channel_name(value: &str) -> bool {
	let lower = value.to_ascii_lowercase();
	!matches!(
		lower.as_str(),
		"addemoji"
			| "addstickers"
			| "addtheme"
			| "bg" | "c"
			| "joinchat"
			| "proxy" | "setlanguage"
			| "share" | "socks"
			| "iv"
	) && value.len() >= 5
		&& value.len() <= 32
		&& value
			.chars()
			.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn telegram_channel_title_from_html(html: &str) -> Option<String> {
	let title = telegram_channel_page_title(html)
		.or_else(|| meta_content(html, "og:title"))
		.or_else(|| html_title_text(html))?;
	let mut lines = vec![format!("Telegram channel: {title}")];
	if let Some(count) = telegram_subscriber_count(html) {
		lines.push(format!("Subscribers: {count}"));
	}
	if let Some(description) = meta_content(html, "og:description")
		.or_else(|| meta_content(html, "description"))
		.or_else(|| telegram_channel_description(html))
	{
		lines.push(format!("Description: {description}"));
	}
	(lines.len() > 1).then(|| lines.join("\n"))
}

fn telegram_channel_page_title(html: &str) -> Option<String> {
	first_capture(
		html,
		r#"(?is)<div[^>]*class=["'][^"']*\btgme_page_title\b[^"']*["'][^>]*>.*?<span[^>]*>(.*?)</span>"#,
	)
	.map(|title| strip_html(&title))
	.and_then(|title| compact_title_text(&title))
}

fn telegram_subscriber_count(html: &str) -> Option<String> {
	let extra = first_capture(
		html,
		r#"(?is)<div[^>]*class=["'][^"']*\btgme_page_extra\b[^"']*["'][^>]*>(.*?)</div>"#,
	)
	.map(|extra| strip_html(&extra))?;
	first_capture(&extra, r#"(?i)\b([0-9][0-9\s.,]*[KMB]?)\s+subscribers?\b"#)
		.or_else(|| {
			extra
				.to_ascii_lowercase()
				.contains("subscriber")
				.then_some(extra)
		})
		.and_then(|count| compact_title_text(&count))
}

fn telegram_channel_description(html: &str) -> Option<String> {
	first_capture(
		html,
		r#"(?is)<div[^>]*class=["'][^"']*\btgme_page_description\b[^"']*["'][^>]*>(.*?)</div>"#,
	)
	.map(|description| strip_html(&description))
	.and_then(|description| compact_title_text(&description))
}

/// Build hover metadata for a public Google Drive folder link.
fn google_drive_folder_title(url: &Url) -> Option<String> {
	let folder_id = google_drive_folder_id(url)?;
	let client = metadata_client()?;
	let html = safe_get(
		&client,
		&format!("https://drive.google.com/embeddedfolderview?id={folder_id}"),
	)?
	.send()
	.ok()?
	.error_for_status()
	.ok()?
	.text()
	.ok()?;
	google_drive_folder_title_from_html(&html)
}

/// Extract the Google Drive folder id from common public folder URLs.
fn google_drive_folder_id(url: &Url) -> Option<String> {
	let host = normalized_host(url)?;
	if host != "drive.google.com" {
		return None;
	}
	let parts = decoded_path_segments(url);
	parts
		.windows(2)
		.find_map(|window| {
			(window[0] == "folders")
				.then(|| window[1].clone())
				.filter(|id| google_drive_id(id))
		})
		.or_else(|| {
			url.query_pairs()
				.find(|(key, _)| key == "id")
				.map(|(_, value)| value.into_owned())
				.filter(|id| google_drive_id(id))
		})
}

fn google_drive_id(id: &str) -> bool {
	id.len() >= 8
		&& id
			.chars()
			.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

fn google_drive_folder_title_from_html(html: &str) -> Option<String> {
	let folder = meta_content(html, "og:title")
		.or_else(|| html_title_text(html))
		.map(|title| title.replace(" - Google Drive", ""))
		.and_then(|title| compact_title_text(&title));
	let files = google_drive_file_names_from_html(html);
	let mut lines = Vec::new();
	if let Some(folder) = folder {
		lines.push(format!("Google Drive folder: {folder}"));
	}
	if !files.is_empty() {
		lines.push("Files:".to_string());
		lines.extend(files.into_iter().map(|file| format!("- {file}")));
	}
	(!lines.is_empty()).then(|| lines.join("\n"))
}

fn google_drive_file_names_from_html(html: &str) -> Vec<String> {
	let mut files = Vec::new();
	for pattern in [
		r#"(?is)<[^>]*class=["'][^"']*\bflip-entry-title\b[^"']*["'][^>]*>(.*?)</[^>]+>"#,
		r#"(?is)\bdata-tooltip\s*=\s*(?:"([^"]+)"|'([^']+)')"#,
		r#"(?is)\baria-label\s*=\s*(?:"([^"]+)"|'([^']+)')"#,
	] {
		let Ok(regex) = Regex::new(pattern) else {
			continue;
		};
		for caps in regex.captures_iter(html) {
			let Some(name) = (1..caps.len())
				.find_map(|index| caps.get(index))
				.map(|value| value.as_str())
			else {
				continue;
			};
			let name = strip_html(&decode_html_entities(name));
			let Some(name) = compact_title_text(&name) else {
				continue;
			};
			if google_drive_file_label(&name) && !files.iter().any(|seen| seen == &name) {
				files.push(name);
				if files.len() >= 30 {
					return files;
				}
			}
		}
	}
	files
}

fn google_drive_file_label(name: &str) -> bool {
	let lower = name.to_ascii_lowercase();
	!matches!(
		lower.as_str(),
		"google drive"
			| "download"
			| "more actions"
			| "list view"
			| "grid view"
			| "open" | "preview"
			| "share"
	) && !lower.starts_with("shared with ")
		&& !lower.starts_with("folder: ")
		&& !name.is_empty()
}

fn stackoverflow_title(url: &Url) -> Option<String> {
	let question_id = stackoverflow_question_id(url)?;
	let response: Value = metadata_client()?
		.get(format!(
			"https://api.stackexchange.com/2.3/questions/{question_id}"
		))
		.query(&[("site", "stackoverflow")])
		.send()
		.ok()?
		.error_for_status()
		.ok()?
		.text()
		.ok()
		.and_then(|body| serde_json::from_str(&body).ok())?;
	let question = response["items"].as_array()?.first()?;
	stackoverflow_title_from_question(question)
}

fn stackoverflow_question_id(url: &Url) -> Option<u64> {
	let host = normalized_host(url)?;
	if host != "stackoverflow.com" {
		return None;
	}
	let parts = decoded_path_segments(url);
	match parts.first().map(String::as_str) {
		Some("questions" | "q") => parts.get(1)?.parse().ok(),
		_ => None,
	}
}

fn stackoverflow_title_from_question(question: &Value) -> Option<String> {
	let title =
		json_string(&question["title"]).map(|title| decode_html_entities(&title).to_string())?;
	let mut lines = vec![format!("Stack Overflow: {title}")];
	if let Some(score) = json_i64(&question["score"]) {
		lines.push(format!("Score: {score}"));
	}
	if let Some(count) = json_u64(&question["answer_count"]) {
		lines.push(format!("Answers: {count}"));
	}
	if question["accepted_answer_id"].as_i64().is_some() {
		lines.push("Accepted answer: yes".to_string());
	} else if let Some(answered) = question["is_answered"].as_bool() {
		lines.push(format!(
			"Accepted answer: {}",
			if answered { "yes" } else { "no" }
		));
	}
	if let Some(count) = json_u64(&question["view_count"]) {
		lines.push(format!("Views: {count}"));
	}
	if let Some(tags) = question["tags"]
		.as_array()
		.and_then(|values| json_string_list(values))
	{
		lines.push(format!("Tags: {tags}"));
	}
	if let Some(owner) = json_string(&question["owner"]["display_name"]) {
		lines.push(format!("Asked by: {owner}"));
	}
	if let Some(created) = json_u64(&question["creation_date"]).and_then(format_stackexchange_date)
	{
		lines.push(format!("Created: {created}"));
	}
	if let Some(activity) =
		json_u64(&question["last_activity_date"]).and_then(format_stackexchange_date)
	{
		lines.push(format!("Last activity: {activity}"));
	}
	Some(lines.join("\n"))
}

fn json_i64(value: &Value) -> Option<i64> {
	match value {
		Value::Number(number) => number.as_i64(),
		Value::String(value) => value.parse().ok(),
		_ => None,
	}
}

fn json_string_list(values: &[Value]) -> Option<String> {
	let list = values.iter().filter_map(json_string).collect::<Vec<_>>();
	(!list.is_empty()).then(|| list.join(", "))
}

fn format_stackexchange_date(timestamp: u64) -> Option<String> {
	let timestamp = i64::try_from(timestamp).ok()?;
	DateTime::<Utc>::from_timestamp(timestamp, 0).map(|date| date.format("%Y-%m-%d").to_string())
}

fn first_capture(html: &str, pattern: &str) -> Option<String> {
	let caps = Regex::new(pattern).ok()?.captures(html)?;
	(1..caps.len())
		.find_map(|index| caps.get(index))
		.map(|value| value.as_str().to_string())
}

fn meta_content(html: &str, key: &str) -> Option<String> {
	let escaped = regex::escape(key);
	for attr in ["name", "property"] {
		let pattern = format!(
			r#"(?is)<meta\b[^>]*\b{}\s*=\s*["']{}["'][^>]*\bcontent\s*=\s*(?:"([^"]*)"|'([^']*)')"#,
			attr, escaped
		);
		if let Some(value) = first_capture(html, &pattern) {
			return compact_title_text(&decode_html_entities(&value));
		}
		let pattern = format!(
			r#"(?is)<meta\b[^>]*\bcontent\s*=\s*(?:"([^"]*)"|'([^']*)')[^>]*\b{}\s*=\s*["']{}["']"#,
			attr, escaped
		);
		if let Some(value) = first_capture(html, &pattern) {
			return compact_title_text(&decode_html_entities(&value));
		}
	}
	None
}

fn html_title_text(html: &str) -> Option<String> {
	first_capture(html, r#"(?is)<title[^>]*>(.*?)</title>"#)
		.map(|title| strip_html(&title))
		.and_then(|title| compact_title_text(&title))
}

fn strip_html(html: &str) -> String {
	let without_scripts = Regex::new(r#"(?is)<script[^>]*>.*?</script>|<style[^>]*>.*?</style>"#)
		.unwrap()
		.replace_all(html, " ");
	let with_breaks = Regex::new(r#"(?is)<\s*(br|/p|/div|/li|/tr)\b[^>]*>"#)
		.unwrap()
		.replace_all(&without_scripts, "\n");
	let without_tags = Regex::new(r#"(?is)<[^>]+>"#)
		.unwrap()
		.replace_all(&with_breaks, " ");
	decode_html_entities(&without_tags).to_string()
}

fn normalized_host(url: &Url) -> Option<String> {
	Some(
		url.host_str()?
			.trim_start_matches("www.")
			.to_ascii_lowercase(),
	)
}

fn expand_rich_link_blocks_with<F>(markdown: &str, expand_url: &F) -> String
where
	F: Fn(&str, &str) -> String,
{
	let rich_link =
		Regex::new(r#"(?is)<(?:p|div)\b[^>]*>\s*<a\b([^>]*)>(.*?)</a>\s*</(?:p|div)>"#).unwrap();
	rich_link
		.replace_all(markdown, |caps: &regex::Captures| {
			let attrs = caps.get(1).unwrap().as_str();
			let Some(href) = href_attr(attrs) else {
				return caps.get(0).unwrap().as_str().to_string();
			};
			let url = clean_url(&href);
			if Url::parse(url).is_err() {
				return caps.get(0).unwrap().as_str().to_string();
			}
			expand_url_and_keep_source(url, caps.get(0).unwrap().as_str(), expand_url)
		})
		.into_owned()
}

fn href_attr(attrs: &str) -> Option<String> {
	let href = Regex::new(r#"(?is)\bhref\s*=\s*(?:"([^"]+)"|'([^']+)'|([^\s>]+))"#).unwrap();
	let caps = href.captures(attrs)?;
	caps.get(1)
		.or_else(|| caps.get(2))
		.or_else(|| caps.get(3))
		.map(|value| value.as_str().to_string())
}

fn set_title_attr(attrs: &str, title: &str) -> String {
	let title_attr = format!(r#"title="{}""#, encode_title_attribute(title));
	let title_regex = Regex::new(r#"(?is)\btitle\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)"#).unwrap();
	if let Some(matched) = title_regex.find(attrs) {
		let mut out = String::new();
		out.push_str(&attrs[..matched.start()]);
		out.push_str(&title_attr);
		out.push_str(&attrs[matched.end()..]);
		return out;
	}
	format!("{attrs} {title_attr}")
}

fn encode_title_attribute(title: &str) -> String {
	encode_double_quoted_attribute(title).replace('\n', "&#10;")
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
		assert_eq!(
			detect("https://archive.org/details/example-item")
				.unwrap()
				.provider,
			WidgetProvider::InternetArchive
		);
		assert_eq!(
			detect("https://ok.ru/video/15486435789402")
				.unwrap()
				.provider,
			WidgetProvider::OkRu
		);
	}

	#[test]
	fn expands_only_bare_links() {
		let md = "hello\nhttps://youtu.be/abc\n[link](https://youtu.be/abc)";
		let out = expand_bare_links(md, true);

		assert!(out.contains(r#"{{ youtube(id="abc") }}"#), "{out}");
		assert!(
			out.contains(r#"<a href="https://youtu.be/abc" rel="noopener">Original link</a>"#),
			"{out}"
		);
		assert!(out.contains("[link](https://youtu.be/abc)"), "{out}");
	}

	#[test]
	fn expands_rich_link_blocks() {
		let html = r#"<div><a href=https://www.youtube.com/watch?v=e2_qbL4TiDg rev=en_rl_none>https://www.youtube.com/watch?v=e2_qbL4TiDg</a></div>"#;
		let out = expand_bare_links(html, true);

		assert_eq!(
			out,
			r#"{{ youtube(id="e2_qbL4TiDg") }}
<p class="embed-link embed-link--source"><a href="https://www.youtube.com/watch?v=e2_qbL4TiDg" rel="noopener">Original link</a></p>"#
		);
	}

	#[test]
	fn expands_labeled_standalone_anchor_blocks() {
		let html =
			r#"<div><a href="https://www.youtube.com/watch?v=e2_qbL4TiDg">YouTube</a></div>"#;
		let out = expand_bare_links(html, true);

		assert_eq!(
			out,
			r#"{{ youtube(id="e2_qbL4TiDg") }}
<p class="embed-link embed-link--source"><a href="https://www.youtube.com/watch?v=e2_qbL4TiDg" rel="noopener">Original link</a></p>"#
		);
	}

	#[test]
	fn expands_direct_audio_video_links() {
		let md = "https://cdn.example.test/audio.oga\nhttps://cdn.example.test/video.mkv";
		let out = expand_bare_links_with(md, true, expand_direct_media_for_test);

		assert!(out.contains("<audio controls"), "{out}");
		assert!(
			out.contains(r#"<source src="https://cdn.example.test/audio.oga">"#),
			"{out}"
		);
		assert!(
			out.contains(
				r#"<a href="https://cdn.example.test/audio.oga" rel="noopener">Original link</a>"#
			),
			"{out}"
		);
		assert!(out.contains("<video controls"), "{out}");
		assert!(out.contains("playsinline"), "{out}");
		assert!(
			out.contains(r#"<source src="https://cdn.example.test/video.mkv">"#),
			"{out}"
		);
		assert!(
			out.contains(
				r#"<a href="https://cdn.example.test/video.mkv" rel="noopener">Original link</a>"#
			),
			"{out}"
		);
	}

	#[test]
	fn expands_rich_direct_media_link_blocks() {
		let html = r#"<p><a href="https://cdn.example.test/song.opus">https://cdn.example.test/song.opus</a></p>"#;
		let out = expand_bare_links_with(html, true, expand_direct_media_for_test);

		assert!(out.contains("<audio controls"), "{out}");
		assert!(
			out.contains(r#"<source src="https://cdn.example.test/song.opus">"#),
			"{out}"
		);
		assert!(
			out.contains(
				r#"<a href="https://cdn.example.test/song.opus" rel="noopener">Original link</a>"#
			),
			"{out}"
		);
	}

	fn expand_direct_media_for_test(url: &str, fallback: &str) -> String {
		Url::parse(url)
			.ok()
			.and_then(|parsed| direct_media_embed(&parsed))
			.unwrap_or_else(|| fallback.to_string())
	}

	#[test]
	fn detects_direct_media_extensions() {
		for url in [
			"https://cdn.example.test/song.ogg",
			"https://cdn.example.test/song.oga",
			"https://cdn.example.test/song.opus",
		] {
			assert_eq!(
				direct_media_kind_from_url(&Url::parse(url).unwrap()),
				Some(DirectMediaKind::Audio)
			);
		}
		for url in [
			"https://cdn.example.test/movie.ogv",
			"https://cdn.example.test/movie.mkv",
			"https://cdn.example.test/movie.mp4",
		] {
			assert_eq!(
				direct_media_kind_from_url(&Url::parse(url).unwrap()),
				Some(DirectMediaKind::Video)
			);
		}
		assert_eq!(
			direct_media_kind_from_url(&Url::parse("https://cdn.example.test/vm.ova").unwrap()),
			None
		);
	}

	#[test]
	fn adds_wikipedia_summary_to_link_title() {
		let html = r#"<p><a href="https://en.wikipedia.org/wiki/PostgreSQL">PostgreSQL</a></p>"#;
		let out = enrich_link_titles_with(html, |url| {
			assert_eq!(url.as_str(), "https://en.wikipedia.org/wiki/PostgreSQL");
			Some(r#"PostgreSQL is a database & "tool"."#.into())
		});

		assert_eq!(
			out,
			r#"<p><a href="https://en.wikipedia.org/wiki/PostgreSQL" title="PostgreSQL is a database &amp; &quot;tool&quot;.">PostgreSQL</a></p>"#
		);
	}

	#[test]
	fn replaces_existing_link_title_when_metadata_exists() {
		let html =
			r#"<a href="https://en.wikipedia.org/wiki/PostgreSQL" title="Keep">PostgreSQL</a>"#;
		let out = enrich_link_titles_with(html, |_| Some("PostgreSQL intro".into()));

		assert_eq!(
			out,
			r#"<a href="https://en.wikipedia.org/wiki/PostgreSQL" title="PostgreSQL intro">PostgreSQL</a>"#
		);
	}

	#[test]
	fn encodes_link_title_line_breaks_as_entities() {
		let html = r#"<a href="https://en.wikipedia.org/wiki/PostgreSQL">PostgreSQL</a>"#;
		let out = enrich_link_titles_with(html, |_| {
			Some("PostgreSQL intro.\n\nPage created: 2003-11-06T08:19:24Z".into())
		});

		assert_eq!(
			out,
			r#"<a href="https://en.wikipedia.org/wiki/PostgreSQL" title="PostgreSQL intro.&#10;&#10;Page created: 2003-11-06T08:19:24Z">PostgreSQL</a>"#
		);
	}

	#[test]
	fn keeps_existing_link_title_when_metadata_is_missing() {
		let html = r#"<a href="https://example.com/" title="Keep">Example</a>"#;
		let out = enrich_link_titles_with(html, |_| None);

		assert_eq!(out, html);
	}

	#[test]
	fn extracts_wikipedia_article_titles() {
		let article = Url::parse("https://en.wikipedia.org/wiki/Free_software").unwrap();
		let mobile = Url::parse("https://en.m.wikipedia.org/wiki/Free_software").unwrap();
		let special = Url::parse("https://en.wikipedia.org/wiki/Special:Random").unwrap();

		assert_eq!(
			wikipedia_article_title(&article).as_deref(),
			Some("Free software")
		);
		assert_eq!(
			wikipedia_api_host(&mobile).as_deref(),
			Some("en.wikipedia.org")
		);
		assert!(wikipedia_article_title(&special).is_none());
	}

	#[test]
	fn builds_archive_org_item_title() {
		let url = Url::parse("https://archive.org/details/example-item").unwrap();
		let metadata = serde_json::json!({
			"item_size": 1536,
			"metadata": {
				"publicdate": "2024-02-03 12:13:14",
				"uploader": "archive-user"
			}
		});

		assert_eq!(
			archive_org_title_from_metadata(&url, &metadata).as_deref(),
			Some("Size: 1.5 KiB\nPublication date: 2024-02-03 12:13:14\nUploaded by: archive-user")
		);
	}

	#[test]
	fn builds_archive_org_file_title() {
		let url = Url::parse("https://archive.org/download/example-item/movie%20file.mp4").unwrap();
		let metadata = serde_json::json!({
			"item_size": 1536,
			"files": [
				{
					"name": "movie file.mp4",
					"size": "10485760"
				}
			],
			"metadata": {
				"date": "2024",
				"creator": "Archive Creator"
			}
		});

		assert_eq!(
			archive_org_title_from_metadata(&url, &metadata).as_deref(),
			Some("Size: 10 MiB\nPublication date: 2024\nUploaded by: Archive Creator")
		);
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
			detect("https://soundcloud.com/forss/flickermood")
				.unwrap()
				.shortcode,
			r#"{{ soundcloud(url="https://w.soundcloud.com/player/?url=https%3A%2F%2Fsoundcloud%2Ecom%2Fforss%2Fflickermood") }}"#
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
	fn expands_requested_video_widgets() {
		assert_eq!(
			detect("https://vimeo.com/123456").unwrap().shortcode,
			r#"{{ vimeo(id="123456") }}"#
		);
		assert_eq!(
			detect("https://rumble.com/embed/v6abc/").unwrap().shortcode,
			r#"{{ rumble(url="https://rumble.com/embed/v6abc/") }}"#
		);
		assert_eq!(
			detect("https://www.dailymotion.com/video/xakbodi")
				.unwrap()
				.shortcode,
			r#"{{ dailymotion(id="xakbodi") }}"#
		);
		assert_eq!(
			detect("https://dai.ly/xakbodi").unwrap().shortcode,
			r#"{{ dailymotion(id="xakbodi") }}"#
		);
		assert_eq!(
			detect("https://archive.org/details/bot_telegram_rutracker_lecture")
				.unwrap()
				.shortcode,
			r#"{{ archive_org(id="bot_telegram_rutracker_lecture") }}"#
		);
		assert_eq!(
			detect("https://odysee.com/@channel:1/video:2")
				.unwrap()
				.shortcode,
			r#"{{ odysee(url="https://odysee.com/$/embed/@channel:1/video:2") }}"#
		);
		assert_eq!(
			detect("https://www.bilibili.com/video/BV1xx411c7mD/")
				.unwrap()
				.shortcode,
			r#"{{ bilibili(url="https://player.bilibili.com/player.html?bvid=BV1xx411c7mD") }}"#
		);
		assert_eq!(
			detect("https://www.tiktok.com/@user/video/1234567890")
				.unwrap()
				.shortcode,
			r#"{{ tiktok(url="https://www.tiktok.com/@user/video/1234567890", id="1234567890") }}"#
		);
		assert_eq!(
			detect("https://store.steampowered.com/app/730/CounterStrike_2/")
				.unwrap()
				.shortcode,
			r#"{{ steam(app_id="730") }}"#
		);
		assert_eq!(
			detect("https://vk.com/audio_playlist-23865151_83082491")
				.unwrap()
				.shortcode,
			r#"{{ vk_playlist(oid="-23865151", pid="83082491") }}"#
		);
		assert_eq!(
			detect("https://ok.ru/video/15486435789402")
				.unwrap()
				.shortcode,
			r#"{{ ok_ru_video(id="15486435789402") }}"#
		);
		assert!(detect("https://vk.com/id1").is_none());
	}

	#[test]
	fn extracts_my_mail_ru_embed_id() {
		let html = r#"<link rel="image_src" href="http://my.mail.ru/+/video/url/sc01/2509222333588176924"><meta property="og:image" content="http://my.mail.ru/+/video/url/sc01/2509222333588176924">"#;

		assert_eq!(
			my_mail_ru_embed_id_from_html(html).as_deref(),
			Some("2509222333588176924")
		);
	}

	#[test]
	fn extracts_rumble_embed_url_from_oembed() {
		let response = serde_json::json!({
			"html": "<iframe src=\"https://rumble.com/embed/v79xd5g/\" title=\"Test\"></iframe>"
		});

		assert_eq!(
			rumble_embed_url_from_oembed(&response).as_deref(),
			Some("https://rumble.com/embed/v79xd5g/")
		);
	}

	#[test]
	fn detects_reddit_posts_and_subreddits() {
		let post = Url::parse("https://www.reddit.com/r/rust/comments/1abcde/example/").unwrap();
		let subreddit = Url::parse("https://www.reddit.com/r/rust/").unwrap();

		assert_eq!(
			reddit_post_json_url(&post).as_deref(),
			Some("https://www.reddit.com/r/rust/comments/1abcde.json")
		);
		assert_eq!(reddit_subreddit(&subreddit).as_deref(), Some("rust"));
		assert!(reddit_subreddit(&post).is_none());
	}

	#[test]
	fn builds_reddit_subreddit_widget_script() {
		let html = reddit_subreddit_widget_html("rust", &reddit_subreddit_embed_url("rust"));

		assert!(html.contains("embed-reddit-subreddit"));
		assert!(html.contains(r#"data-subreddit="rust""#));
		assert!(html.contains(r#"src="https://www.reddit.com/r/rust/.embed?limit=5""#));
		assert!(html.contains("Open r/rust"));
	}

	#[test]
	fn builds_reddit_subreddit_card() {
		let data = serde_json::json!({
			"data": {
				"display_name_prefixed": "r/rust",
				"title": "Rust",
				"url": "/r/rust/",
				"public_description": "A place for Rust language discussion.",
				"subscribers": 1234567,
				"active_user_count": 456,
				"icon_img": "https://styles.redditmedia.com/rust.png?width=256"
			}
		});
		let card = reddit_subreddit_card_from_data(&data).unwrap();

		assert!(card.contains("reddit-subreddit-card"));
		assert!(card.contains("Rust"));
		assert!(card.contains("r/rust"));
		assert!(card.contains("A place for Rust language discussion."));
		assert!(card.contains("<dd>1.2M</dd>"));
		assert!(card.contains("<dd>456</dd>"));
		assert!(card.contains("rust.png"));
	}

	#[test]
	fn builds_mastodon_embed_urls_and_profile_cards() {
		let status = Url::parse("https://mastodon.social/@Gargron/100254678717223630").unwrap();
		let profile = Url::parse("https://mastodon.social/@Gargron").unwrap();
		assert!(mastodon_status_url(&status));
		assert_eq!(
			mastodon_profile_account(&profile).as_deref(),
			Some("Gargron")
		);
		assert_eq!(
			mastodon_embed_url_from_oembed_html(
				&status,
				r#"<blockquote data-embed-url="https://mastodon.social/@Gargron/100254678717223630/embed"></blockquote>"#,
			)
			.as_deref(),
			Some("https://mastodon.social/@Gargron/100254678717223630/embed")
		);
		assert!(
			mastodon_embed_url_from_oembed_html(
				&status,
				r#"<blockquote data-embed-url="https://evil.example/@Gargron/1/embed"></blockquote>"#,
			)
			.is_none()
		);

		let account = serde_json::json!({
			"url": "https://mastodon.social/@Gargron",
			"acct": "Gargron",
			"display_name": "Eugen <Rochko>",
			"avatar_static": "https://files.mastodon.social/avatar.png",
			"note": "<p>Founder & developer</p>",
			"statuses_count": 12345,
			"followers_count": 678901,
			"following_count": 42
		});
		let card = mastodon_profile_card_from_account("https://mastodon.social/@Gargron", &account)
			.unwrap();

		assert!(card.contains("mastodon-profile-card"));
		assert!(card.contains("Eugen &lt;Rochko&gt;"));
		assert!(card.contains("@Gargron@mastodon.social"));
		assert!(card.contains("Founder &amp; developer"));
		assert!(card.contains("<dd>12.3K</dd>"));
		assert!(card.contains("<dd>678.9K</dd>"));
	}

	#[test]
	fn marks_broken_links_red_without_fetching_title() {
		let html = r#"<p><a href="https://example.invalid/missing">missing</a></p>"#;
		let out = enrich_link_titles_with_status(
			html,
			|_| panic!("title resolver must not run"),
			|_| Some("Broken link: HTTP 404".into()),
		);

		assert_eq!(
			out,
			r#"<p><a href="https://example.invalid/missing" class="broken-link" title="Broken link: HTTP 404">missing</a></p>"#
		);
		assert!(definitely_broken_status(404));
		assert!(!definitely_broken_status(403));
	}

	#[test]
	fn ssrf_guard_blocks_non_public_targets() {
		for url in [
			"http://localhost/latest/meta-data/",
			"http://127.0.0.1/",
			"http://10.0.0.1/",
			"http://100.64.0.1/",
			"http://169.254.169.254/latest/meta-data/",
			"http://172.16.0.1/",
			"http://192.168.0.1/",
			"http://192.0.2.1/",
			"http://[::1]/",
			"http://[fe80::1]/",
			"http://[fc00::1]/",
			"http://[::ffff:169.254.169.254]/",
			"file:///etc/passwd",
		] {
			let parsed = Url::parse(url).unwrap();
			assert!(!ssrf_safe_url(&parsed), "{url}");
		}
	}

	#[test]
	fn ssrf_guard_allows_public_ip_targets() {
		for url in [
			"https://93.184.216.34/",
			"http://[2606:2800:220:1:248:1893:25c8:1946]/",
		] {
			let parsed = Url::parse(url).unwrap();
			assert!(ssrf_safe_url(&parsed), "{url}");
		}
	}

	#[test]
	fn does_not_treat_numeric_non_vimeo_urls_as_vimeo() {
		let stackoverflow = Url::parse(
			"http://stackoverflow.com/questions/9604723/alternate-output-format-for-psql",
		)
		.unwrap();
		let vimeo = Url::parse("https://vimeo.com/123456").unwrap();

		assert_eq!(vimeo_video_id(&stackoverflow), None);
		assert_eq!(vimeo_video_id(&vimeo), Some("123456"));
	}

	#[test]
	fn builds_wikipedia_title_with_creation_date() {
		let page = serde_json::json!({
			"extract": "PostgreSQL is a free and open-source relational database.",
			"revisions": [{"timestamp": "2003-11-06T08:19:24Z"}]
		});

		assert_eq!(
			wikipedia_title_from_page(&page).as_deref(),
			Some(
				"PostgreSQL is a free and open-source relational database.\n\nPage created: 2003-11-06T08:19:24Z"
			)
		);
	}

	#[test]
	fn builds_commons_file_title() {
		let page = serde_json::json!({
			"imageinfo": [{
				"width": 640,
				"height": 480,
				"mime": "image/jpeg",
				"size": 2048,
				"extmetadata": {
					"ImageDescription": {"value": "<p>Test image</p>"},
					"Artist": {"value": "<a>Jane Example</a>"},
					"DateTimeOriginal": {"value": "2024-05-01"},
					"LicenseShortName": {"value": "CC BY-SA 4.0"}
				}
			}]
		});

		let title = commons_file_title_from_page("File:Test.jpg", &page).unwrap();
		assert!(title.contains("Wikimedia Commons file: File:Test.jpg"));
		assert!(title.contains("Description: Test image"));
		assert!(title.contains("Author: Jane Example"));
		assert!(title.contains("License: CC BY-SA 4.0"));
		assert!(title.contains("Dimensions: 640 x 480"));
	}

	#[test]
	fn maps_commons_file_pages_to_direct_media_urls() {
		let page_url = Url::parse("https://commons.wikimedia.org/wiki/File:Example.webm").unwrap();
		let page = serde_json::json!({
			"imageinfo": [{
				"url": "https://upload.wikimedia.org/wikipedia/commons/0/00/Example.webm"
			}]
		});
		let media_url = commons_file_media_url_from_page(&page).unwrap();
		let html = direct_media_embed_html(&media_url, DirectMediaKind::Video);

		assert_eq!(
			commons_file_page_title(&page_url).as_deref(),
			Some("File:Example.webm")
		);
		assert_eq!(
			direct_media_kind_from_filename("File:Example.webm"),
			Some(DirectMediaKind::Video)
		);
		assert_eq!(
			media_url,
			"https://upload.wikimedia.org/wikipedia/commons/0/00/Example.webm"
		);
		assert!(
			html.contains(
				r#"<source src="https://upload.wikimedia.org/wikipedia/commons/0/00/Example.webm">"#
			),
			"{html}"
		);
		assert!(!html.contains("commons.wikimedia.org/wiki/File:"), "{html}");
	}

	#[test]
	fn builds_musicbrainz_release_title() {
		let metadata = serde_json::json!({
			"title": "Kind of Blue",
			"date": "1959",
			"country": "US",
			"status": "Official",
			"artist-credit": [{"name": "Miles Davis"}],
			"label-info": [{"label": {"name": "Columbia"}}],
			"media": [{"track-count": 5}]
		});

		let title = musicbrainz_title_from_metadata("release", &metadata).unwrap();
		assert!(title.contains("MusicBrainz release: Kind of Blue"));
		assert!(title.contains("Artist: Miles Davis"));
		assert!(title.contains("Tracks: 5"));
	}

	#[test]
	fn builds_repo_titles() {
		let metadata = serde_json::json!({
			"full_name": "vitaly-zdanevich/everpublich",
			"description": "Evernote to static website",
			"stargazers_count": 42,
			"forks_count": 3,
			"open_issues_count": 2,
			"language": "Rust",
			"license": {"spdx_id": "AGPL-3.0-or-later"},
			"pushed_at": "2026-07-06T00:00:00Z"
		});

		let title = repo_title_from_metadata("GitHub", &metadata).unwrap();
		assert!(title.contains("GitHub repository: vitaly-zdanevich/everpublich"));
		assert!(title.contains("Stars: 42"));
		assert!(title.contains("License: AGPL-3.0-or-later"));
	}

	#[test]
	fn builds_github_and_gitlab_file_titles() {
		let github =
			Url::parse("https://github.com/vitaly-zdanevich/everpublich/blob/main/src/widgets.rs")
				.unwrap();
		let gitlab =
			Url::parse("https://gitlab.com/group/sub/project/-/blob/main/src/lib.rs").unwrap();

		assert_eq!(
			github_file_raw_url(&github),
			Some((
				"vitaly-zdanevich/everpublich/src/widgets.rs".into(),
				"https://raw.githubusercontent.com/vitaly-zdanevich/everpublich/main/src/widgets.rs"
					.into()
			))
		);
		assert_eq!(
			gitlab_file_raw_url(&gitlab),
			Some((
				"group/sub/project/src/lib.rs".into(),
				"https://gitlab.com/group/sub/project/-/raw/main/src/lib.rs".into()
			))
		);

		let body = (1..=120)
			.map(|number| format!("line {number}"))
			.collect::<Vec<_>>()
			.join("\n");
		let preview = first_source_lines(&body, 100).unwrap();
		assert!(preview.contains("line 1"));
		assert!(preview.contains("line 100"));
		assert!(!preview.contains("line 101"));
	}

	#[test]
	fn builds_wikidata_statement_title() {
		let entity = serde_json::json!({
			"labels": {"en": {"value": "Douglas Adams"}},
			"claims": {
				"P31": [{
					"mainsnak": {
						"datavalue": {
							"type": "wikibase-entityid",
							"value": {"entity-type": "item", "id": "Q5"}
						}
					}
				}]
			}
		});
		let labels = HashMap::from([
			("P31".to_string(), "instance of".to_string()),
			("Q5".to_string(), "human".to_string()),
		]);

		assert_eq!(
			wikidata_title_from_entity("Q42", &entity, &labels).as_deref(),
			Some("Wikidata: Douglas Adams\ninstance of: human")
		);
	}

	#[test]
	fn links_plain_wikidata_ids_for_title_enrichment() {
		let html = r#"<p>See Q42, not fooQ5 or https://example.com/Q7.</p><p><a href="https://example.com/Q8">Q8</a> <code>Q9</code> {{ audio(src="Q10.mp3") }}</p>"#;
		let linked = link_wikidata_ids(html);
		assert!(linked.contains(r#"<a href="https://www.wikidata.org/wiki/Q42">Q42</a>"#));
		assert!(linked.contains("fooQ5"));
		assert!(linked.contains("https://example.com/Q7"));
		assert!(linked.contains(r#"<a href="https://example.com/Q8">Q8</a>"#));
		assert!(linked.contains("<code>Q9</code>"));
		assert!(linked.contains(r#"{{ audio(src="Q10.mp3") }}"#));

		let enriched = enrich_link_titles_with(&linked, |url| {
			(url.as_str() == "https://www.wikidata.org/wiki/Q42")
				.then(|| "Wikidata: Douglas Adams".to_string())
		});
		assert!(
			enriched.contains(
				r#"<a href="https://www.wikidata.org/wiki/Q42" title="Wikidata: Douglas Adams">Q42</a>"#
			),
			"{enriched}"
		);
	}

	#[test]
	fn builds_rutracker_title() {
		let html = r#"<html><head><title>Topic name :: RuTracker.org</title></head>
<body><div class="post_body">Topic <b>description</b></div><span>Ответов: 123</span></body></html>"#;

		assert_eq!(
			rutracker_title_from_html(html).as_deref(),
			Some("RuTracker topic: Topic name\nDescription: Topic description\nComments: 123")
		);
	}

	#[test]
	fn builds_gentoo_package_title() {
		let html = r#"<p class="lead kk-package-maindesc">Erlang grammar for Tree-sitter</p>
<div><span class="fa fa-fw fa-legal"></span> License</div><div class="col-xs-12 col-md-9">MIT</div>
<div><span class="fa fa-fw fa-user"></span> Maintainer(s)</div><div class="col-xs-12 col-md-9"><a title="Maciej Barc">Maciej</a></div>"#;

		assert_eq!(
			gentoo_package_title_from_html("dev-libs/tree-sitter-erlang", html).as_deref(),
			Some(
				"Gentoo package: dev-libs/tree-sitter-erlang\nDescription: Erlang grammar for Tree-sitter\nLicense: MIT\nMaintainer: Maciej Barc"
			)
		);
	}

	#[test]
	fn builds_lastfm_track_titles() {
		let url = Url::parse("https://www.last.fm/music/Cher/_/Believe").unwrap();
		assert_eq!(lastfm_track(&url), Some(("Cher".into(), "Believe".into())));

		let metadata = serde_json::json!({
			"track": {
				"name": "Believe",
				"artist": {"name": "Cher"},
				"album": {"title": "Believe"},
				"duration": "239000",
				"listeners": "1163524",
				"playcount": "7671529",
				"toptags": {"tag": [{"name": "pop"}]},
				"wiki": {"summary": "Believe is a song."}
			}
		});
		let api_title = lastfm_title_from_api(&metadata).unwrap();
		assert!(api_title.contains("Last.fm track: Cher - Believe"));
		assert!(api_title.contains("Length: 3:59"));
		assert!(api_title.contains("Tags: pop"));

		let html = r#"<title>Believe — Cher | Last.fm</title>
<meta name="description" content="Watch the video for Believe.">
<dt>Length</dt><dd>3:59</dd>
<h4 class="header-metadata-tnew-title">Listeners</h4><abbr title="1,163,524">1.2M</abbr>
<h4 class="header-metadata-tnew-title">Scrobbles</h4><abbr title="7,671,529">7.7M</abbr>"#;
		let html_title = lastfm_title_from_html(html).unwrap();
		assert!(html_title.contains("Last.fm track: Believe — Cher | Last.fm"));
		assert!(html_title.contains("Listeners: 1,163,524"));
	}

	#[test]
	fn builds_mdn_and_livejournal_titles() {
		let mdn = r#"<title>runtime.onMessage - Mozilla | MDN</title>
<meta name="description" content="Use this event to listen for messages from another part of your extension.">"#;
		assert_eq!(
			mdn_title_from_html(mdn).as_deref(),
			Some(
				"MDN: runtime.onMessage - Mozilla | MDN\nSummary: Use this event to listen for messages from another part of your extension."
			)
		);

		let livejournal = r#"<title>Post title</title>
<meta property="og:description" content="Post intro text.">
<a>42 comments</a>"#;
		assert_eq!(
			livejournal_title_from_html(livejournal).as_deref(),
			Some("LiveJournal: Post title\nText: Post intro text.\nComments: 42")
		);
	}

	#[test]
	fn builds_habr_user_title() {
		let url = Url::parse("https://habr.com/en/users/zdanevich-vitaly/").unwrap();
		assert_eq!(
			habr_user_profile(&url),
			Some(("zdanevich-vitaly".into(), "en"))
		);

		let card = serde_json::json!({
			"alias": "zdanevich-vitaly",
			"fullname": "Vitaly Zdanevich",
			"rating": 0.8,
			"registerDateTime": "2006-07-28T05:42:49+00:00",
			"counterStats": {
				"commentCount": 177,
				"publicationStats": {
					"articleCount": 32,
					"postCount": 14,
					"newsCount": 3
				}
			}
		});

		assert_eq!(
			habr_user_title_from_card(&card).as_deref(),
			Some(
				"Habr user: Vitaly Zdanevich (@zdanevich-vitaly)\nArticles: 32\nPosts: 14\nNews: 3\nComments: 177\nRegistration date: 2006-07-28T05:42:49+00:00\nRating: 0.8"
			)
		);
	}

	#[test]
	fn builds_telegram_channel_title() {
		let channel = Url::parse("https://t.me/telegram").unwrap();
		let preview = Url::parse("https://t.me/s/telegram").unwrap();
		let post = Url::parse("https://t.me/telegram/123").unwrap();
		assert_eq!(telegram_channel(&channel).as_deref(), Some("telegram"));
		assert_eq!(telegram_channel(&preview).as_deref(), Some("telegram"));
		assert!(telegram_channel(&post).is_none());

		let html = r#"<meta property="og:description" content="The official Telegram on Telegram.">
<div class="tgme_page_title" dir="auto"><span dir="auto">Telegram News</span><i>ok</i></div>
<div class="tgme_page_extra">10 169 937 subscribers</div>"#;
		assert_eq!(
			telegram_channel_title_from_html(html).as_deref(),
			Some(
				"Telegram channel: Telegram News\nSubscribers: 10 169 937\nDescription: The official Telegram on Telegram."
			)
		);
	}

	#[test]
	fn builds_google_drive_folder_file_list_title() {
		let folder = Url::parse("https://drive.google.com/drive/folders/abcDEF_123-456").unwrap();
		let html = r#"<title>Project files - Google Drive</title>
<div class="flip-entry-title">notes.txt</div>
<div class="flip-entry-title">image.png</div>
<button aria-label="More actions"></button>"#;

		assert_eq!(
			google_drive_folder_id(&folder).as_deref(),
			Some("abcDEF_123-456")
		);
		assert_eq!(
			google_drive_folder_title_from_html(html).as_deref(),
			Some("Google Drive folder: Project files\nFiles:\n- notes.txt\n- image.png")
		);
	}

	#[test]
	fn builds_stackoverflow_question_title() {
		let question =
			Url::parse("https://stackoverflow.com/questions/11227809/why-is-processing-a-sorted-array-faster-than-processing-an-unsorted-array")
				.unwrap();
		let short = Url::parse("https://stackoverflow.com/q/11227809").unwrap();
		assert_eq!(stackoverflow_question_id(&question), Some(11_227_809));
		assert_eq!(stackoverflow_question_id(&short), Some(11_227_809));

		let metadata = serde_json::json!({
			"title": "Why is processing a sorted array faster than processing an unsorted array?",
			"score": 27804,
			"answer_count": 28,
			"accepted_answer_id": 11227902,
			"view_count": 1960000,
			"tags": ["java", "c++", "performance"],
			"owner": {"display_name": "GManNickG"},
			"creation_date": 1339078386,
			"last_activity_date": 1710000000
		});
		let title = stackoverflow_title_from_question(&metadata).unwrap();
		assert!(title.contains("Stack Overflow: Why is processing"));
		assert!(title.contains("Score: 27804"));
		assert!(title.contains("Answers: 28"));
		assert!(title.contains("Accepted answer: yes"));
		assert!(title.contains("Tags: java, c++, performance"));
		assert!(title.contains("Created: 2012-06-07"));
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
