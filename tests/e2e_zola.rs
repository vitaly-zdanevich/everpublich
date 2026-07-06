use everpublich::evernote::{notes_to_posts, utc};
use everpublich::models::{BuildState, EvernoteAccessMode, Note, Resource, SiteSettings, UserItem};
use everpublich::zola::write_zola_site;
use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn zola_build_renders_public_html() {
	let dir = tempfile::tempdir().unwrap();
	let user = user_fixture();
	let posts = notes_to_posts(&note_fixtures(), true);

	let generated = write_zola_site(dir.path(), &user, &posts).unwrap();
	assert_eq!(generated.posts, 3);
	assert_eq!(generated.pages, 1);
	assert_eq!(generated.podcast_items, 1);

	let output = Command::new("zola")
		.arg("--root")
		.arg(dir.path())
		.arg("build")
		.output()
		.expect("zola binary must be installed for the end-to-end HTML test");
	assert!(
		output.status.success(),
		"zola build failed\nstdout:\n{}\nstderr:\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);

	let public = dir.path().join("public");
	let index = read(&public, "index.html");
	assert_contains(&index, "Rich ENML formatting is preserved.");
	assert_contains(&index, "www.youtube.com/embed/dQw4w9WgXcQ");
	assert_contains(&index, "id=site-search");
	assert_contains(&index, "<ul hidden id=search-results>");
	assert_contains(&index, "search_index.en.js");
	assert_contains(&index, "search_metadata.js");
	assert_contains(&index, "rss.xml");
	assert_contains(&index, "podcast.xml");
	assert_contains(&index, ">About<");
	assert_contains(&index, ">Tags<");
	assert_not_contains(&index, ">RSS<");

	let search_js = read(&public, "search.js");
	assert_contains(&search_js, "documentStore.docs");
	assert_contains(&search_js, "everpublichSearchMetadata");
	assert_contains(&search_js, ".title=");
	let search_metadata = read(&public, "search_metadata.js");
	assert_contains(
		&search_metadata,
		"\"https://my-notebook.everpublich.example/posts/hello-from-evernote/\":{\"date\":\"2023-11-14\"}",
	);
	let style = read(&public, "style.css");
	assert_contains(&style, ".search:focus-within");
	assert_contains(&style, "transition:width .12s");
	assert_contains(&style, "list-style:none");
	assert_contains(&style, "#search-results li:hover");
	assert_contains(&style, ".post-nav");
	assert_contains(&style, "min-height:100vh");
	assert_contains(
		&style,
		"main{flex-direction:column;flex:1 0 auto;display:flex}",
	);
	assert_contains(&style, "margin:auto 0 0;padding-top:36px");
	assert_contains(&style, "::selection");
	assert_contains(&style, "background-color:#292");

	let first_post = read(&public, "posts/hello-from-evernote/index.html");
	assert_contains(
		&first_post,
		"href=https://my-notebook.everpublich.example/day/2023-11-14",
	);
	assert_contains(&first_post, "title=22:13:20");
	assert_contains(&first_post, "href=/posts/linked-note/");
	assert_contains(&first_post, "src=photo.jpg");
	assert_contains(&first_post, "href=archive.pdf");
	assert_contains(&first_post, "post-nav");
	assert_contains(
		&first_post,
		"Linked Note\nLinked from another note.\nSecond tooltip line.\nQuoted tooltip line\nMore quote.",
	);
	let linked_post = read(&public, "posts/linked-note/index.html");
	assert_contains(&linked_post, ">Newer<");
	assert_contains(&linked_post, ">Older<");
	assert_not_contains(&linked_post, ">Previous<");
	assert_not_contains(&linked_post, ">Next<");

	let media_post = read(&public, "posts/media-note/index.html");
	assert_contains(&media_post, "<audio controls");
	assert_contains(&media_post, "episode.mp3");
	assert_contains(&media_post, "<video controls");
	assert_contains(&media_post, "clip.mp4");

	let calendar = read(&public, "calendar/index.html");
	assert_contains(
		&calendar,
		"<h3><a href=https://my-notebook.everpublich.example/month/2023-11/>November</a></h3>",
	);
	assert_not_contains(&calendar, "<h3>11</h3>");
	assert_contains(&calendar, "calendar-days");
	assert_contains(&calendar, "calendar-day--one");
	assert_contains(&calendar, "title=\"1 post\n\nHello from Evernote\"");
	assert_not_contains(&calendar, "<h1>Calendar</h1>");
	assert_contains(&calendar, "<span>Calendar</span>");
	assert_not_contains(&calendar, "<ol>");
	assert_not_contains(&calendar, "<li>");
	assert_not_contains(&calendar, "<span>1</span>");
	assert_contains(
		&calendar,
		"https://my-notebook.everpublich.example/day/2023-11-14/",
	);
	let month = read(&public, "month/2023-11/index.html");
	assert_contains(&month, "<h1>November 2023</h1>");
	assert_contains(&month, "Rich ENML formatting is preserved.");
	assert_contains(&month, "Audio and video stay playable.");

	let day = read(&public, "day/2023-11-14/index.html");
	assert_contains(&day, "Rich ENML formatting is preserved.");
	assert_contains(&day, "aria-label=\"Day navigation\"");
	assert_contains(
		&day,
		"href=https://my-notebook.everpublich.example/day/2023-11-15/",
	);
	assert_contains(&day, "title=\"November 15\n\nLinked Note\"");
	assert_contains(&day, "<strong>November 15</strong>");
	assert_contains(
		&day,
		"href=https://my-notebook.everpublich.example/posts/hello-from-evernote/",
	);

	let about = read(&public, "about/index.html");
	assert_contains(&about, "I use Evernote from 2009 and love it.");
	assert_not_contains(&about, "post-nav");

	assert!(public.join("sitemap.xml").exists());
	assert!(public.join("rss.xml").exists());
	assert!(public.join("podcast.xml").exists());
	assert!(public.join("tags/intro/index.html").exists());

	let podcast = read(&public, "podcast.xml");
	assert_contains(
		&podcast,
		r#"<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">"#,
	);
	assert_contains(&podcast, "<language>en</language>");
	assert_contains(&podcast, "<generator>Everpublich</generator>");
	assert_contains(&podcast, "<itunes:explicit>false</itunes:explicit>");
	assert_contains(
		&podcast,
		"<guid isPermaLink=\"true\">https://my-notebook.everpublich.example/posts/media-note/</guid>",
	);
	assert_contains(
		&podcast,
		"<description>Audio and video stay playable.</description>",
	);
	assert_contains(
		&podcast,
		"enclosure url=\"https://my-notebook.everpublich.example/posts/media-note/episode.mp3\" type=\"audio/mpeg\" length=\"0\"",
	);
}

#[test]
fn zola_build_omits_about_link_without_about_page() {
	let dir = tempfile::tempdir().unwrap();
	let user = user_fixture();
	let notes = note_fixtures()
		.into_iter()
		.filter(|note| !note.tags.iter().any(|tag| tag == "about"))
		.collect::<Vec<_>>();
	let posts = notes_to_posts(&notes, true);

	let generated = write_zola_site(dir.path(), &user, &posts).unwrap();
	assert_eq!(generated.pages, 0);

	let output = Command::new("zola")
		.arg("--root")
		.arg(dir.path())
		.arg("build")
		.output()
		.expect("zola binary must be installed for the end-to-end HTML test");
	assert!(
		output.status.success(),
		"zola build failed\nstdout:\n{}\nstderr:\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);

	let public = dir.path().join("public");
	let index = read(&public, "index.html");
	assert_not_contains(&index, ">About<");
	assert!(!public.join("about/index.html").exists());
}

#[test]
fn zola_build_omits_tags_link_without_public_tags() {
	let dir = tempfile::tempdir().unwrap();
	let user = user_fixture();
	let mut notes = note_fixtures();
	for note in &mut notes {
		note.tags.clear();
	}
	let posts = notes_to_posts(&notes, true);

	let generated = write_zola_site(dir.path(), &user, &posts).unwrap();
	assert_eq!(generated.posts, 4);
	assert_eq!(generated.pages, 0);

	let output = Command::new("zola")
		.arg("--root")
		.arg(dir.path())
		.arg("build")
		.output()
		.expect("zola binary must be installed for the end-to-end HTML test");
	assert!(
		output.status.success(),
		"zola build failed\nstdout:\n{}\nstderr:\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);

	let public = dir.path().join("public");
	let index = read(&public, "index.html");
	assert_not_contains(&index, ">Tags<");
	let tags = read(&public, "tags/index.html");
	assert_contains(&tags, "No tags found in the synced Evernote cache.");
}

fn user_fixture() -> UserItem {
	UserItem {
		user_id: "user-1".into(),
		registration_date: utc(1_700_000_000),
		evernote_user_id: Some("evernote-user-1".into()),
		evernote_access_mode: EvernoteAccessMode::UserOauth,
		evernote_token: None,
		github_token: None,
		settings: SiteSettings::new("My Notebook", "everpublich.example"),
		build: BuildState::default(),
		deleted_at: None,
	}
}

fn note_fixtures() -> Vec<Note> {
	vec![
		Note {
			guid: "11111111-1111-1111-1111-111111111111".into(),
			title: "Hello from Evernote".into(),
			created: utc(1_700_000_000),
			updated: utc(1_700_000_100),
			tags: vec!["intro".into()],
			enml: r#"<en-note><p><span style="font-size: 20px; color: #207a4d">Rich ENML formatting is preserved.</span></p><p><a href="evernote:///view/1/s1/22222222-2222-2222-2222-222222222222/22222222-2222-2222-2222-222222222222/">Linked note</a></p><p>https://youtu.be/dQw4w9WgXcQ</p><en-media type="image/jpeg" hash="img"/><en-media type="application/pdf" hash="pdf"/></en-note>"#.into(),
			resources: vec![
				Resource {
					hash: "img".into(),
					file_name: "photo.jpg".into(),
					mime: "image/jpeg".into(),
					s3_key: None,
				},
				Resource {
					hash: "pdf".into(),
					file_name: "archive.pdf".into(),
					mime: "application/pdf".into(),
					s3_key: None,
				},
			],
		},
		Note {
			guid: "22222222-2222-2222-2222-222222222222".into(),
			title: "Linked Note".into(),
			created: utc(1_700_086_400),
			updated: utc(1_700_086_500),
			tags: vec!["reference".into()],
			enml: "<en-note><p>Linked from another note.</p><p>Second tooltip line.</p><blockquote>Quoted tooltip line<br/>More quote.</blockquote></en-note>".into(),
			resources: vec![],
		},
		Note {
			guid: "33333333-3333-3333-3333-333333333333".into(),
			title: "Media note".into(),
			created: utc(1_700_172_800),
			updated: utc(1_700_172_900),
			tags: vec!["podcast".into(), "media".into()],
			enml: r#"<en-note><p>Audio and video stay playable.</p><en-media type="audio/mpeg" hash="audio"/><en-media type="video/mp4" hash="video"/></en-note>"#.into(),
			resources: vec![
				Resource {
					hash: "audio".into(),
					file_name: "episode.mp3".into(),
					mime: "audio/mpeg".into(),
					s3_key: None,
				},
				Resource {
					hash: "video".into(),
					file_name: "clip.mp4".into(),
					mime: "video/mp4".into(),
					s3_key: None,
				},
			],
		},
		Note {
			guid: "44444444-4444-4444-4444-444444444444".into(),
			title: "About".into(),
			created: utc(1_700_259_200),
			updated: utc(1_700_259_300),
			tags: vec!["about".into()],
			enml: "<en-note><p>I use Evernote from 2009 and love it.</p></en-note>".into(),
			resources: vec![],
		},
	]
}

fn read(public: &Path, relative: &str) -> String {
	fs::read_to_string(public.join(relative)).unwrap()
}

fn assert_contains(haystack: &str, needle: &str) {
	assert!(
		haystack.contains(needle),
		"expected generated HTML to contain {needle:?}\n\n{haystack}"
	);
}

fn assert_not_contains(haystack: &str, needle: &str) {
	assert!(
		!haystack.contains(needle),
		"expected generated HTML not to contain {needle:?}\n\n{haystack}"
	);
}
