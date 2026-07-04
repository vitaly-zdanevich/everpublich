//! Local Everpublich command-line helper.

use anyhow::Result;
use clap::{Parser, Subcommand};
use everpublich::evernote::{notes_to_posts, utc};
use everpublich::models::{BuildState, EvernoteAccessMode, Note, Resource, SiteSettings, UserItem};
use everpublich::zola::write_zola_site;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version, about = "Everpublich local tools")]
struct Cli {
	#[command(subcommand)]
	command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
	/// Generate a mock Zola site for visual/manual testing.
	MockSite {
		/// Output directory for Zola source.
		#[arg(long, default_value = "build/mock-site")]
		output: PathBuf,
		/// Base domain used to construct the sample site URL.
		#[arg(long, default_value = "everpublich.example")]
		base_domain: String,
	},
}

fn main() -> Result<()> {
	let cli = Cli::parse();
	match cli.command {
		Command::MockSite {
			output,
			base_domain,
		} => mock_site(output, &base_domain),
	}
}

fn mock_site(output: PathBuf, base_domain: &str) -> Result<()> {
	let settings = SiteSettings::new("Everpublich Demo", base_domain);
	let user = UserItem {
		user_id: "demo".into(),
		registration_date: utc(1_700_000_000),
		evernote_user_id: Some("demo".into()),
		evernote_access_mode: EvernoteAccessMode::UserOauth,
		evernote_token: None,
		github_token: None,
		settings,
		build: BuildState::default(),
		deleted_at: None,
	};
	let notes = mock_notes();
	let posts = notes_to_posts(&notes, true);
	let generated = write_zola_site(&output, &user, &posts)?;
	println!(
		"Wrote {} posts, {} pages, {} podcast items to {}",
		generated.posts,
		generated.pages,
		generated.podcast_items,
		output.canonicalize().unwrap_or(output).display()
	);
	Ok(())
}

fn mock_notes() -> Vec<Note> {
	vec![
        Note {
            guid: "11111111-1111-1111-1111-111111111111".into(),
            title: "Hello from Evernote".into(),
            created: utc(1_700_000_000),
            updated: utc(1_700_000_000),
            tags: vec!["intro".into()],
            enml: r#"<en-note><p><span style="font-size: 20px; color: #207a4d">Rich ENML formatting is preserved.</span></p><p>https://youtu.be/dQw4w9WgXcQ</p><table><tr><td>Tables</td><td>work</td></tr></table></en-note>"#.into(),
            resources: vec![],
        },
        Note {
            guid: "22222222-2222-2222-2222-222222222222".into(),
            title: "Podcast episode".into(),
            created: utc(1_700_086_400),
            updated: utc(1_700_086_400),
            tags: vec!["podcast".into(), "audio".into()],
            enml: r#"<en-note><p>Audio notes become a podcast item.</p><en-media type="audio/mpeg" hash="abc"/></en-note>"#.into(),
            resources: vec![Resource {
                hash: "abc".into(),
                file_name: "episode.mp3".into(),
                mime: "audio/mpeg".into(),
                s3_key: None,
            }],
        },
        Note {
            guid: "33333333-3333-3333-3333-333333333333".into(),
            title: "About".into(),
            created: utc(1_700_172_800),
            updated: utc(1_700_172_800),
            tags: vec!["about".into()],
            enml: r#"<en-note><p>I use Evernote from 2009 and love it.</p></en-note>"#.into(),
            resources: vec![],
        },
    ]
}
