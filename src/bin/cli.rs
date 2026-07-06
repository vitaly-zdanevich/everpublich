//! Local Everpublich command-line helper.

use anyhow::Result;
use clap::{Parser, Subcommand};
use everpublich::evernote::{notes_to_posts, utc};
use everpublich::evernote_api::{
	DEFAULT_USER_STORE_URL, EvernoteApiClient, LinkedNotebookFailure, LinkedNotebookProbe,
	LinkedNotebookSummary,
};
use everpublich::evernote_cache::{RebuildOptions, rebuild_all};
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
	/// Probe notebooks shared to the token owner's Evernote account.
	EvernoteSharedNotebooks {
		/// Developer token for the account that receives shared notebooks.
		#[arg(long, env = "EVERNOTE_DEVELOPER_TOKEN", hide_env_values = true)]
		token: String,
		/// Evernote UserStore URL.
		#[arg(long, default_value = DEFAULT_USER_STORE_URL)]
		user_store_url: String,
		/// Optional NoteStore URL when you already know the shard endpoint.
		#[arg(long)]
		note_store_url: Option<String>,
		/// Number of note metadata rows to fetch from each shared notebook.
		#[arg(long, default_value_t = 5)]
		max_sample_notes: i32,
	},
	/// Rebuild all websites from the official Evernote desktop cache.
	RebuildAll {
		/// SQLite database with Everpublich users and settings.
		#[arg(long)]
		database: PathBuf,
		/// Evernote config directory owned by the service user.
		#[arg(long)]
		evernote_config_dir: PathBuf,
		/// Root directory where per-site Zola trees are generated.
		#[arg(long)]
		sites_dir: PathBuf,
		/// Future wildcard domain used after DNS is configured.
		#[arg(
			long,
			env = "EVERPUBLICH_BASE_DOMAIN",
			default_value = "everpublich.my"
		)]
		base_domain: String,
		/// Current CloudFront URL used before wildcard DNS exists.
		#[arg(long, env = "EVERPUBLICH_CLOUDFRONT_URL")]
		cloudfront_url: Option<String>,
	},
}

fn main() -> Result<()> {
	let cli = Cli::parse();
	match cli.command {
		Command::MockSite {
			output,
			base_domain,
		} => mock_site(output, &base_domain),
		Command::EvernoteSharedNotebooks {
			token,
			user_store_url,
			note_store_url,
			max_sample_notes,
		} => evernote_shared_notebooks(token, user_store_url, note_store_url, max_sample_notes),
		Command::RebuildAll {
			database,
			evernote_config_dir,
			sites_dir,
			base_domain,
			cloudfront_url,
		} => rebuild_all_sites(
			database,
			evernote_config_dir,
			sites_dir,
			base_domain,
			cloudfront_url,
		),
	}
}

fn rebuild_all_sites(
	database: PathBuf,
	evernote_config_dir: PathBuf,
	sites_dir: PathBuf,
	base_domain: String,
	cloudfront_url: Option<String>,
) -> Result<()> {
	let summary = rebuild_all(&RebuildOptions {
		database,
		evernote_config_dir,
		sites_dir,
		base_domain,
		cloudfront_url,
	})?;
	println!(
		"Rebuild finished: {} notebook(s), {} note(s), {} built, {} failed",
		summary.notebooks_seen, summary.notes_seen, summary.sites_built, summary.sites_failed
	);
	Ok(())
}

fn evernote_shared_notebooks(
	token: String,
	user_store_url: String,
	note_store_url: Option<String>,
	max_sample_notes: i32,
) -> Result<()> {
	let client = EvernoteApiClient::new(token, Some(user_store_url), note_store_url)?;
	let probes = client.linked_notebook_probes(max_sample_notes)?;
	println!("Found {} linked/shared notebooks", probes.len());
	for probe in probes {
		match probe {
			LinkedNotebookProbe::Accessible(notebook) => print_linked_notebook(&notebook),
			LinkedNotebookProbe::Failed(failure) => print_failed_linked_notebook(&failure),
		}
	}
	Ok(())
}

fn print_linked_notebook(notebook: &LinkedNotebookSummary) {
	let name = notebook.share_name.as_deref().unwrap_or("(unnamed)");
	let owner = notebook
		.owner_username
		.as_deref()
		.unwrap_or("(unknown owner)");
	let privilege = notebook
		.privilege
		.as_deref()
		.unwrap_or("(unknown privilege)");
	println!("- {name}");
	println!("  owner: {owner}");
	println!("  notebook_guid: {}", notebook.notebook_guid);
	println!("  privilege: {privilege}");
	println!("  notebook_modifiable: {}", notebook.notebook_modifiable);
	println!("  total_notes: {}", notebook.total_notes);
	if notebook.sample_notes.is_empty() {
		println!("  sample_notes: none");
	} else {
		println!("  sample_notes:");
		for note in &notebook.sample_notes {
			let title = note.title.as_deref().unwrap_or("(untitled)");
			println!("    - {title} [{}]", note.guid);
		}
	}
}

fn print_failed_linked_notebook(failure: &LinkedNotebookFailure) {
	let name = failure.share_name.as_deref().unwrap_or("(unnamed)");
	let owner = failure
		.owner_username
		.as_deref()
		.unwrap_or("(unknown owner)");
	println!("- {name}");
	println!("  owner: {owner}");
	println!("  status: failed");
	println!(
		"  has_shared_notebook_global_id: {}",
		failure.has_shared_notebook_global_id
	);
	println!("  has_uri: {}", failure.has_uri);
	println!("  has_note_store_url: {}", failure.has_note_store_url);
	println!("  error: {}", failure.error);
}

fn mock_site(output: PathBuf, base_domain: &str) -> Result<()> {
	let settings = SiteSettings::new("Everpublich Demo", base_domain);
	let user = UserItem {
		user_id: "demo".into(),
		registration_date: utc(1_700_000_000),
		evernote_user_id: Some("demo".into()),
		evernote_access_mode: EvernoteAccessMode::SharedToServiceAccount,
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
                original_file_name: None,
                mime: "audio/mpeg".into(),
                s3_key: None,
                text_preview: None,
                archive_tree: None,
				size_bytes: None,
            }],
        },
        Note {
            guid: "33333333-3333-3333-3333-333333333333".into(),
            title: "everpublich:about".into(),
            created: utc(1_700_172_800),
            updated: utc(1_700_172_800),
            tags: vec![],
            enml: r#"<en-note><p>I use Evernote from 2009 and love it.</p></en-note>"#.into(),
            resources: vec![],
        },
    ]
}
