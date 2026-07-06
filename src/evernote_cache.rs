//! Reader and builder for the official Evernote desktop cache.
//!
//! Evernote no longer issues public API credentials for this SaaS shape, so the
//! single-VM MVP runs the official client and treats its local SQLite cache as a
//! read-only sync source. This module keeps that coupling isolated from the
//! renderer and from the admin database.

use crate::evernote::notes_to_posts;
use crate::models::{
	BuildState, EvernoteAccessMode, IndexMode, Note, Post, PostKind, Resource, SearchMode,
	SiteSettings, UserItem,
};
use crate::slug::{slug_from_title_and_tags, slugify};
use crate::store::SqliteUserRow;
use crate::zola::{GeneratedSite, write_zola_site};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, TimeZone, Utc};
use html_escape::{encode_double_quoted_attribute, encode_text};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Runtime inputs for rebuilding every generated website.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildOptions {
	/// Everpublich SQLite database path.
	pub database: PathBuf,
	/// Evernote desktop config/cache directory.
	pub evernote_config_dir: PathBuf,
	/// Root directory for generated per-site Zola source trees.
	pub sites_dir: PathBuf,
	/// Future wildcard domain for user websites.
	pub base_domain: String,
	/// CloudFront URL used while wildcard DNS is not configured.
	pub cloudfront_url: Option<String>,
}

/// Count summary returned by a full rebuild.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebuildSummary {
	/// Number of Evernote notebooks discovered.
	pub notebooks_seen: usize,
	/// Number of Evernote notes discovered.
	pub notes_seen: usize,
	/// Number of Zola sites built successfully.
	pub sites_built: usize,
	/// Number of Zola sites that failed.
	pub sites_failed: usize,
}

#[derive(Debug, Clone)]
struct CacheSite {
	user: UserItem,
	notes: Vec<CachedNote>,
}

#[derive(Debug, Clone)]
struct CachedNote {
	note: Note,
	files: Vec<CachedResourceFile>,
}

#[derive(Debug, Clone)]
struct CachedResourceFile {
	file_name: String,
	source_path: PathBuf,
}

