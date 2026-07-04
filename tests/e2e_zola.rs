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
	assert_contains(&index, "search_index.en.js");
	assert_contains(&index, "rss.xml");
	assert_contains(&index, "podcast.xml");

	let first_post = read(&public, "posts/hello-from-evernote/index.html");
	assert_contains(&first_post, "href=/posts/linked-note/");
	assert_contains(&first_post, "src=photo.jpg");
	assert_contains(&first_post, "href=archive.pdf");

	let media_post = read(&public, "posts/media-note/index.html");
	assert_contains(&media_post, "<audio controls");
	assert_contains(&media_post, "episode.mp3");
	assert_contains(&media_post, "<video controls");
	assert_contains(&media_post, "clip.mp4");

	let calendar = read(&public, "calendar/index.html");
	assert_contains(&calendar, "/day/2023-11-14");
	assert!(public.join("day/2023-11-14/index.html").exists());

	let about = read(&public, "about/index.html");
	assert_contains(&about, "I use Evernote from 2009 and love it.");

	assert!(public.join("sitemap.xml").exists());
	assert!(public.join("rss.xml").exists());
	assert!(public.join("podcast.xml").exists());
	assert!(public.join("tags/intro/index.html").exists());
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
			enml: "<en-note><p>Linked from another note.</p></en-note>".into(),
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
