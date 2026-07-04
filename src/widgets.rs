//! Expand supported external links into static-site widgets.

use regex::Regex;
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
		WidgetProvider::Spotify => format!(r#"{{{{ spotify(url="{original}") }}}}"#),
		WidgetProvider::ApplePodcasts => format!(r#"{{{{ apple_podcast(url="{original}") }}}}"#),
		WidgetProvider::YandexMusic => format!(r#"{{{{ yandex_music(url="{original}") }}}}"#),
		WidgetProvider::Instagram => format!(r#"{{{{ instagram(url="{original}") }}}}"#),
		WidgetProvider::Pinterest => format!(r#"{{{{ pinterest(url="{original}") }}}}"#),
		_ => generic_embed(provider, original),
	}
}

fn generic_embed(provider: WidgetProvider, original: &str) -> String {
	format!(
		r#"<p class="embed-link"><a href="{original}" rel="noopener">{}</a></p>"#,
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
	fn lists_extra_widget_ideas() {
		assert!(supported_widget_names().contains(&"Bandcamp"));
		assert!(supported_widget_names().contains(&"TikTok"));
		assert!(supported_widget_names().contains(&"Internet Archive"));
	}
}