#[derive(Debug, Clone)]
struct CachedAttachment {
	resource: Resource,
	source_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct CacheNotebook {
	id: String,
	label: String,
}

#[derive(Debug, Clone)]
struct CacheNoteRow {
	id: String,
	title: String,
	created_millis: i64,
	updated_millis: i64,
	content: String,
	snippet: String,
}

#[derive(Debug, Clone)]
struct StoredSiteSettings {
	site_slug: String,
	site_title: String,
	index_mode: IndexMode,
	search_mode: SearchMode,
	custom_css: Option<String>,
	google_analytics_id: Option<String>,
	yandex_metrica_id: Option<String>,
	expand_widgets: bool,
}

/// Rebuild all Zola websites from the current Evernote cache and persist build
/// status in the Everpublich SQLite database.
pub fn rebuild_all(options: &RebuildOptions) -> Result<RebuildSummary> {
	fs::create_dir_all(&options.sites_dir)
		.with_context(|| format!("failed to create {}", options.sites_dir.display()))?;

	let app_db = Connection::open(&options.database)
		.with_context(|| format!("failed to open {}", options.database.display()))?;
	let graph_dbs = discover_remote_graphs(&options.evernote_config_dir)?;
	if graph_dbs.is_empty() {
		bail!(
			"no Evernote RemoteGraph SQLite cache found under {}",
			options.evernote_config_dir.display()
		);
	}

	let mut sites = Vec::new();
	for graph_db in graph_dbs {
		sites.extend(read_cache_sites(options, &app_db, &graph_db)?);
	}
	let allowed_slugs = sites
		.iter()
		.map(|site| site.user.settings.subdomain.clone())
		.collect::<BTreeSet<_>>();
	prune_generated_sites(&options.sites_dir, &allowed_slugs)?;

	let mut summary = RebuildSummary::default();
	for site in &mut sites {
		summary.notebooks_seen += 1;
		summary.notes_seen += site.notes.len();

		upsert_user(&app_db, &site.user)?;
		let build_id = start_build(&app_db, &site.user.user_id, site.notes.len())?;
		match build_site(options, site) {
			Ok((generated, posts)) => {
				replace_note_index(&app_db, &site.user.user_id, &posts)?;
				finish_build(&app_db, build_id, "succeeded", None)?;
				summary.sites_built += 1;
				println!(
					"Built {} at {} ({} posts, {} pages, {} podcast items)",
					site.user.settings.title,
					site.user.settings.base_url,
					generated.posts,
					generated.pages,
					generated.podcast_items
				);
			}
			Err(error) => {
				finish_build(&app_db, build_id, "failed", Some(&error.to_string()))?;
				summary.sites_failed += 1;
				eprintln!("Failed to build {}: {error:#}", site.user.settings.title);
			}
		}
	}

	Ok(summary)
}

fn prune_generated_sites(sites_dir: &Path, allowed_slugs: &BTreeSet<String>) -> Result<()> {
	if !sites_dir.exists() {
		return Ok(());
	}

	for entry in fs::read_dir(sites_dir)
		.with_context(|| format!("failed to read {}", sites_dir.display()))?
	{
		let entry = entry?;
		if !entry.file_type()?.is_dir() {
			continue;
		}
		let slug = entry.file_name().to_string_lossy().to_string();
		if !allowed_slugs.contains(&slug) {
			fs::remove_dir_all(entry.path()).with_context(|| {
				format!(
					"failed to remove stale generated site {}",
					entry.path().display()
				)
			})?;
			println!("Removed stale generated site {slug}");
		}
	}
	Ok(())
}

fn discover_remote_graphs(config_dir: &Path) -> Result<Vec<PathBuf>> {
	let storage_dir = config_dir.join("conduit-storage");
	if !storage_dir.exists() {
		return Ok(Vec::new());
	}

	let mut graph_dbs = Vec::new();
	for service_dir in fs::read_dir(&storage_dir)
		.with_context(|| format!("failed to read {}", storage_dir.display()))?
	{
		let service_dir = service_dir?;
		if !service_dir.file_type()?.is_dir() {
			continue;
		}
		for entry in fs::read_dir(service_dir.path())? {
			let entry = entry?;
			let path = entry.path();
			let name = entry.file_name();
			let name = name.to_string_lossy();
			if entry.file_type()?.is_file() && name.ends_with("+RemoteGraph.sql") {
				graph_dbs.push(path);
			}
		}
	}
	graph_dbs.sort();
	Ok(graph_dbs)
}

fn read_cache_sites(
	options: &RebuildOptions,
	app_db: &Connection,
	graph_db: &Path,
) -> Result<Vec<CacheSite>> {
	let account_label = account_label_from_db_path(graph_db);
	let cache_db = Connection::open_with_flags(graph_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
		.with_context(|| format!("failed to open Evernote cache {}", graph_db.display()))?;
	cache_db.execute_batch("pragma query_only = on;")?;

	let notebooks = read_notebooks(&cache_db)?;
	let mut sites = Vec::new();
	for notebook in notebooks {
		let mut settings = SiteSettings::new(&notebook.label, &options.base_domain);
		settings.notebook_guid = Some(notebook.id.clone());
		settings.notebook_name = Some(notebook.label.clone());
		settings.base_url = public_base_url(
			&settings.subdomain,
			&options.base_domain,
			options.cloudfront_url.as_deref(),
		);

		let mut user = UserItem {
			user_id: format!("evernote-cache-{}-{}", slugify(&account_label), notebook.id),
			registration_date: Utc::now(),
			evernote_user_id: Some(account_label.clone()),
			evernote_access_mode: EvernoteAccessMode::SharedToServiceAccount,
			evernote_token: None,
			github_token: None,
			settings,
			build: BuildState::default(),
			deleted_at: None,
		};
		apply_stored_settings(
			app_db,
			&mut user,
			&options.base_domain,
			options.cloudfront_url.as_deref(),
		)?;

		let mut notes = read_notes_for_notebook(
			&cache_db,
			&options.evernote_config_dir,
			&notebook.id,
			&account_label,
		)?;
		deduplicate_note_slugs(&mut notes);
		if !notes.is_empty() {
			sites.push(CacheSite { user, notes });
		}
	}
	Ok(sites)
}

fn read_notebooks(cache_db: &Connection) -> Result<Vec<CacheNotebook>> {
	let mut stmt = cache_db.prepare(
		"select n.id, n.label \
		 from Nodes_Notebook n \
		 cross join Nodes_User u \
		 where n.isExternal = 1 \
		 or exists (
		 	select 1 from Nodes_Invitation i \
		 	where i.parent_Notebook_id = n.id \
		 	and i.invitationType = 'NOTEBOOK' \
		 	and i.recipient_Profile_id = u.profile_Profile_id
		 ) \
		 or exists (
		 	select 1 from Nodes_Membership m \
		 	where m.parent_Notebook_id = n.id and m.recipientIsMe = 1
		 ) \
		 or (
		 	n.owner is not null \
		 	and u.internal_userID is not null \
		 	and cast(n.owner as integer) != cast(u.internal_userID as integer)
		 ) \
		 order by n.label collate nocase",
	)?;
	let notebooks = stmt
		.query_map([], |row| {
			Ok(CacheNotebook {
				id: row.get(0)?,
				label: row.get(1)?,
			})
		})?
		.collect::<rusqlite::Result<Vec<_>>>()?;
	Ok(notebooks)
}

fn read_notes_for_notebook(
	cache_db: &Connection,
	config_dir: &Path,
	notebook_id: &str,
	account_label: &str,
) -> Result<Vec<CachedNote>> {
	let mut stmt = cache_db.prepare(
		"select n.id, n.label, n.created, n.updated, \
		 coalesce(c.content, ''), coalesce(n.snippet, '') \
		 from Nodes_Note n \
		 left join Offline_Search_Note_Content c on c.id = n.id \
		 where n.deleted is null and n.parent_Notebook_id = ?1 \
		 order by n.created desc, n.label collate nocase",
	)?;
	let rows = stmt
		.query_map([notebook_id], |row| {
			Ok(CacheNoteRow {
				id: row.get(0)?,
				title: row.get(1)?,
				created_millis: row.get(2)?,
				updated_millis: row.get(3)?,
				content: row.get(4)?,
				snippet: row.get(5)?,
			})
		})?
		.collect::<rusqlite::Result<Vec<_>>>()?;

	rows.into_iter()
		.map(|row| {
			let tags = read_tags(cache_db, &row.id)?;
			let attachments = read_attachments(cache_db, config_dir, &row.id)?;
			let resources = attachments
				.iter()
				.map(|attachment| attachment.resource.clone())
				.collect::<Vec<_>>();
			let rte_text = read_note_rte_text(config_dir, &row.id, &row.title)?;
			let files = attachments
				.into_iter()
				.filter_map(|attachment| {
					attachment
						.source_path
						.map(|source_path| CachedResourceFile {
							file_name: attachment.resource.file_name,
							source_path,
						})
				})
				.collect();
			Ok(CachedNote {
				note: Note {
					guid: row.id,
					title: clean_title(&row.title),
					created: evernote_millis_to_utc(row.created_millis)?,
					updated: evernote_millis_to_utc(row.updated_millis)?,
					enml: plain_text_to_enml(
						&row.content,
						&row.snippet,
						rte_text.as_deref(),
						&resources,
					),
					resources,
					tags,
				},
				files,
			})
		})
		.collect::<Result<Vec<_>>>()
		.with_context(|| format!("failed to read notes for Evernote account {account_label}"))
}

fn read_tags(cache_db: &Connection, note_id: &str) -> Result<Vec<String>> {
	let mut stmt = cache_db.prepare(
		"select t.label \
		 from Nodes_Tag t \
		 inner join NoteTag nt on nt.Tag_id = t.id \
		 where nt.Note_id = ?1 \
		 order by t.label collate nocase",
	)?;
	let tags = stmt
		.query_map([note_id], |row| row.get(0))?
		.collect::<rusqlite::Result<Vec<String>>>()?;
	Ok(tags)
}

fn read_attachments(
	cache_db: &Connection,
	config_dir: &Path,
	note_id: &str,
) -> Result<Vec<CachedAttachment>> {
	let mut stmt = cache_db.prepare(
		"select dataHash, filename, mime \
		 from Attachment \
		 where parent_Note_id = ?1 and isActive = 1 \
		 order by filename collate nocase",
	)?;
	let attachments = stmt
		.query_map([note_id], |row| {
			let hash: String = row.get(0)?;
			let filename: String = row.get(1)?;
			let mime: String = row.get(2)?;
			Ok((hash, filename, mime))
		})?
		.collect::<rusqlite::Result<Vec<_>>>()?;

	attachments
		.into_iter()
		.map(|(hash, filename, mime)| {
			let file_name = safe_file_name(&filename, &hash);
			let source_path = find_cached_resource_file(config_dir, &hash)?;
			Ok(CachedAttachment {
				resource: Resource {
					hash: hash.to_ascii_lowercase(),
					file_name,
					mime,
					s3_key: None,
				},
				source_path,
			})
		})
		.collect()
}

fn find_cached_resource_file(config_dir: &Path, hash: &str) -> Result<Option<PathBuf>> {
	let root = config_dir.join("conduit-fs");
	if !root.exists() {
		return Ok(None);
	}

	let wanted = hash.to_ascii_lowercase();
	let mut stack = vec![root];
	while let Some(dir) = stack.pop() {
		for entry in
			fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
		{
			let entry = entry?;
			let path = entry.path();
			let file_type = entry.file_type()?;
			if file_type.is_dir() {
				stack.push(path);
				continue;
			}
			if !file_type.is_file() {
				continue;
			}
			let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
			if name == wanted || name == format!("{wanted}.dat") {
				return Ok(Some(path));
			}
		}
	}
	Ok(None)
}

fn read_note_rte_text(config_dir: &Path, note_id: &str, title: &str) -> Result<Option<String>> {
	let Some(path) = find_note_rte_doc_file(config_dir, note_id)? else {
		return Ok(None);
	};
	let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
	Ok(extract_rte_doc_text(&bytes, title))
}

fn find_note_rte_doc_file(config_dir: &Path, note_id: &str) -> Result<Option<PathBuf>> {
	let root = config_dir.join("conduit-fs");
	if !root.exists() {
		return Ok(None);
	}

	let wanted = format!("{note_id}.dat").to_ascii_lowercase();
	let mut stack = vec![root];
	while let Some(dir) = stack.pop() {
		for entry in
			fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
		{
			let entry = entry?;
			let path = entry.path();
			let file_type = entry.file_type()?;
			if file_type.is_dir() {
				stack.push(path);
				continue;
			}
			if !file_type.is_file() {
				continue;
			}

			let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
			if name == wanted && path_contains_component(&path, "internal_rteDoc") {
				return Ok(Some(path));
			}
		}
	}
	Ok(None)
}

fn extract_rte_doc_text(bytes: &[u8], title: &str) -> Option<String> {
	let mut candidates = printable_runs(bytes)
		.into_iter()
		.filter(|candidate| is_human_rte_candidate(candidate))
		.collect::<Vec<_>>();
	let title = clean_title(title);
	if let Some(candidate) = candidates.iter().find(|candidate| **candidate == title) {
		return Some(candidate.clone());
	}

	let title_without_first = title.chars().skip(1).collect::<String>();
	if !title_without_first.is_empty()
		&& candidates
			.iter()
			.any(|candidate| candidate == &title_without_first)
	{
		return Some(title);
	}

	candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.chars().count()));
	candidates.into_iter().next()
}

fn printable_runs(bytes: &[u8]) -> Vec<String> {
	let text = String::from_utf8_lossy(bytes);
	let mut runs = Vec::new();
	let mut current = String::new();
	for character in text.chars() {
		if character.is_control() || character == '\u{fffd}' {
			push_printable_run(&mut runs, &mut current);
		} else {
			current.push(character);
		}
	}
	push_printable_run(&mut runs, &mut current);
	runs
}

fn push_printable_run(runs: &mut Vec<String>, current: &mut String) {
	let value = normalize_rte_candidate(current.trim());
	if value.chars().count() >= 2 {
		runs.push(value);
	}
	current.clear();
}

fn normalize_rte_candidate(candidate: &str) -> String {
	if let Some(rest) = candidate.strip_prefix("y ") {
		return rest.trim().to_string();
	}
	candidate.to_string()
}

fn is_human_rte_candidate(candidate: &str) -> bool {
	let lower = candidate.to_ascii_lowercase();
	let known = [
		"br",
		"calendareventids",
		"content",
		"customnotestyles",
		"div",
		"en-note",
		"fontcolor",
		"fontfamily",
		"fontsize",
		"fontstyle",
		"fontweight",
		"headingstyles",
		"inherit",
		"isempty",
		"lastenmlnormalizationversion",
		"lineheight",
		"meta",
		"resources",
		"schemaversion",
		"taskgrouporder",
		"taskidclocksmap",
		"textdecoration",
		"title",
	];
	if known.iter().any(|word| lower.contains(word)) {
		return false;
	}
	candidate.chars().any(|character| character.is_alphabetic())
}

fn path_contains_component(path: &Path, component: &str) -> bool {
	path.components()
		.any(|part| part.as_os_str().to_string_lossy() == component)
}

fn build_site(options: &RebuildOptions, site: &CacheSite) -> Result<(GeneratedSite, Vec<Post>)> {
	let site_dir = options.sites_dir.join(&site.user.settings.subdomain);
	if site_dir.exists() {
		fs::remove_dir_all(&site_dir)
			.with_context(|| format!("failed to remove stale {}", site_dir.display()))?;
	}
	fs::create_dir_all(&site_dir)
		.with_context(|| format!("failed to create {}", site_dir.display()))?;

	let notes = site
		.notes
		.iter()
		.map(|cached| cached.note.clone())
		.collect::<Vec<_>>();
	let posts = notes_to_posts(&notes, site.user.settings.expand_widgets);
	let generated = write_zola_site(&site_dir, &site.user, &posts)?;
	copy_resource_files(&site_dir, &site.notes, &posts)?;

	let output = Command::new("zola")
		.arg("--root")
		.arg(&site_dir)
		.arg("build")
		.output()
		.with_context(|| "failed to execute zola build")?;
	if !output.status.success() {
		bail!(
			"zola build failed for {}\nstdout:\n{}\nstderr:\n{}",
			site.user.settings.title,
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
	}

	Ok((generated, posts))
}

fn copy_resource_files(site_dir: &Path, notes: &[CachedNote], posts: &[Post]) -> Result<()> {
	for (cached, post) in notes.iter().zip(posts) {
		let content_dir = match post.kind {
			PostKind::BlogPost => site_dir.join("content/posts").join(&post.slug),
			PostKind::Page => site_dir.join("content/pages").join(&post.slug),
			PostKind::About => site_dir.join("content/pages/about"),
		};
		for file in &cached.files {
			fs::copy(&file.source_path, content_dir.join(&file.file_name)).with_context(|| {
				format!(
					"failed to copy {} into {}",
					file.source_path.display(),
					content_dir.display()
				)
			})?;
		}
	}
	Ok(())
}

fn apply_stored_settings(
	app_db: &Connection,
	user: &mut UserItem,
	base_domain: &str,
	cloudfront_url: Option<&str>,
) -> Result<()> {
	let stored = app_db
		.query_row(
			"select u.site_slug, u.site_title, u.home_page_mode, \
			 s.google_analytics_id, s.yandex_metrica_id, s.custom_css, \
			 coalesce(s.expand_widgets, 1), coalesce(s.static_search_enabled, 1), \
			 coalesce(s.google_search_enabled, 0) \
			 from users u \
			 left join site_settings s on s.user_id = u.user_id \
			 where u.user_id = ?1",
			[&user.user_id],
			|row| {
				let home_page_mode: String = row.get(2)?;
				let static_search: i64 = row.get(7)?;
				let google_search: i64 = row.get(8)?;
				Ok(StoredSiteSettings {
					site_slug: row.get(0)?,
					site_title: row.get(1)?,
					index_mode: if home_page_mode == "titles_only" {
						IndexMode::TitlesOnly
					} else {
						IndexMode::FullPosts
					},
					search_mode: if google_search == 1 {
						SearchMode::Google
					} else if static_search == 1 {
						SearchMode::ZolaStatic
					} else {
						SearchMode::None
					},
					google_analytics_id: row.get(3)?,
					yandex_metrica_id: row.get(4)?,
					custom_css: row.get(5)?,
					expand_widgets: row.get::<_, i64>(6)? == 1,
				})
			},
		)
		.optional()?;

	if let Some(stored) = stored {
		user.settings.subdomain = stored.site_slug;
		user.settings.site_name = stored.site_title.clone();
		user.settings.title = stored.site_title;
		user.settings.index_mode = stored.index_mode;
		user.settings.search_mode = stored.search_mode;
		user.settings.custom_css = stored.custom_css;
		user.settings.google_analytics_id = stored.google_analytics_id;
		user.settings.yandex_metrica_id = stored.yandex_metrica_id;
		user.settings.expand_widgets = stored.expand_widgets;
		user.settings.base_url =
			public_base_url(&user.settings.subdomain, base_domain, cloudfront_url);
	}

	Ok(())
}

fn upsert_user(app_db: &Connection, user: &UserItem) -> Result<()> {
	let row = SqliteUserRow::from_user(user);
	app_db.execute(
		"insert into users (
			user_id, site_slug, site_title, registration_date_utc,
			evernote_account_label, shared_notebook_guid, shared_notebook_name,
			home_page_mode, public_base_url, github_repository_visibility, updated_at_utc
		)
		values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
		on conflict(user_id) do update set
			site_slug = excluded.site_slug,
			site_title = excluded.site_title,
			evernote_account_label = excluded.evernote_account_label,
			shared_notebook_guid = excluded.shared_notebook_guid,
			shared_notebook_name = excluded.shared_notebook_name,
			home_page_mode = excluded.home_page_mode,
			public_base_url = excluded.public_base_url,
			github_repository_visibility = excluded.github_repository_visibility,
			updated_at_utc = excluded.updated_at_utc",
		params![
			row.user_id,
			row.site_slug,
			row.site_title,
			row.registration_date_utc,
			user.evernote_user_id,
			row.shared_notebook_guid,
			row.shared_notebook_name,
			row.home_page_mode.as_str(),
			row.public_base_url,
			row.github_repository_visibility
				.map(|visibility| visibility.as_str()),
			sqlite_now(),
		],
	)?;

	app_db.execute(
		"insert into site_settings (
			user_id, expand_widgets, static_search_enabled, google_search_enabled,
			custom_css, google_analytics_id, yandex_metrica_id, updated_at_utc
		)
		values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
		on conflict(user_id) do update set
			expand_widgets = excluded.expand_widgets,
			static_search_enabled = excluded.static_search_enabled,
			google_search_enabled = excluded.google_search_enabled,
			custom_css = excluded.custom_css,
			google_analytics_id = excluded.google_analytics_id,
			yandex_metrica_id = excluded.yandex_metrica_id,
			updated_at_utc = excluded.updated_at_utc",
		params![
			user.user_id,
			bool_to_int(user.settings.expand_widgets),
			bool_to_int(matches!(user.settings.search_mode, SearchMode::ZolaStatic)),
			bool_to_int(matches!(user.settings.search_mode, SearchMode::Google)),
			user.settings.custom_css,
			user.settings.google_analytics_id,
			user.settings.yandex_metrica_id,
			sqlite_now(),
		],
	)?;
	Ok(())
}

fn start_build(app_db: &Connection, user_id: &str, notes_seen: usize) -> Result<i64> {
	app_db.execute(
		"insert into build_runs (user_id, status, notes_seen) values (?1, 'running', ?2)",
		params![user_id, notes_seen as i64],
	)?;
	Ok(app_db.last_insert_rowid())
}

fn finish_build(
	app_db: &Connection,
	build_id: i64,
	status: &str,
	error_message: Option<&str>,
) -> Result<()> {
	app_db.execute(
		"update build_runs \
		 set finished_at_utc = ?1, status = ?2, error_message = ?3 \
		 where build_id = ?4",
		params![sqlite_now(), status, error_message, build_id],
	)?;
	Ok(())
}

fn replace_note_index(app_db: &Connection, user_id: &str, posts: &[Post]) -> Result<()> {
	app_db.execute("delete from note_index where user_id = ?1", [user_id])?;
	let mut stmt = app_db.prepare(
		"insert into note_index (
			user_id, note_guid, title, slug, updated_at_utc, is_page, is_podcast
		)
		values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
	)?;
	for post in posts {
		stmt.execute(params![
			user_id,
			post.guid,
			post.title,
			post.slug,
			post.date
				.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
			bool_to_int(matches!(post.kind, PostKind::Page | PostKind::About)),
			bool_to_int(
				post.tags
					.iter()
					.any(|tag| tag.eq_ignore_ascii_case("podcast"))
			),
		])?;
	}
	Ok(())
}

fn plain_text_to_enml(
	content: &str,
	snippet: &str,
	rte_text: Option<&str>,
	resources: &[Resource],
) -> String {
	let text = if !content.trim().is_empty() {
		normalize_indexed_plain_text(content)
	} else if !snippet.trim().is_empty() {
		normalize_indexed_plain_text(snippet)
	} else {
		normalize_plain_text(rte_text.unwrap_or(""))
	};
	let mut enml = String::from("<en-note>");
	for paragraph in text
		.split("\n\n")
		.map(str::trim)
		.filter(|paragraph| !paragraph.is_empty())
	{
		enml.push_str("<p>");
		for (index, line) in paragraph.lines().enumerate() {
			if index > 0 {
				enml.push_str("<br>");
			}
			enml.push_str(&encode_text(line));
		}
		enml.push_str("</p>");
	}
	for resource in resources {
		enml.push_str("<p><en-media type=\"");
		enml.push_str(&encode_double_quoted_attribute(&resource.mime));
		enml.push_str("\" hash=\"");
		enml.push_str(&encode_double_quoted_attribute(&resource.hash));
		enml.push_str("\"/></p>");
	}
	enml.push_str("</en-note>");
	enml
}

fn normalize_indexed_plain_text(text: &str) -> String {
	normalize_plain_text(text).replace("/n", "\n")
}

fn normalize_plain_text(text: &str) -> String {
	text.replace("\r\n", "\n").replace('\r', "\n")
}

fn deduplicate_note_slugs(notes: &mut [CachedNote]) {
	let mut seen = HashMap::<String, usize>::new();
	for cached in notes {
		let slug = slug_from_title_and_tags(&cached.note.title, &cached.note.tags);
		let count = seen.entry(slug.clone()).or_default();
		if *count > 0 {
			cached
				.note
				.tags
				.push(format!("slug:{}-{}", slug, short_guid(&cached.note.guid)));
		}
		*count += 1;
	}
}

fn public_base_url(subdomain: &str, base_domain: &str, cloudfront_url: Option<&str>) -> String {
	if let Some(cloudfront_url) = cloudfront_url.filter(|url| !url.trim().is_empty()) {
		return format!("{}/{}/", cloudfront_url.trim_end_matches('/'), subdomain);
	}
	format!(
		"https://{}.{}/",
		subdomain,
		base_domain.trim_end_matches('.')
	)
}

fn evernote_millis_to_utc(milliseconds: i64) -> Result<DateTime<Utc>> {
	let seconds = milliseconds.div_euclid(1000);
	let millis = milliseconds.rem_euclid(1000);
	Utc.timestamp_opt(seconds, (millis as u32) * 1_000_000)
		.single()
		.ok_or_else(|| anyhow!("invalid Evernote timestamp {milliseconds}"))
}

fn clean_title(title: &str) -> String {
	let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
	if title.is_empty() {
		"Untitled".into()
	} else {
		title
	}
}

fn safe_file_name(filename: &str, fallback_hash: &str) -> String {
	let name = Path::new(filename)
		.file_name()
		.and_then(|name| name.to_str())
		.map(str::trim)
		.filter(|name| !name.is_empty())
		.unwrap_or(fallback_hash);
	name.chars()
		.map(|character| match character {
			'/' | '\\' | ':' | '\0' => '-',
			_ => character,
		})
		.collect()
}

fn account_label_from_db_path(graph_db: &Path) -> String {
	graph_db
		.file_name()
		.and_then(|name| name.to_str())
		.and_then(|name| name.strip_prefix("UDB-"))
		.and_then(|name| name.strip_suffix("+RemoteGraph.sql"))
		.unwrap_or("evernote-account")
		.to_string()
}

fn short_guid(guid: &str) -> String {
	guid.chars()
		.filter(|character| character.is_ascii_hexdigit())
		.take(8)
		.collect::<String>()
}

fn bool_to_int(value: bool) -> i64 {
	i64::from(value)
}

fn sqlite_now() -> String {
	Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
	use super::*;
	use pretty_assertions::assert_eq;

	#[test]
	fn reads_cache_notebook_as_site() {
		let fixture = CacheFixture::new();
		let app_db = fixture.app_db();
		let options = fixture.options();

		let sites = read_cache_sites(&options, &app_db, &fixture.graph_db).unwrap();

		assert_eq!(sites.len(), 1);
		let site = &sites[0];
		assert_eq!(site.user.settings.title, "Public Notebook");
		assert_eq!(
			site.user.settings.base_url,
			"https://d111111abcdef8.cloudfront.net/public-notebook/"
		);
		assert_eq!(site.notes.len(), 1);
		assert_eq!(site.notes[0].note.title, "Hello from cache");
		assert_eq!(site.notes[0].note.tags, vec!["intro"]);
		assert!(site.notes[0].note.enml.contains("Hello cached world"));
		assert_eq!(site.notes[0].files.len(), 1);
	}

	#[test]
	fn rebuild_all_generates_public_html_from_cache() {
		let fixture = CacheFixture::new();
		let options = fixture.options();

		let summary = rebuild_all(&options).unwrap();

		assert_eq!(
			summary,
			RebuildSummary {
				notebooks_seen: 1,
				notes_seen: 1,
				sites_built: 1,
				sites_failed: 0,
			}
		);
		let html = fs::read_to_string(
			fixture
				.sites_dir
				.join("public-notebook/public/posts/hello-from-cache/index.html"),
		)
		.unwrap();
		assert!(html.contains("Hello cached world"));
		assert!(html.contains("<audio controls"));
		assert!(
			fixture
				.sites_dir
				.join("public-notebook/public/posts/hello-from-cache/episode.mp3")
				.exists()
		);
	}

	#[test]
	fn extracts_body_text_from_rte_cache_fallback() {
		let bytes = b"fontWeight\0inherit\0y Body text from rte\0content\0en-note";

		assert_eq!(
			extract_rte_doc_text(bytes, "Hello from cache").as_deref(),
			Some("Body text from rte")
		);
	}

	#[test]
	fn normalizes_indexed_slash_n_line_breaks() {
		let enml = plain_text_to_enml(
			"Chapter 8. Data Types/nTable of Contents/n8.1. /nNumeric Types",
			"",
			None,
			&[],
		);

		assert_eq!(
			enml,
			"<en-note><p>Chapter 8. Data Types<br>Table of Contents<br>8.1. <br>Numeric Types</p></en-note>"
		);
	}

	struct CacheFixture {
		_temp: tempfile::TempDir,
		config_dir: PathBuf,
		graph_db: PathBuf,
		app_db_path: PathBuf,
		sites_dir: PathBuf,
	}

	impl CacheFixture {
		fn new() -> Self {
			let temp = tempfile::tempdir().unwrap();
			let config_dir = temp.path().join("Evernote");
			let service_dir = config_dir
				.join("conduit-storage")
				.join("https%3A%2F%2Fwww.evernote.com");
			fs::create_dir_all(&service_dir).unwrap();
			let graph_db = service_dir.join("UDB-User42+RemoteGraph.sql");
			create_cache_db(&graph_db);
			let resource_dir = config_dir
				.join("conduit-fs")
				.join("https%3A%2F%2Fwww.evernote.com")
				.join("User42")
				.join("resources");
			fs::create_dir_all(&resource_dir).unwrap();
			fs::write(resource_dir.join("abc123.dat"), "audio").unwrap();
			let rte_dir = config_dir
				.join("conduit-fs")
				.join("https%3A%2F%2Fwww.evernote.com")
				.join("User42")
				.join("rte")
				.join("Note")
				.join("internal_rteDoc")
				.join("not")
				.join("e-1");
			fs::create_dir_all(&rte_dir).unwrap();
			fs::write(
				rte_dir.join("note-1.dat"),
				b"fontWeight\0inherit\0y Hello cached world\0content\0en-note",
			)
			.unwrap();

			let app_db_path = temp.path().join("app.sqlite");
			let app_db = Connection::open(&app_db_path).unwrap();
			app_db
				.execute_batch(include_str!("../infra/sqlite-schema.sql"))
				.unwrap();
			drop(app_db);

			let sites_dir = temp.path().join("sites");
			Self {
				_temp: temp,
				config_dir,
				graph_db,
				app_db_path,
				sites_dir,
			}
		}

		fn app_db(&self) -> Connection {
			Connection::open(&self.app_db_path).unwrap()
		}

		fn options(&self) -> RebuildOptions {
			RebuildOptions {
				database: self.app_db_path.clone(),
				evernote_config_dir: self.config_dir.clone(),
				sites_dir: self.sites_dir.clone(),
				base_domain: "everpublich.xyz".into(),
				cloudfront_url: Some("https://d111111abcdef8.cloudfront.net/".into()),
			}
		}
	}

	fn create_cache_db(path: &Path) {
		let db = Connection::open(path).unwrap();
		db.execute_batch(
			"
				create table Nodes_Notebook(
					id text primary key,
					label text not null,
					isShared integer not null,
					isExternal integer not null,
					owner real
				);
				create table Nodes_User(
					internal_userID real,
					profile_Profile_id text
				);
			create table Nodes_Note(
				id text primary key,
				label text not null,
				created integer not null,
				updated integer not null,
				deleted integer,
				parent_Notebook_id text,
				snippet text
			);
			create table Offline_Search_Note_Content(id text primary key, content text not null);
			create table Nodes_Tag(id text primary key, label text not null);
			create table NoteTag(id text primary key, Note_id text not null, Tag_id text not null);
				create table Nodes_Invitation(
					id text primary key,
					invitationType text not null,
					parent_Notebook_id text,
					recipient_Profile_id text
				);
				create table Nodes_Membership(
					id text primary key,
					parent_Notebook_id text,
					recipientIsMe integer not null
				);
			create table Attachment(
				id text primary key,
				filename text not null,
				mime text not null,
				isActive integer not null,
				dataHash text not null,
				parent_Note_id text not null
			);
				insert into Nodes_User(internal_userID, profile_Profile_id)
				values (42, 'Profile:USR:42');
				insert into Nodes_Notebook(id, label, isShared, isExternal, owner)
				values ('notebook-1', 'Public Notebook', 0, 1, 99);
				insert into Nodes_Notebook(id, label, isShared, isExternal, owner)
				values ('notebook-private', 'Private Notebook', 0, 0, 42);
				insert into Nodes_Notebook(id, label, isShared, isExternal, owner)
				values ('notebook-outgoing', 'Outgoing Notebook', 0, 0, 42);
				insert into Nodes_Invitation(
					id, invitationType, parent_Notebook_id, recipient_Profile_id
				)
				values ('invitation-1', 'NOTEBOOK', 'notebook-1', 'Profile:USR:42');
				insert into Nodes_Invitation(
					id, invitationType, parent_Notebook_id, recipient_Profile_id
				)
				values ('invitation-outgoing', 'NOTEBOOK', 'notebook-outgoing', 'Profile:USR:99');
				insert into Nodes_Membership(id, parent_Notebook_id, recipientIsMe)
				values ('membership-1', 'notebook-1', 1);
				insert into Nodes_Membership(id, parent_Notebook_id, recipientIsMe)
				values ('membership-outgoing', 'notebook-outgoing', 0);
			insert into Nodes_Note(
				id, label, created, updated, deleted, parent_Notebook_id, snippet
			)
			values (
				'note-1', 'Hello from cache', 1700000000000, 1700000001000, null,
				'notebook-1', ''
			);
				insert into Offline_Search_Note_Content(id, content)
				values ('note-1', '');
			insert into Nodes_Tag(id, label) values ('tag-1', 'intro');
			insert into NoteTag(id, Note_id, Tag_id) values ('note-tag-1', 'note-1', 'tag-1');
			insert into Attachment(id, filename, mime, isActive, dataHash, parent_Note_id)
			values ('attachment-1', 'episode.mp3', 'audio/mpeg', 1, 'abc123', 'note-1');
			",
		)
		.unwrap();
	}
}
