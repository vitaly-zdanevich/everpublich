//! Reader and builder for Evernote-backed generated sites.
//!
//! The preferred MVP path reads notebooks shared to the Everpublich service
//! account through the Evernote API. The old official-client cache reader stays
//! as a fallback/debug source. This module keeps those sync details isolated
//! from the renderer and from the admin database.

use crate::evernote::notes_to_posts;
use crate::evernote_api::{
	DownloadedLinkedNotebook, DownloadedNote, EvernoteApiClient, NoteDownloadCache, NoteSummary,
};
use crate::models::{
	BuildState, EvernoteAccessMode, IndexMode, Note, Post, PostKind, Resource, SearchMode,
	SiteSettings, UserItem,
};
use crate::site_output::{BuiltSiteAnnotation, annotate_built_site, duration_milliseconds};
use crate::slug::slugify;
use crate::store::SqliteUserRow;
use crate::zola::{GeneratedSite, write_zola_site};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, TimeZone, Utc};
use html_escape::{decode_html_entities, encode_double_quoted_attribute, encode_text};
use regex::Regex;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;
use yrs::types::text::YChange;
use yrs::updates::decoder::Decode;
use yrs::{
	Any, Doc, Out, ReadTxn, Text, Transact, Update, Xml, XmlElementRef, XmlFragment,
	XmlFragmentRef, XmlOut, XmlTextRef,
};

/// Runtime inputs for rebuilding every generated website.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildOptions {
	/// Everpublich SQLite database path.
	pub database: PathBuf,
	/// Evernote desktop config/cache directory used by the fallback reader.
	pub evernote_config_dir: PathBuf,
	/// Root directory for generated per-site Zola source trees.
	pub sites_dir: PathBuf,
	/// Future wildcard domain for user websites.
	pub base_domain: String,
	/// CloudFront URL used while wildcard DNS is not configured.
	pub cloudfront_url: Option<String>,
	/// OAuth token for the Evernote service account that receives shared notebooks.
	pub evernote_service_token: Option<String>,
	/// Evernote UserStore endpoint for the service-account token.
	pub evernote_user_store_url: String,
	/// Optional Evernote NoteStore endpoint when the shard URL is already known.
	pub evernote_note_store_url: Option<String>,
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
	original_file_name: Option<String>,
	source_path: PathBuf,
	transform: ResourceTransform,
}

#[derive(Debug, Clone)]
struct CachedAttachment {
	resource: Resource,
	source_path: Option<PathBuf>,
	transform: ResourceTransform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceTransform {
	Copy,
	ImageToAvif,
	VectorToAvif,
	DngEmbeddedJpeg,
	DngToAvif,
	TrackerToOpus,
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
	source_url: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RteNoteContent {
	enml: Option<String>,
	text: Option<String>,
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

/// Rebuild all Zola websites from Evernote and persist build status in SQLite.
pub fn rebuild_all(options: &RebuildOptions) -> Result<RebuildSummary> {
	fs::create_dir_all(&options.sites_dir)
		.with_context(|| format!("failed to create {}", options.sites_dir.display()))?;

	let app_db = Connection::open(&options.database)
		.with_context(|| format!("failed to open {}", options.database.display()))?;

	let mut sites = Vec::new();
	if let Some(token) = options
		.evernote_service_token
		.as_deref()
		.map(str::trim)
		.filter(|token| !token.is_empty())
	{
		sites.extend(read_api_sites(options, &app_db, token)?);
	} else {
		let graph_dbs = discover_remote_graphs(&options.evernote_config_dir)?;
		if graph_dbs.is_empty() {
			bail!(
				"no Evernote RemoteGraph SQLite cache found under {}",
				options.evernote_config_dir.display()
			);
		}
		for graph_db in graph_dbs {
			sites.extend(read_cache_sites(options, &app_db, &graph_db)?);
		}
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
		let generation_started = Instant::now();
		match build_site(options, site, generation_started) {
			Ok((generated, posts, annotation)) => {
				replace_note_index(&app_db, &site.user.user_id, &posts)?;
				finish_build(&app_db, build_id, "succeeded", None)?;
				summary.sites_built += 1;
				print_site_generation_metric(
					&site.user.settings.subdomain,
					annotation.generation_duration_milliseconds,
				);
				println!(
					"Built {} at {} ({} posts, {} pages, {} podcast items, {} bytes raw, {} bytes Brotli, {:.2}% savings)",
					site.user.settings.title,
					site.user.settings.base_url,
					generated.posts,
					generated.pages,
					generated.podcast_items,
					annotation.total_size_bytes,
					annotation.brotli_size_bytes,
					annotation.brotli_savings_percent
				);
			}
			Err(error) => {
				print_site_generation_metric(
					&site.user.settings.subdomain,
					duration_milliseconds(generation_started.elapsed()),
				);
				finish_build(&app_db, build_id, "failed", Some(&error.to_string()))?;
				summary.sites_failed += 1;
				eprintln!("Failed to build {}: {error:#}", site.user.settings.title);
			}
		}
	}

	Ok(summary)
}

fn print_site_generation_metric(site_slug: &str, duration_milliseconds: u64) {
	println!(
		"SiteGenerationSeconds\t{}\t{:.3}",
		site_slug,
		duration_milliseconds as f64 / 1000.0
	);
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
		if slug.starts_with('.') {
			continue;
		}
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

		let notes = read_notes_for_notebook(
			&cache_db,
			&options.evernote_config_dir,
			&notebook.id,
			&account_label,
		)?;
		if !notes.is_empty() {
			sites.push(CacheSite { user, notes });
		}
	}
	Ok(sites)
}

fn read_api_sites(
	options: &RebuildOptions,
	app_db: &Connection,
	token: &str,
) -> Result<Vec<CacheSite>> {
	let download_root = api_download_root(&options.sites_dir);
	fs::create_dir_all(&download_root)
		.with_context(|| format!("failed to create {}", download_root.display()))?;
	let mut cache = ApiNoteDownloadCache::new(&download_root);

	let client = EvernoteApiClient::new(
		token.to_string(),
		Some(options.evernote_user_store_url.clone()),
		options.evernote_note_store_url.clone(),
	)?;
	let notebooks = client.download_linked_notebooks_with_cache(None, &mut cache)?;
	prune_api_download_cache(&download_root, &notebooks)?;
	notebooks
		.into_iter()
		.filter_map(|notebook| {
			match api_notebook_to_site(options, app_db, &download_root, notebook) {
				Ok(Some(site)) => Some(Ok(site)),
				Ok(None) => None,
				Err(error) => Some(Err(error)),
			}
		})
		.collect()
}

fn api_download_root(sites_dir: &Path) -> PathBuf {
	sites_dir.join(".evernote-api-resources")
}

#[derive(Debug, Clone)]
struct ApiNoteDownloadCache {
	root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiCachedNote {
	guid: String,
	title: Option<String>,
	content: Option<String>,
	created: Option<i64>,
	updated: Option<i64>,
	source_url: Option<String>,
	tag_names: Vec<String>,
	resources: Vec<ApiCachedResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiCachedResource {
	cache_hash: String,
	body_hash_hex: Option<String>,
	guid: Option<String>,
	mime: String,
	size: Option<i32>,
	file_name: Option<String>,
	body_file: String,
}

impl ApiNoteDownloadCache {
	fn new(root: &Path) -> Self {
		Self {
			root: root.to_path_buf(),
		}
	}

	fn note_dir(&self, notebook_guid: &str, note_guid: &str) -> PathBuf {
		self.root
			.join(safe_file_name(notebook_guid, "notebook"))
			.join(safe_file_name(note_guid, "note"))
	}

	fn note_json_path(&self, notebook_guid: &str, note_guid: &str) -> PathBuf {
		self.note_dir(notebook_guid, note_guid).join("note.json")
	}
}

impl NoteDownloadCache for ApiNoteDownloadCache {
	fn get_cached_note(
		&mut self,
		notebook_guid: &str,
		metadata: &NoteSummary,
	) -> Result<Option<DownloadedNote>> {
		let Some(updated) = metadata.updated else {
			return Ok(None);
		};
		let note_json = self.note_json_path(notebook_guid, &metadata.guid);
		if !note_json.exists() {
			return Ok(None);
		}
		let bytes = fs::read(&note_json)
			.with_context(|| format!("failed to read {}", note_json.display()))?;
		let cached: ApiCachedNote = serde_json::from_slice(&bytes)
			.with_context(|| format!("failed to parse {}", note_json.display()))?;
		if cached.guid != metadata.guid || cached.updated != Some(updated) {
			return Ok(None);
		}
		let Some(content) = cached.content.clone().filter(|content| !content.is_empty()) else {
			return Ok(None);
		};
		let resource_dir = self
			.note_dir(notebook_guid, &metadata.guid)
			.join("resources");
		let mut resources = Vec::new();
		for resource in &cached.resources {
			let body_path = resource_dir.join(&resource.body_file);
			let body = match fs::read(&body_path) {
				Ok(body) if !body.is_empty() => body,
				Ok(_) => return Ok(None),
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
				Err(error) => {
					return Err(error)
						.with_context(|| format!("failed to read {}", body_path.display()));
				}
			};
			resources.push(cached_resource_to_edam(resource, body));
		}
		let tag_names = cached.tag_names.clone();
		Ok(Some(DownloadedNote {
			note: cached_note_to_edam(cached, notebook_guid, content, resources),
			tag_names,
		}))
	}

	fn put_cached_note(&mut self, notebook_guid: &str, note: &DownloadedNote) -> Result<()> {
		let Some(guid) = note
			.note
			.guid
			.as_deref()
			.filter(|guid| !guid.trim().is_empty())
		else {
			return Ok(());
		};
		let note_dir = self.note_dir(notebook_guid, guid);
		let resource_dir = note_dir.join("resources");
		fs::create_dir_all(&resource_dir)
			.with_context(|| format!("failed to create {}", resource_dir.display()))?;

		let mut resources = Vec::new();
		for resource in note.note.resources.as_deref().unwrap_or_default() {
			let Some(data) = &resource.data else {
				continue;
			};
			let Some(body) = data.body.as_deref().filter(|body| !body.is_empty()) else {
				continue;
			};
			let cache_hash = api_resource_cache_hash(resource);
			let body_file = safe_file_name(&format!("{cache_hash}.body"), "resource.body");
			fs::write(resource_dir.join(&body_file), body).with_context(|| {
				format!(
					"failed to write cached Evernote resource {}",
					resource_dir.join(&body_file).display()
				)
			})?;
			resources.push(ApiCachedResource {
				cache_hash,
				body_hash_hex: data.body_hash.as_deref().map(hex_lower),
				guid: resource.guid.clone(),
				mime: resource
					.mime
					.as_deref()
					.map(str::trim)
					.filter(|mime| !mime.is_empty())
					.unwrap_or("application/octet-stream")
					.to_string(),
				size: data.size.or_else(|| i32::try_from(body.len()).ok()),
				file_name: resource
					.attributes
					.as_ref()
					.and_then(|attributes| attributes.file_name.clone()),
				body_file,
			});
		}

		let cached = ApiCachedNote {
			guid: guid.to_string(),
			title: note.note.title.clone(),
			content: note.note.content.clone(),
			created: note.note.created,
			updated: note.note.updated,
			source_url: note
				.note
				.attributes
				.as_ref()
				.and_then(|attributes| attributes.source_u_r_l.clone()),
			tag_names: note.tag_names.clone(),
			resources,
		};
		fs::write(
			note_dir.join("note.json"),
			serde_json::to_vec_pretty(&cached)?,
		)
		.with_context(|| format!("failed to write {}", note_dir.join("note.json").display()))?;
		Ok(())
	}
}

fn cached_note_to_edam(
	cached: ApiCachedNote,
	notebook_guid: &str,
	content: String,
	resources: Vec<evernote_edam::types::Resource>,
) -> evernote_edam::types::Note {
	evernote_edam::types::Note {
		guid: Some(cached.guid),
		title: cached.title,
		content: Some(content),
		content_hash: None,
		content_length: None,
		created: cached.created,
		updated: cached.updated,
		deleted: None,
		active: Some(true),
		update_sequence_num: None,
		notebook_guid: Some(notebook_guid.to_string()),
		tag_guids: None,
		resources: Some(resources),
		attributes: Some(note_attributes_with_source_url(cached.source_url)),
		tag_names: Some(cached.tag_names),
		shared_notes: None,
		restrictions: None,
		limits: None,
	}
}

fn cached_resource_to_edam(
	resource: &ApiCachedResource,
	body: Vec<u8>,
) -> evernote_edam::types::Resource {
	evernote_edam::types::Resource {
		guid: resource
			.guid
			.clone()
			.or_else(|| Some(resource.cache_hash.clone())),
		note_guid: None,
		data: Some(evernote_edam::types::Data {
			body_hash: resource.body_hash_hex.as_deref().and_then(hex_to_bytes),
			size: resource
				.size
				.or_else(|| i32::try_from(body.len()).ok())
				.map(|size| size.max(0)),
			body: Some(body),
		}),
		mime: Some(resource.mime.clone()),
		width: None,
		height: None,
		duration: None,
		active: Some(true),
		recognition: None,
		attributes: Some(resource_attributes_with_file_name(
			resource.file_name.clone(),
		)),
		update_sequence_num: None,
		alternate_data: None,
	}
}

fn note_attributes_with_source_url(
	source_url: Option<String>,
) -> evernote_edam::types::NoteAttributes {
	evernote_edam::types::NoteAttributes {
		subject_date: None,
		latitude: None,
		longitude: None,
		altitude: None,
		author: None,
		source: None,
		source_u_r_l: source_url,
		source_application: None,
		share_date: None,
		reminder_order: None,
		reminder_done_time: None,
		reminder_time: None,
		place_name: None,
		content_class: None,
		application_data: None,
		last_edited_by: None,
		classifications: None,
		creator_id: None,
		last_editor_id: None,
		shared_with_business: None,
		conflict_source_note_guid: None,
		note_title_quality: None,
	}
}

fn resource_attributes_with_file_name(
	file_name: Option<String>,
) -> evernote_edam::types::ResourceAttributes {
	evernote_edam::types::ResourceAttributes {
		source_u_r_l: None,
		timestamp: None,
		latitude: None,
		longitude: None,
		altitude: None,
		camera_make: None,
		camera_model: None,
		client_will_index: None,
		reco_type: None,
		file_name,
		attachment: Some(true),
		application_data: None,
	}
}

fn api_resource_cache_hash(resource: &evernote_edam::types::Resource) -> String {
	resource
		.data
		.as_ref()
		.and_then(|data| data.body_hash.as_deref())
		.map(hex_lower)
		.filter(|hash| !hash.is_empty())
		.or_else(|| resource.guid.as_deref().map(slugify))
		.unwrap_or_else(|| "resource".to_string())
}

fn hex_to_bytes(value: &str) -> Option<Vec<u8>> {
	if !value.len().is_multiple_of(2) {
		return None;
	}
	let mut bytes = Vec::with_capacity(value.len() / 2);
	for index in (0..value.len()).step_by(2) {
		let byte = u8::from_str_radix(&value[index..index + 2], 16).ok()?;
		bytes.push(byte);
	}
	Some(bytes)
}

fn prune_api_download_cache(root: &Path, notebooks: &[DownloadedLinkedNotebook]) -> Result<()> {
	let allowed_notebooks = notebooks
		.iter()
		.map(|notebook| safe_file_name(&notebook.notebook_guid, "notebook"))
		.collect::<BTreeSet<_>>();
	if root.exists() {
		for entry in
			fs::read_dir(root).with_context(|| format!("failed to read {}", root.display()))?
		{
			let entry = entry?;
			if !entry.file_type()?.is_dir() {
				continue;
			}
			let name = entry.file_name().to_string_lossy().to_string();
			if !allowed_notebooks.contains(&name) {
				fs::remove_dir_all(entry.path()).with_context(|| {
					format!(
						"failed to remove stale API cache {}",
						entry.path().display()
					)
				})?;
			}
		}
	}
	for notebook in notebooks {
		prune_api_notebook_cache(root, notebook)?;
	}
	Ok(())
}

fn prune_api_notebook_cache(root: &Path, notebook: &DownloadedLinkedNotebook) -> Result<()> {
	let notebook_dir = root.join(safe_file_name(&notebook.notebook_guid, "notebook"));
	if !notebook_dir.exists() {
		return Ok(());
	}
	let allowed_notes = notebook
		.notes
		.iter()
		.filter_map(|note| note.note.guid.as_deref())
		.map(|guid| safe_file_name(guid, "note"))
		.collect::<BTreeSet<_>>();
	for entry in fs::read_dir(&notebook_dir)
		.with_context(|| format!("failed to read {}", notebook_dir.display()))?
	{
		let entry = entry?;
		if !entry.file_type()?.is_dir() {
			continue;
		}
		let name = entry.file_name().to_string_lossy().to_string();
		if !allowed_notes.contains(&name) {
			fs::remove_dir_all(entry.path()).with_context(|| {
				format!(
					"failed to remove stale API note cache {}",
					entry.path().display()
				)
			})?;
		}
	}
	Ok(())
}

fn existing_user_id_for_api_notebook(
	app_db: &Connection,
	notebook_guid: &str,
	site_slug: &str,
) -> Result<Option<String>> {
	app_db
		.query_row(
			"select user_id from users
			 where shared_notebook_guid = ?1 or site_slug = ?2
			 order by case when shared_notebook_guid = ?1 then 0 else 1 end
			 limit 1",
			params![notebook_guid, site_slug],
			|row| row.get(0),
		)
		.optional()
		.context("failed to find existing user row for API notebook")
}

fn api_notebook_to_site(
	options: &RebuildOptions,
	app_db: &Connection,
	download_root: &Path,
	notebook: DownloadedLinkedNotebook,
) -> Result<Option<CacheSite>> {
	let label = notebook
		.share_name
		.as_deref()
		.map(str::trim)
		.filter(|name| !name.is_empty())
		.unwrap_or(&notebook.notebook_guid);
	let mut settings = SiteSettings::new(label, &options.base_domain);
	settings.notebook_guid = Some(notebook.notebook_guid.clone());
	settings.notebook_name = Some(label.to_string());
	settings.base_url = public_base_url(
		&settings.subdomain,
		&options.base_domain,
		options.cloudfront_url.as_deref(),
	);
	let user_id =
		existing_user_id_for_api_notebook(app_db, &notebook.notebook_guid, &settings.subdomain)?
			.unwrap_or_else(|| format!("evernote-api-{}", slugify(&notebook.notebook_guid)));

	let mut user = UserItem {
		user_id,
		registration_date: Utc::now(),
		evernote_user_id: notebook.owner_username,
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

	let notes = notebook
		.notes
		.into_iter()
		.map(|note| api_note_to_cached_note(download_root, &notebook.notebook_guid, note))
		.collect::<Result<Vec<_>>>()?;
	Ok((!notes.is_empty()).then_some(CacheSite { user, notes }))
}

fn api_note_to_cached_note(
	download_root: &Path,
	notebook_guid: &str,
	downloaded: DownloadedNote,
) -> Result<CachedNote> {
	let note = downloaded.note;
	let guid = note
		.guid
		.clone()
		.filter(|guid| !guid.trim().is_empty())
		.ok_or_else(|| anyhow!("Evernote API note is missing a GUID"))?;
	let title = clean_title(note.title.as_deref().unwrap_or("Untitled"));
	let note_download_dir = download_root
		.join(safe_file_name(notebook_guid, "notebook"))
		.join(safe_file_name(&guid, "note"));
	fs::create_dir_all(&note_download_dir)
		.with_context(|| format!("failed to create {}", note_download_dir.display()))?;
	let attachments = note
		.resources
		.unwrap_or_default()
		.into_iter()
		.map(|resource| api_resource_to_attachment(&note_download_dir, resource))
		.collect::<Result<Vec<_>>>()?
		.into_iter()
		.flatten()
		.collect::<Vec<_>>();
	let resources = attachments
		.iter()
		.map(|attachment| attachment.resource.clone())
		.collect::<Vec<_>>();

	let content = note
		.content
		.as_deref()
		.filter(|content| !content.trim().is_empty())
		.unwrap_or("<en-note></en-note>");
	let plain_text = enml_plain_text(content);
	let (_, content_metadata) = extract_trailing_content_metadata(&plain_text);
	let mut tags = Vec::new();
	extend_unique_tags(&mut tags, downloaded.tag_names);
	extend_unique_tags(&mut tags, content_metadata.tags);
	let source_url = note
		.attributes
		.as_ref()
		.and_then(|attributes| attributes.source_u_r_l.as_deref())
		.filter(|url| !url.trim().is_empty())
		.or(content_metadata.source_url.as_deref());
	let enml = rich_text_to_enml(
		content,
		content_metadata.raw_line.as_deref(),
		source_url,
		&resources,
	);
	let created = note.created.or(note.updated).unwrap_or_default();
	let updated = note.updated.or(note.created).unwrap_or(created);
	let files = attachments
		.into_iter()
		.filter_map(|attachment| {
			attachment
				.source_path
				.map(|source_path| CachedResourceFile {
					file_name: attachment.resource.file_name.clone(),
					original_file_name: attachment.resource.original_file_name.clone(),
					source_path,
					transform: attachment.transform,
				})
		})
		.collect();

	Ok(CachedNote {
		note: Note {
			guid,
			title,
			created: evernote_millis_to_utc(created)?,
			updated: evernote_millis_to_utc(updated)?,
			enml,
			resources,
			tags,
		},
		files,
	})
}

fn api_resource_to_attachment(
	note_download_dir: &Path,
	resource: evernote_edam::types::Resource,
) -> Result<Option<CachedAttachment>> {
	let mime = resource
		.mime
		.as_deref()
		.map(str::trim)
		.filter(|mime| !mime.is_empty())
		.unwrap_or("application/octet-stream")
		.to_string();
	let data = resource.data.unwrap_or_default();
	let body = data.body.unwrap_or_default();
	if body.is_empty() {
		return Ok(None);
	}
	let hash = data
		.body_hash
		.as_deref()
		.map(hex_lower)
		.filter(|hash| !hash.is_empty())
		.or_else(|| resource.guid.as_deref().map(slugify))
		.unwrap_or_else(|| "resource".to_string());
	let original_file_name = resource
		.attributes
		.as_ref()
		.and_then(|attributes| attributes.file_name.as_deref())
		.map(|file_name| safe_file_name(file_name, &hash))
		.unwrap_or_else(|| resource_file_name_from_mime(&hash, &mime));
	let source_path = note_download_dir.join(format!("{hash}-{original_file_name}"));
	fs::write(&source_path, &body)
		.with_context(|| format!("failed to write {}", source_path.display()))?;
	let size_bytes = Some(data.size.unwrap_or(body.len() as i32).max(0) as u64);
	let transform = resource_transform(&original_file_name, &mime, Some(&source_path));
	let file_name = transformed_file_name(&original_file_name, &hash, transform);
	let resource_mime = transformed_mime(&mime, transform);
	let original_resource_file_name = preview_original_file_name(&original_file_name, transform);
	let text_preview = Some(source_path.as_path())
		.filter(|_| is_text_like_attachment(&file_name, &mime))
		.and_then(read_text_preview);
	let archive_tree = Some(source_path.as_path())
		.filter(|_| is_archive_attachment(&file_name, &mime))
		.and_then(read_archive_tree);

	Ok(Some(CachedAttachment {
		resource: Resource {
			hash,
			file_name,
			original_file_name: original_resource_file_name,
			mime: resource_mime,
			s3_key: None,
			text_preview,
			archive_tree,
			size_bytes,
		},
		source_path: Some(source_path),
		transform,
	}))
}

fn resource_file_name_from_mime(hash: &str, mime: &str) -> String {
	let extension = match mime {
		"application/pdf" => "pdf",
		"audio/mpeg" => "mp3",
		"image/jpeg" => "jpg",
		"image/svg+xml" => "svg",
		"text/plain" => "txt",
		value => value
			.rsplit_once('/')
			.map(|(_, extension)| extension)
			.unwrap_or("bin"),
	};
	safe_file_name(&format!("{hash}.{extension}"), hash)
}

fn hex_lower(bytes: &[u8]) -> String {
	let mut output = String::with_capacity(bytes.len() * 2);
	for byte in bytes {
		output.push_str(&format!("{byte:02x}"));
	}
	output
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
		 coalesce(c.content, ''), coalesce(n.snippet, ''), n.source_URL \
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
				source_url: row.get(6)?,
			})
		})?
		.collect::<rusqlite::Result<Vec<_>>>()?;

	rows.into_iter()
		.map(|row| {
			let mut tags = read_tags(cache_db, &row.id)?;
			let attachments = read_attachments(cache_db, config_dir, &row.id)?;
			let resources = attachments
				.iter()
				.map(|attachment| attachment.resource.clone())
				.collect::<Vec<_>>();
			let rte_content = read_note_rte_content(config_dir, &row.id, &row.title)?;
			let plain_text =
				note_plain_text(&row.content, &row.snippet, rte_content.text.as_deref());
			let (body_text, content_metadata) = extract_trailing_content_metadata(&plain_text);
			extend_unique_tags(&mut tags, content_metadata.tags);
			let source_url = row
				.source_url
				.as_deref()
				.filter(|url| !url.trim().is_empty())
				.or(content_metadata.source_url.as_deref());
			let enml = if let Some(rte_enml) = rte_content.enml {
				rich_text_to_enml(
					&rte_enml,
					content_metadata.raw_line.as_deref(),
					source_url,
					&resources,
				)
			} else {
				plain_text_to_enml(&body_text, source_url, &resources)
			};
			let files = attachments
				.into_iter()
				.filter_map(|attachment| {
					attachment
						.source_path
						.map(|source_path| CachedResourceFile {
							file_name: attachment.resource.file_name.clone(),
							original_file_name: attachment.resource.original_file_name.clone(),
							source_path,
							transform: attachment.transform,
						})
				})
				.collect();
			Ok(CachedNote {
				note: Note {
					guid: row.id,
					title: clean_title(&row.title),
					created: evernote_millis_to_utc(row.created_millis)?,
					updated: evernote_millis_to_utc(row.updated_millis)?,
					enml,
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

/// Merge tags parsed from note text with real Evernote tags without adding
/// case-insensitive duplicates.
fn extend_unique_tags(tags: &mut Vec<String>, content_tags: Vec<String>) {
	for tag in content_tags {
		if !tags
			.iter()
			.any(|existing| existing.eq_ignore_ascii_case(&tag))
		{
			tags.push(tag);
		}
	}
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

	let attachments = attachments
		.into_iter()
		.map(|(hash, filename, mime)| cached_attachment(config_dir, &hash, &filename, &mime))
		.collect::<Result<Vec<_>>>()?
		.into_iter()
		.flatten()
		.collect();
	Ok(attachments)
}

fn cached_attachment(
	config_dir: &Path,
	hash: &str,
	filename: &str,
	mime: &str,
) -> Result<Option<CachedAttachment>> {
	let original_file_name = safe_file_name(filename, hash);
	let Some(source_path) = find_cached_resource_file(config_dir, hash)? else {
		return Ok(None);
	};
	let size_bytes = Some(
		fs::metadata(&source_path)
			.with_context(|| format!("failed to stat attachment {}", source_path.display()))?
			.len(),
	);
	let transform = resource_transform(&original_file_name, mime, Some(&source_path));
	let file_name = transformed_file_name(&original_file_name, hash, transform);
	let resource_mime = transformed_mime(mime, transform);
	let original_resource_file_name = preview_original_file_name(&original_file_name, transform);
	let text_preview = Some(source_path.as_path())
		.filter(|_| is_text_like_attachment(&file_name, mime))
		.and_then(read_text_preview);
	let archive_tree = Some(source_path.as_path())
		.filter(|_| is_archive_attachment(&file_name, mime))
		.and_then(read_archive_tree);

	Ok(Some(CachedAttachment {
		resource: Resource {
			hash: hash.to_ascii_lowercase(),
			file_name,
			original_file_name: original_resource_file_name,
			mime: resource_mime,
			s3_key: None,
			text_preview,
			archive_tree,
			size_bytes,
		},
		source_path: Some(source_path),
		transform,
	}))
}

fn transformed_file_name(
	original_file_name: &str,
	hash: &str,
	transform: ResourceTransform,
) -> String {
	match transform {
		ResourceTransform::Copy => original_file_name.to_string(),
		ResourceTransform::ImageToAvif
		| ResourceTransform::VectorToAvif
		| ResourceTransform::DngToAvif => file_name_with_extension(original_file_name, hash, "avif"),
		ResourceTransform::DngEmbeddedJpeg => {
			file_name_with_extension(original_file_name, hash, "jpg")
		}
		ResourceTransform::TrackerToOpus => {
			file_name_with_extension(original_file_name, hash, "opus")
		}
	}
}

fn transformed_mime(mime: &str, transform: ResourceTransform) -> String {
	match transform {
		ResourceTransform::ImageToAvif
		| ResourceTransform::VectorToAvif
		| ResourceTransform::DngToAvif => "image/avif".to_string(),
		ResourceTransform::DngEmbeddedJpeg => "image/jpeg".to_string(),
		ResourceTransform::TrackerToOpus => "audio/opus".to_string(),
		ResourceTransform::Copy => mime.to_string(),
	}
}

fn preview_original_file_name(
	original_file_name: &str,
	transform: ResourceTransform,
) -> Option<String> {
	matches!(
		transform,
		ResourceTransform::ImageToAvif
			| ResourceTransform::VectorToAvif
			| ResourceTransform::DngEmbeddedJpeg
			| ResourceTransform::DngToAvif
	)
	.then(|| original_file_name.to_string())
}

fn is_text_like_attachment(file_name: &str, mime: &str) -> bool {
	let mime = mime.to_ascii_lowercase();
	mime.starts_with("text/")
		|| matches!(
			mime.as_str(),
			"application/rtf"
				| "application/x-rtf"
				| "application/x-subrip"
				| "application/ttml+xml"
				| "application/json"
				| "application/xml"
		) || matches!(
		file_extension(file_name).as_deref(),
		Some(
			"txt"
				| "text" | "md"
				| "markdown" | "rtf"
				| "log" | "srt"
				| "vtt" | "ass"
				| "ssa" | "sub"
				| "sbv" | "ttml"
				| "dfxp" | "csv"
				| "tsv" | "json"
				| "xml" | "yaml"
				| "yml" | "toml"
		)
	)
}

fn is_archive_attachment(file_name: &str, mime: &str) -> bool {
	let mime = mime.to_ascii_lowercase();
	matches!(
		mime.as_str(),
		"application/zip"
			| "application/x-rar-compressed"
			| "application/vnd.rar"
			| "application/x-tar"
			| "application/gzip"
			| "application/x-gzip"
			| "application/x-bzip2"
			| "application/x-xz"
			| "application/zstd"
	) || matches!(
		file_extension(file_name).as_deref(),
		Some(
			"zip"
				| "rar" | "tar"
				| "tgz" | "tbz"
				| "tbz2" | "txz"
				| "tar.gz" | "tar.bz2"
				| "tar.xz" | "tar.zst"
		)
	)
}

fn read_text_preview(path: &Path) -> Option<String> {
	let bytes = fs::read(path).ok()?;
	let limit = bytes.len().min(128 * 1024);
	let mut text = String::from_utf8_lossy(&bytes[..limit]).to_string();
	if bytes.len() > limit {
		text.push_str("\n... preview truncated ...");
	}
	Some(text)
}

fn read_archive_tree(path: &Path) -> Option<String> {
	let extension = file_extension(path.file_name()?.to_str()?);
	let output = match extension.as_deref() {
		Some("zip") => Command::new("zipinfo").arg("-1").arg(path).output().ok(),
		Some("rar") => Command::new("unrar").arg("lb").arg(path).output().ok(),
		Some(
			"tar" | "tgz" | "tbz" | "tbz2" | "txz" | "tar.gz" | "tar.bz2" | "tar.xz" | "tar.zst",
		) => Command::new("tar").arg("-tf").arg(path).output().ok(),
		_ => None,
	}?;
	if !output.status.success() {
		return None;
	}
	let entries = String::from_utf8_lossy(&output.stdout)
		.lines()
		.take(1000)
		.map(str::trim_end)
		.filter(|line| !line.is_empty())
		.map(str::to_string)
		.collect::<Vec<_>>();
	let tree = archive_entries_to_tree(&entries);
	(!tree.is_empty()).then_some(tree)
}

#[derive(Default)]
struct ArchiveTreeNode {
	children: BTreeMap<String, ArchiveTreeNode>,
}

fn archive_entries_to_tree(entries: &[String]) -> String {
	archive_entries_to_tree_command(entries)
		.unwrap_or_else(|| archive_entries_to_tree_fallback(entries))
}

fn archive_entries_to_tree_command(entries: &[String]) -> Option<String> {
	let entries = normalized_archive_entries(entries);
	if entries.is_empty() {
		return None;
	}
	let mut child = Command::new("tree")
		.arg("--fromfile")
		.arg("--noreport")
		.arg("--charset=ascii")
		.arg(".")
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.ok()?;
	{
		let mut stdin = child.stdin.take()?;
		for entry in entries {
			writeln!(stdin, "{entry}").ok()?;
		}
	}
	let output = child.wait_with_output().ok()?;
	if !output.status.success() {
		return None;
	}
	let tree = String::from_utf8_lossy(&output.stdout)
		.lines()
		.map(str::trim_end)
		.filter(|line| !line.is_empty())
		.collect::<Vec<_>>()
		.join("\n");
	(!tree.is_empty()).then_some(tree)
}

fn archive_entries_to_tree_fallback(entries: &[String]) -> String {
	let mut root = ArchiveTreeNode::default();
	for entry in normalized_archive_entries(entries) {
		let parts = entry
			.split('/')
			.filter(|part| !part.is_empty())
			.collect::<Vec<_>>();
		if parts.is_empty() {
			continue;
		}
		let mut node = &mut root;
		for part in parts {
			node = node.children.entry(part.to_string()).or_default();
		}
	}
	if root.children.is_empty() {
		return String::new();
	}
	let mut tree = ".".to_string();
	push_archive_tree_children(&root, "", &mut tree);
	tree
}

fn normalized_archive_entries(entries: &[String]) -> Vec<String> {
	entries
		.iter()
		.map(|entry| entry.trim().trim_matches('/'))
		.filter(|entry| !entry.is_empty())
		.map(str::to_string)
		.collect()
}

fn push_archive_tree_children(node: &ArchiveTreeNode, prefix: &str, out: &mut String) {
	let count = node.children.len();
	for (index, (name, child)) in node.children.iter().enumerate() {
		let is_last = index + 1 == count;
		out.push('\n');
		out.push_str(prefix);
		out.push_str(if is_last { "`-- " } else { "|-- " });
		out.push_str(name);
		let next_prefix = if is_last {
			format!("{prefix}    ")
		} else {
			format!("{prefix}|   ")
		};
		push_archive_tree_children(child, &next_prefix, out);
	}
}

fn file_extension(file_name: &str) -> Option<String> {
	let lower = file_name.to_ascii_lowercase();
	for extension in ["tar.gz", "tar.bz2", "tar.xz", "tar.zst"] {
		if lower.ends_with(extension) {
			return Some(extension.to_string());
		}
	}
	lower
		.rsplit_once('.')
		.map(|(_, extension)| extension.to_string())
}

fn file_extension_from_path(path: &Path) -> Option<String> {
	path.file_name()
		.and_then(|file_name| file_name.to_str())
		.and_then(file_extension)
}

fn find_cached_resource_file(config_dir: &Path, hash: &str) -> Result<Option<PathBuf>> {
	let wanted = hash.to_ascii_lowercase();
	let roots = [
		config_dir.join("conduit-fs"),
		config_dir.join("resource-cache"),
	];
	let mut stack = roots
		.into_iter()
		.filter(|root| root.exists())
		.collect::<Vec<_>>();
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
			if name.ends_with(".meta") {
				continue;
			}
			if name == wanted || name == format!("{wanted}.dat") {
				return Ok(Some(path));
			}
		}
	}
	Ok(None)
}

fn read_note_rte_content(config_dir: &Path, note_id: &str, title: &str) -> Result<RteNoteContent> {
	let Some(path) = find_note_rte_doc_file(config_dir, note_id)? else {
		return Ok(RteNoteContent::default());
	};
	let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
	let enml = rte_doc_to_enml(&bytes).ok().flatten();
	let text = extract_rte_doc_text(&bytes, title);
	Ok(RteNoteContent { enml, text })
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

/// Decode an Evernote rich-text cache document into ENML.
///
/// The official desktop client stores note bodies as Yjs XML updates. We use
/// `yrs` to read that tree and then serialize the `content` XML fragment into
/// the ENML subset consumed by the existing Zola renderer.
fn rte_doc_to_enml(bytes: &[u8]) -> Result<Option<String>> {
	match rte_doc_to_enml_with_decoder(bytes, Update::decode_v1) {
		Ok(enml) => Ok(enml),
		Err(v1_error) => rte_doc_to_enml_with_decoder(bytes, Update::decode_v2)
			.with_context(|| format!("failed to decode Evernote RTE Yjs update: {v1_error}")),
	}
}

fn rte_doc_to_enml_with_decoder(
	bytes: &[u8],
	decode: fn(&[u8]) -> Result<Update, yrs::encoding::read::Error>,
) -> Result<Option<String>> {
	let doc = Doc::new();
	let content = doc.get_or_insert_xml_fragment("content");
	let update = decode(bytes)?;
	doc.transact_mut().apply_update(update)?;
	let txn = doc.transact();
	let enml = restore_adjacent_inline_spacing(&serialize_xml_fragment(&content, &txn));
	if enml.trim().is_empty() {
		Ok(None)
	} else {
		Ok(Some(enml))
	}
}

fn restore_adjacent_inline_spacing(html: &str) -> String {
	let adjacent_inline_words = Regex::new(
		r#"(?is)([\p{L}\p{N}])</(b|i|u|s|span)>\s*<(b|i|u|s|span)([^>]*)>([\p{L}\p{N}])"#,
	)
	.unwrap();
	adjacent_inline_words
		.replace_all(html, "$1</$2>&nbsp;<$3$4>$5")
		.into_owned()
}

fn serialize_xml_fragment<T: ReadTxn>(fragment: &XmlFragmentRef, txn: &T) -> String {
	let mut out = String::new();
	for node in fragment.children(txn) {
		serialize_xml_out(node, txn, &mut out);
	}
	out
}

fn serialize_xml_out<T: ReadTxn>(node: XmlOut, txn: &T, out: &mut String) {
	match node {
		XmlOut::Element(element) => serialize_xml_element(&element, txn, out),
		XmlOut::Fragment(fragment) => out.push_str(&serialize_xml_fragment(&fragment, txn)),
		XmlOut::Text(text) => serialize_xml_text(&text, txn, out),
	}
}

fn serialize_xml_element<T: ReadTxn>(element: &XmlElementRef, txn: &T, out: &mut String) {
	let tag = element.tag();
	let mut attributes = element
		.attributes(txn)
		.map(|(key, value)| (key.to_string(), value.to_string(txn)))
		.collect::<Vec<_>>();
	attributes.sort_by(|left, right| left.0.cmp(&right.0));
	let children = element.children(txn).collect::<Vec<_>>();

	out.push('<');
	out.push_str(tag);
	for (key, value) in attributes {
		out.push(' ');
		out.push_str(&key);
		out.push_str("=\"");
		out.push_str(&encode_double_quoted_attribute(&value));
		out.push('"');
	}

	if children.is_empty() && is_self_closing_enml_tag(tag) {
		out.push_str("/>");
		return;
	}

	out.push('>');
	for child in children {
		serialize_xml_out(child, txn, out);
	}
	out.push_str("</");
	out.push_str(tag);
	out.push('>');
}

fn serialize_xml_text<T: ReadTxn>(text: &XmlTextRef, txn: &T, out: &mut String) {
	let mut previous_formatted_word_end = false;
	for diff in text.diff(txn, YChange::identity) {
		let attributes = diff
			.attributes
			.as_deref()
			.map(sorted_text_attributes)
			.unwrap_or_default();
		let (first, last) = text_insert_edges(&diff.insert);
		if previous_formatted_word_end
			&& !attributes.is_empty()
			&& first.is_some_and(is_word_character)
		{
			out.push_str("&nbsp;");
		}
		for (key, value) in &attributes {
			push_text_format_start(out, key, value);
		}
		serialize_text_insert(diff.insert, txn, out);
		for (key, _) in attributes.iter().rev() {
			out.push_str("</");
			out.push_str(text_format_tag_name(key));
			out.push('>');
		}
		previous_formatted_word_end = !attributes.is_empty() && last.is_some_and(is_word_character);
	}
}

fn text_insert_edges(insert: &Out) -> (Option<char>, Option<char>) {
	let text = match insert {
		Out::Any(Any::String(value)) => value.to_string(),
		Out::Any(value) => value.to_string(),
		Out::YXmlElement(_) | Out::YXmlFragment(_) | Out::YXmlText(_) => return (None, None),
		other => other.to_string(),
	};
	(text.chars().next(), text.chars().next_back())
}

fn is_word_character(character: char) -> bool {
	character.is_alphanumeric()
}

fn serialize_text_insert<T: ReadTxn>(insert: Out, txn: &T, out: &mut String) {
	match insert {
		Out::Any(Any::String(value)) => out.push_str(&encode_text_with_line_breaks(value.as_ref())),
		Out::Any(value) => out.push_str(&encode_text_with_line_breaks(&value.to_string())),
		Out::YXmlElement(element) => serialize_xml_element(&element, txn, out),
		Out::YXmlFragment(fragment) => out.push_str(&serialize_xml_fragment(&fragment, txn)),
		Out::YXmlText(text) => serialize_xml_text(&text, txn, out),
		other => out.push_str(&encode_text_with_line_breaks(&other.to_string(txn))),
	}
}

fn encode_text_with_line_breaks(value: &str) -> String {
	let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
	let mut out = String::new();
	for (index, line) in normalized.split('\n').enumerate() {
		if index > 0 {
			out.push_str("<br>");
		}
		out.push_str(&encode_text(line));
	}
	out
}

fn sorted_text_attributes(attributes: &yrs::types::Attrs) -> Vec<(&str, &Any)> {
	let mut items = attributes
		.iter()
		.filter(|(_, value)| !matches!(value, Any::Null | Any::Undefined | Any::Bool(false)))
		.map(|(key, value)| (key.as_ref(), value))
		.collect::<Vec<_>>();
	items.sort_by(|left, right| left.0.cmp(right.0));
	items
}

fn push_text_format_start(out: &mut String, tag: &str, value: &Any) {
	out.push('<');
	out.push_str(text_format_tag_name(tag));
	if let Any::Map(attributes) = value {
		let mut attributes = attributes.iter().collect::<Vec<_>>();
		attributes.sort_by(|left, right| left.0.cmp(right.0));
		for (key, value) in attributes {
			out.push(' ');
			out.push_str(key);
			out.push_str("=\"");
			out.push_str(&encode_double_quoted_attribute(&value.to_string()));
			out.push('"');
		}
	}
	out.push('>');
}

/// Map Evernote's rich-text mark names to HTML tags we want in generated ENML.
fn text_format_tag_name(tag: &str) -> &str {
	match tag.to_ascii_lowercase().as_str() {
		"codespan" | "inlinecode" | "inline-code" | "monospace" => "code",
		_ => tag,
	}
}

fn is_self_closing_enml_tag(tag: &str) -> bool {
	matches!(
		tag,
		"area"
			| "base" | "br"
			| "col" | "embed"
			| "en-crypt"
			| "en-media"
			| "en-todo"
			| "hr" | "img"
			| "input" | "link"
			| "meta" | "param"
			| "source"
			| "track" | "wbr"
	)
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

fn build_site(
	options: &RebuildOptions,
	site: &CacheSite,
	generation_started: Instant,
) -> Result<(GeneratedSite, Vec<Post>, BuiltSiteAnnotation)> {
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

	let annotation = annotate_built_site(
		&site_dir.join("public"),
		Utc::now(),
		generation_started.elapsed(),
	)?;

	Ok((generated, posts, annotation))
}

fn copy_resource_files(site_dir: &Path, notes: &[CachedNote], posts: &[Post]) -> Result<()> {
	for (cached, post) in notes.iter().zip(posts) {
		let content_dir = match post.kind {
			PostKind::BlogPost => site_dir.join("content/posts").join(&post.slug),
			PostKind::Page => site_dir.join("content/pages").join(&post.slug),
			PostKind::About => site_dir.join("content/pages/about"),
			PostKind::NavTag | PostKind::Config => continue,
		};
		for file in &cached.files {
			let destination = content_dir.join(&file.file_name);
			match file.transform {
				ResourceTransform::Copy => {
					fs::copy(&file.source_path, &destination).with_context(|| {
						format!(
							"failed to copy {} into {}",
							file.source_path.display(),
							content_dir.display()
						)
					})?;
				}
				ResourceTransform::ImageToAvif | ResourceTransform::DngToAvif => {
					convert_image_to_avif(&file.source_path, &destination)?;
				}
				ResourceTransform::VectorToAvif => {
					convert_vector_to_avif(&file.source_path, &destination)?;
				}
				ResourceTransform::DngEmbeddedJpeg => {
					extract_embedded_jpeg(&file.source_path, &destination).with_context(|| {
						format!(
							"failed to extract embedded JPEG from {}",
							file.source_path.display()
						)
					})?;
				}
				ResourceTransform::TrackerToOpus => {
					convert_audio_to_opus(&file.source_path, &destination)?;
				}
			}
			if let Some(original_file_name) = &file.original_file_name {
				let original_destination = content_dir.join(original_file_name);
				fs::copy(&file.source_path, &original_destination).with_context(|| {
					format!(
						"failed to copy original {} into {}",
						file.source_path.display(),
						content_dir.display()
					)
				})?;
			}
		}
	}
	Ok(())
}

/// Render an artwork-like vector document to a web-display AVIF preview.
fn convert_vector_to_avif(source: &Path, destination: &Path) -> Result<()> {
	let preview = destination.with_file_name(format!(
		".{}.preview.png",
		destination
			.file_name()
			.and_then(|file_name| file_name.to_str())
			.unwrap_or("vector")
	));
	let mut errors = Vec::new();

	if render_vector_preview(source, &preview, &mut errors)? {
		let result = convert_image_to_avif(&preview, destination);
		let _ = fs::remove_file(&preview);
		return result;
	}

	bail!(
		"failed to render vector preview for {}\n{}",
		source.display(),
		errors.join("\n")
	);
}

fn render_vector_preview(source: &Path, preview: &Path, errors: &mut Vec<String>) -> Result<bool> {
	let mut attempts = Vec::new();
	if matches!(
		file_extension_from_path(source).as_deref(),
		Some("ai" | "pdf")
	) {
		let prefix = preview.with_extension("");
		attempts.push((
			"pdftoppm".to_string(),
			vec![
				"-png".into(),
				"-singlefile".into(),
				"-f".into(),
				"1".into(),
				"-l".into(),
				"1".into(),
				source.display().to_string(),
				prefix.display().to_string(),
			],
		));
	}
	attempts.push((
		"gs".to_string(),
		vec![
			"-dSAFER".into(),
			"-dBATCH".into(),
			"-dNOPAUSE".into(),
			"-dFirstPage=1".into(),
			"-dLastPage=1".into(),
			"-sDEVICE=pngalpha".into(),
			"-r144".into(),
			format!("-sOutputFile={}", preview.display()),
			source.display().to_string(),
		],
	));
	for program in ["magick", "convert"] {
		attempts.push((
			program.to_string(),
			vec![
				format!("{}[0]", source.display()),
				"-auto-orient".into(),
				"-background".into(),
				"white".into(),
				"-alpha".into(),
				"remove".into(),
				preview.display().to_string(),
			],
		));
	}

	for (program, args) in attempts {
		let output = match Command::new(&program).args(&args).output() {
			Ok(output) => output,
			Err(error) => {
				errors.push(format!("{program} failed to launch: {error}"));
				continue;
			}
		};
		if output.status.success() && preview.exists() {
			return Ok(true);
		}
		errors.push(format!(
			"{program} exited with {}: {}",
			output.status,
			String::from_utf8_lossy(&output.stderr).trim()
		));
	}
	Ok(false)
}

/// Convert a tracker module into Opus so every generated site can play it.
fn convert_audio_to_opus(source: &Path, destination: &Path) -> Result<()> {
	let output = Command::new("ffmpeg")
		.arg("-hide_banner")
		.arg("-loglevel")
		.arg("error")
		.arg("-y")
		.arg("-i")
		.arg(source)
		.arg("-c:a")
		.arg("libopus")
		.arg("-b:a")
		.arg("160k")
		.arg(destination)
		.output()
		.with_context(|| format!("failed to execute ffmpeg for {}", source.display()))?;
	if !output.status.success() {
		bail!(
			"ffmpeg failed to convert {} to {}\nstderr:\n{}",
			source.display(),
			destination.display(),
			String::from_utf8_lossy(&output.stderr)
		);
	}
	Ok(())
}

/// Convert an Evernote browser-hostile image attachment into AVIF.
fn convert_image_to_avif(source: &Path, destination: &Path) -> Result<()> {
	let ffmpeg_output = Command::new("ffmpeg")
		.arg("-hide_banner")
		.arg("-loglevel")
		.arg("error")
		.arg("-y")
		.arg("-i")
		.arg(source)
		.arg("-frames:v")
		.arg("1")
		.arg("-c:v")
		.arg("libaom-av1")
		.arg("-still-picture")
		.arg("1")
		.arg("-crf")
		.arg("28")
		.arg("-b:v")
		.arg("0")
		.arg(destination)
		.output()
		.with_context(|| format!("failed to execute ffmpeg for {}", source.display()))?;
	if ffmpeg_output.status.success() {
		return Ok(());
	}

	let mut errors = vec![format!(
		"ffmpeg exited with {}: {}",
		ffmpeg_output.status,
		String::from_utf8_lossy(&ffmpeg_output.stderr).trim()
	)];
	for program in ["magick", "convert"] {
		let output = match Command::new(program)
			.arg(source)
			.arg("-auto-orient")
			.arg("-colorspace")
			.arg("sRGB")
			.arg("-quality")
			.arg("80")
			.arg(destination)
			.output()
		{
			Ok(output) => output,
			Err(error) => {
				errors.push(format!("{program} failed to launch: {error}"));
				continue;
			}
		};
		if output.status.success() {
			return Ok(());
		}
		errors.push(format!(
			"{program} exited with {}: {}",
			output.status,
			String::from_utf8_lossy(&output.stderr).trim()
		));
	}

	bail!(
		"failed to convert {} to {}\n{}",
		source.display(),
		destination.display(),
		errors.join("\n")
	)
}

/// Extract the largest embedded JPEG preview from a DNG-like file.
fn extract_embedded_jpeg(source: &Path, destination: &Path) -> Result<()> {
	let bytes = fs::read(source).with_context(|| format!("failed to read {}", source.display()))?;
	let jpeg = largest_jpeg_span(&bytes)
		.ok_or_else(|| anyhow!("no embedded JPEG preview found in {}", source.display()))?;
	fs::write(destination, jpeg).with_context(|| {
		format!(
			"failed to write embedded JPEG preview to {}",
			destination.display()
		)
	})
}

/// Return the largest byte range that looks like a complete JPEG stream.
fn largest_jpeg_span(bytes: &[u8]) -> Option<&[u8]> {
	let mut largest = None::<(usize, usize)>;
	let mut index = 0;
	while index + 2 < bytes.len() {
		if bytes[index] == 0xff && bytes[index + 1] == 0xd8 && bytes[index + 2] == 0xff {
			let mut end = None;
			let mut cursor = index + 2;
			while cursor + 1 < bytes.len() {
				if bytes[cursor] == 0xff && bytes[cursor + 1] == 0xd9 {
					end = Some(cursor + 2);
					break;
				}
				cursor += 1;
			}
			if let Some(end) = end {
				if largest.is_none_or(|(start, previous_end)| end - index > previous_end - start) {
					largest = Some((index, end));
				}
				index = end;
				continue;
			}
		}
		index += 1;
	}
	largest.map(|(start, end)| &bytes[start..end])
}

/// Return how an Evernote image resource should be written into the static site.
fn resource_transform(
	file_name: &str,
	mime: &str,
	source_path: Option<&Path>,
) -> ResourceTransform {
	if is_dng_image(file_name, mime) {
		if source_path.is_some_and(dng_has_embedded_jpeg) {
			ResourceTransform::DngEmbeddedJpeg
		} else {
			ResourceTransform::DngToAvif
		}
	} else if is_tracker_module(file_name, mime) {
		ResourceTransform::TrackerToOpus
	} else if is_vector_artwork(file_name, mime) {
		ResourceTransform::VectorToAvif
	} else if should_convert_image_to_avif(file_name, mime) {
		ResourceTransform::ImageToAvif
	} else {
		ResourceTransform::Copy
	}
}

/// Return true for module/tracker music that browsers cannot play natively.
fn is_tracker_module(file_name: &str, mime: &str) -> bool {
	let mime = mime.to_ascii_lowercase();
	matches!(
		mime.as_str(),
		"audio/mod"
			| "audio/x-mod"
			| "audio/x-xm"
			| "audio/x-s3m"
			| "audio/x-it"
			| "audio/x-stm"
			| "audio/x-mtm"
			| "audio/x-669"
	) || matches!(
		file_extension(file_name).as_deref(),
		Some(
			"mod"
				| "xm" | "s3m"
				| "it" | "stm"
				| "mtm" | "669"
				| "ult" | "far"
				| "med" | "okt"
				| "amf" | "ams"
				| "dbm" | "dmf"
				| "dsm" | "ptm"
				| "umx"
		)
	)
}

/// Return true for vector artwork containers that need a raster preview.
fn is_vector_artwork(file_name: &str, mime: &str) -> bool {
	let mime = mime.to_ascii_lowercase();
	matches!(
		mime.as_str(),
		"application/illustrator"
			| "application/postscript"
			| "application/eps"
			| "image/x-eps"
			| "image/svg+xml-compressed"
	) || matches!(
		file_extension(file_name).as_deref(),
		Some("ai" | "eps" | "epsf" | "ps" | "svgz")
	)
}

/// Check whether a DNG file has an embedded JPEG preview without decoding RAW.
fn dng_has_embedded_jpeg(path: &Path) -> bool {
	fs::read(path)
		.ok()
		.and_then(|bytes| largest_jpeg_span(&bytes).map(|_| ()))
		.is_some()
}

/// Return true for image resources that browsers cannot reliably display.
fn should_convert_image_to_avif(file_name: &str, mime: &str) -> bool {
	if browser_displayable_image(file_name, mime) {
		return false;
	}
	is_image_resource(file_name, mime)
}

/// Return true for image resources that browser engines can usually show.
fn browser_displayable_image(file_name: &str, mime: &str) -> bool {
	let mime = mime.to_ascii_lowercase();
	matches!(
		mime.as_str(),
		"image/jpeg"
			| "image/png"
			| "image/gif"
			| "image/webp"
			| "image/avif"
			| "image/svg+xml"
			| "image/x-icon"
			| "image/vnd.microsoft.icon"
	) || matches!(
		file_extension(file_name).as_deref(),
		Some("jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "svg" | "ico")
	)
}

/// Return true for image-like resources, including common RAW containers.
fn is_image_resource(file_name: &str, mime: &str) -> bool {
	let mime = mime.to_ascii_lowercase();
	mime.starts_with("image/")
		|| matches!(
			file_extension(file_name).as_deref(),
			Some(
				"bmp"
					| "tif" | "tiff"
					| "heic" | "heif"
					| "heics" | "heifs"
					| "jxl" | "jp2" | "j2k"
					| "j2c" | "jpf" | "psd"
					| "psb" | "tga" | "ppm"
					| "pgm" | "pbm" | "pnm"
					| "exr" | "hdr" | "dds"
					| "cr2" | "cr3" | "nef"
					| "nrw" | "arw" | "raf"
					| "rw2" | "orf" | "pef"
					| "srw" | "x3f" | "erf"
					| "kdc" | "dcr" | "mos"
			)
		)
}

/// Return true for DNG RAW resources that need a web-display derivative.
fn is_dng_image(file_name: &str, mime: &str) -> bool {
	let mime = mime.to_ascii_lowercase();
	matches!(mime.as_str(), "image/x-adobe-dng" | "image/dng")
		|| file_extension(file_name).as_deref() == Some("dng")
}

/// Replace a safe filename's extension for generated derivative images.
fn file_name_with_extension(file_name: &str, fallback_hash: &str, extension: &str) -> String {
	let stem = Path::new(file_name)
		.file_stem()
		.and_then(|stem| stem.to_str())
		.map(str::trim)
		.filter(|stem| !stem.is_empty())
		.unwrap_or(fallback_hash);
	safe_file_name(&format!("{stem}.{extension}"), fallback_hash)
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

/// Choose the best available plain text source from the Evernote cache.
fn note_plain_text(content: &str, snippet: &str, rte_text: Option<&str>) -> String {
	if !content.trim().is_empty() {
		normalize_indexed_plain_text(content)
	} else if !snippet.trim().is_empty() {
		normalize_indexed_plain_text(snippet)
	} else {
		normalize_plain_text(rte_text.unwrap_or(""))
	}
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ContentMetadata {
	tags: Vec<String>,
	source_url: Option<String>,
	raw_line: Option<String>,
}

/// Split a final metadata line from the rendered body.
///
/// The line is treated as metadata only when every token is `#tag`, a bare
/// `slug:value`, or one bare `http(s)` source URL. `#slug:value` is
/// intentionally not accepted. The returned body excludes that metadata line.
fn extract_trailing_content_metadata(text: &str) -> (String, ContentMetadata) {
	let mut lines = text.lines().collect::<Vec<_>>();
	let Some(tag_line_index) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
		return (text.into(), ContentMetadata::default());
	};
	let metadata = parse_content_metadata_line(lines[tag_line_index]);
	if metadata.tags.is_empty() && metadata.source_url.is_none() {
		return (text.into(), ContentMetadata::default());
	}
	let mut metadata = metadata;
	metadata.raw_line = Some(lines[tag_line_index].trim().to_string());
	lines.remove(tag_line_index);
	while lines.last().is_some_and(|line| line.trim().is_empty()) {
		lines.pop();
	}
	(lines.join("\n"), metadata)
}

/// Parse one content metadata line into tags understood by the post mapper.
fn parse_content_metadata_line(line: &str) -> ContentMetadata {
	let tokens = line.split_whitespace().collect::<Vec<_>>();
	if tokens.is_empty() || tokens.iter().any(|token| !is_content_metadata_token(token)) {
		return ContentMetadata::default();
	}
	let mut metadata = ContentMetadata::default();
	for token in tokens {
		if is_metadata_url(token) {
			metadata.source_url.get_or_insert_with(|| token.to_string());
		} else {
			let tag = token.strip_prefix('#').unwrap_or(token).trim();
			if !tag.is_empty() {
				metadata.tags.push(tag.to_string());
			}
		}
	}
	metadata
}

/// Return whether a final-line token is supported note metadata.
fn is_content_metadata_token(token: &str) -> bool {
	is_metadata_url(token)
		|| token.starts_with("slug:")
		|| (token.starts_with('#') && !token.starts_with("#slug:"))
}

fn is_metadata_url(token: &str) -> bool {
	token.starts_with("https://") || token.starts_with("http://")
}

fn enml_plain_text(enml: &str) -> String {
	let block_breaks =
		Regex::new(r"(?i)<br\s*/?>|</(?:div|p|li|tr|h[1-6]|blockquote|pre|en-note)>")
			.expect("valid ENML block regex");
	let with_breaks = block_breaks.replace_all(enml, "\n");
	let tags = Regex::new(r"(?s)<[^>]+>").expect("valid ENML tag regex");
	let text = tags.replace_all(&with_breaks, "");
	decode_html_entities(&text)
		.lines()
		.map(str::trim)
		.collect::<Vec<_>>()
		.join("\n")
}

/// Convert plain text from the Evernote cache into the subset of ENML that the
/// renderer already understands.
fn plain_text_to_enml(text: &str, source_url: Option<&str>, resources: &[Resource]) -> String {
	let mut enml = String::from("<en-note>");
	if let Some(url) = source_url.filter(|url| !url.trim().is_empty()) {
		push_source_url_enml(&mut enml, url);
	}
	for raw_paragraph in text.split("\n\n") {
		let paragraph = raw_paragraph.trim_matches('\n');
		if paragraph.trim().is_empty() {
			continue;
		}
		if let Some(code) = fenced_code(paragraph) {
			push_code_markdown(&mut enml, &code.body);
		} else if is_code_paragraph(paragraph) {
			push_code_markdown(&mut enml, paragraph.trim());
		} else {
			push_text_paragraph_enml(&mut enml, paragraph.trim());
		}
	}
	for resource in resources {
		push_resource_enml(&mut enml, resource);
	}
	enml.push_str("</en-note>");
	enml
}

fn rich_text_to_enml(
	rte_enml: &str,
	metadata_line: Option<&str>,
	source_url: Option<&str>,
	resources: &[Resource],
) -> String {
	let mut enml = metadata_line
		.map(|line| strip_trailing_metadata_enml(rte_enml, line))
		.unwrap_or_else(|| rte_enml.to_string());
	if let Some(url) = source_url.filter(|url| !url.trim().is_empty()) {
		enml = prepend_source_url_to_enml(&enml, url);
	}
	append_missing_resources_enml(&enml, resources)
}

fn strip_trailing_metadata_enml(enml: &str, metadata_line: &str) -> String {
	let encoded_line = encode_text(metadata_line);
	let escaped_line = regex::escape(encoded_line.as_ref());
	for tag in ["div", "p"] {
		let before_close = Regex::new(&format!(
			r#"(?is)<{tag}\b[^>]*>\s*{escaped_line}\s*</{tag}>\s*(</en-note>\s*)$"#
		))
		.unwrap();
		if before_close.is_match(enml) {
			return before_close.replace(enml, "$1").into_owned();
		}

		let at_end = Regex::new(&format!(
			r#"(?is)<{tag}\b[^>]*>\s*{escaped_line}\s*</{tag}>\s*$"#
		))
		.unwrap();
		if at_end.is_match(enml) {
			return at_end.replace(enml, "").into_owned();
		}

		let exact_block = Regex::new(&format!(
			r#"(?is)<{tag}\b[^>]*>\s*{escaped_line}\s*</{tag}>"#
		))
		.unwrap();
		if let Some(block) = exact_block.find_iter(enml).last() {
			let mut out = String::with_capacity(enml.len() - block.len());
			out.push_str(&enml[..block.start()]);
			out.push_str(&enml[block.end()..]);
			return out;
		}
	}
	enml.to_string()
}

fn prepend_source_url_to_enml(enml: &str, url: &str) -> String {
	let mut source = String::new();
	push_source_url_enml(&mut source, url);
	let opening_en_note = Regex::new(r#"(?is)<en-note\b[^>]*>"#).unwrap();
	if let Some(location) = opening_en_note.find(enml) {
		let mut out = String::with_capacity(enml.len() + source.len());
		out.push_str(&enml[..location.end()]);
		out.push_str(&source);
		out.push_str(&enml[location.end()..]);
		out
	} else {
		format!("<en-note>{source}{enml}</en-note>")
	}
}

fn append_missing_resources_enml(enml: &str, resources: &[Resource]) -> String {
	let lower_enml = enml.to_ascii_lowercase();
	let mut missing = String::new();
	for resource in resources {
		if !lower_enml.contains(&resource.hash.to_ascii_lowercase()) {
			push_resource_enml(&mut missing, resource);
		}
	}
	if missing.is_empty() {
		return enml.to_string();
	}

	let lower = enml.to_ascii_lowercase();
	if let Some(index) = lower.rfind("</en-note>") {
		let mut out = String::with_capacity(enml.len() + missing.len());
		out.push_str(&enml[..index]);
		out.push_str(&missing);
		out.push_str(&enml[index..]);
		out
	} else {
		let mut out = enml.to_string();
		out.push_str(&missing);
		out
	}
}

fn push_resource_enml(enml: &mut String, resource: &Resource) {
	enml.push_str("<p><en-media type=\"");
	enml.push_str(&encode_double_quoted_attribute(&resource.mime));
	enml.push_str("\" hash=\"");
	enml.push_str(&encode_double_quoted_attribute(&resource.hash));
	enml.push_str("\"/></p>");
}

fn push_source_url_enml(enml: &mut String, url: &str) {
	let href = encode_double_quoted_attribute(url);
	enml.push_str("<p><a href=\"");
	enml.push_str(&href);
	enml.push_str("\">");
	enml.push_str(&encode_text(url));
	enml.push_str("</a></p>\n\n");
}

fn push_text_paragraph_enml(enml: &mut String, paragraph: &str) {
	enml.push_str("<p>");
	for (index, line) in paragraph.lines().enumerate() {
		if index > 0 {
			enml.push_str("<br>");
		}
		enml.push_str(&encode_text(line));
	}
	enml.push_str("</p>");
}

fn push_code_markdown(enml: &mut String, code: &str) {
	let code = code.trim_matches('\n');
	let fence = markdown_code_fence(code);
	enml.push_str("\n\n");
	enml.push_str(&fence);
	enml.push('\n');
	enml.push_str(code);
	enml.push('\n');
	enml.push_str(&fence);
	enml.push_str("\n\n");
}

struct CodeBlock {
	body: String,
}

fn fenced_code(paragraph: &str) -> Option<CodeBlock> {
	let trimmed = paragraph.trim();
	let mut lines = trimmed.lines();
	let first = lines.next()?;
	first.strip_prefix("```")?;
	let mut body = lines.collect::<Vec<_>>();
	if body.last().is_some_and(|line| line.trim() == "```") {
		body.pop();
	}
	Some(CodeBlock {
		body: body.join("\n"),
	})
}

fn markdown_code_fence(code: &str) -> String {
	let max_run = Regex::new(r"`+")
		.unwrap()
		.find_iter(code)
		.map(|m| m.as_str().len())
		.max()
		.unwrap_or(0);
	"`".repeat(usize::max(3, max_run + 1))
}

fn is_code_paragraph(paragraph: &str) -> bool {
	let lines = paragraph
		.lines()
		.map(str::trim)
		.filter(|line| !line.is_empty())
		.collect::<Vec<_>>();
	if lines.is_empty() {
		return false;
	}
	if paragraph
		.lines()
		.any(|line| line.starts_with(' ') || line.starts_with('\t'))
	{
		return true;
	}
	let code_lines = lines.iter().filter(|line| is_code_line(line)).count();
	code_lines > 0
		&& lines
			.iter()
			.all(|line| is_code_line(line) || line.eq_ignore_ascii_case("or"))
}

fn is_code_line(line: &str) -> bool {
	let lower = line.to_ascii_lowercase();
	let first = lower.split_whitespace().next().unwrap_or("");
	matches!(
		first,
		"select"
			| "update"
			| "insert"
			| "delete"
			| "create"
			| "alter" | "drop"
			| "with" | "explain"
			| "vacuum"
			| "grant" | "revoke"
			| "begin" | "commit"
			| "rollback"
			| "copy" | "truncate"
			| "analyze"
	) || line.starts_with('\\')
		|| line.starts_with("$ ")
		|| line.starts_with("> ")
		|| (line.ends_with(';') && (line.contains('=') || line.contains('(')))
}

fn normalize_indexed_plain_text(text: &str) -> String {
	normalize_plain_text(text).replace("/n", "\n")
}

fn normalize_plain_text(text: &str) -> String {
	text.replace("\r\n", "\n").replace('\r', "\n")
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

fn bool_to_int(value: bool) -> i64 {
	i64::from(value)
}

fn sqlite_now() -> String {
	Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
	use super::*;
	use evernote_edam::types as edam_types;
	use pretty_assertions::assert_eq;

	const TEST_USER_STORE_URL: &str = crate::evernote_api::DEFAULT_USER_STORE_URL;

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
		assert!(site.notes[0].note.enml.contains("Cached note body"));
		assert_eq!(site.notes[0].files.len(), 1);
		assert_eq!(site.notes[0].note.resources[0].size_bytes, Some(5));
	}

	#[test]
	fn maps_api_note_to_cached_note_with_resources_and_body_tags() {
		let temp = tempfile::tempdir().unwrap();
		let note = edam_types::Note {
			guid: Some("note-guid".into()),
			title: Some(" API note ".into()),
			content: Some(
				r#"<en-note><div>Hello API</div><div>#postgres slug:api-note</div><en-media type="text/plain" hash="abcd"/></en-note>"#
					.into(),
			),
			content_hash: None,
			content_length: None,
			created: Some(1_700_000_000_000),
			updated: Some(1_700_000_100_000),
			deleted: None,
			active: Some(true),
			update_sequence_num: None,
			notebook_guid: Some("notebook-guid".into()),
			tag_guids: None,
			resources: Some(vec![edam_types::Resource {
				guid: Some("resource-guid".into()),
				note_guid: Some("note-guid".into()),
				data: Some(edam_types::Data {
					body_hash: Some(vec![0xab, 0xcd]),
					size: Some(11),
					body: Some(b"hello world".to_vec()),
				}),
				mime: Some("text/plain".into()),
				width: None,
				height: None,
				duration: None,
				active: Some(true),
				recognition: None,
				attributes: Some(edam_types::ResourceAttributes {
					source_u_r_l: None,
					timestamp: None,
					latitude: None,
					longitude: None,
					altitude: None,
					camera_make: None,
					camera_model: None,
					client_will_index: None,
					reco_type: None,
					file_name: Some("readme.txt".into()),
					attachment: Some(true),
					application_data: None,
				}),
				update_sequence_num: None,
				alternate_data: None,
			}]),
			attributes: Some(edam_types::NoteAttributes {
				subject_date: None,
				latitude: None,
				longitude: None,
				altitude: None,
				author: None,
				source: None,
				source_u_r_l: Some("https://example.com/source".into()),
				source_application: None,
				share_date: None,
				reminder_order: None,
				reminder_done_time: None,
				reminder_time: None,
				place_name: None,
				content_class: None,
				application_data: None,
				last_edited_by: None,
				classifications: None,
				creator_id: None,
				last_editor_id: None,
				shared_with_business: None,
				conflict_source_note_guid: None,
				note_title_quality: None,
			}),
			tag_names: None,
			shared_notes: None,
			restrictions: None,
			limits: None,
		};

		let cached = api_note_to_cached_note(
			temp.path(),
			"notebook-guid",
			DownloadedNote {
				note,
				tag_names: Vec::new(),
			},
		)
		.unwrap();

		assert_eq!(cached.note.title, "API note");
		assert_eq!(cached.note.tags, vec!["postgres", "slug:api-note"]);
		assert!(cached.note.enml.contains("https://example.com/source"));
		assert!(!cached.note.enml.contains("slug:api-note"));
		assert_eq!(cached.note.resources[0].hash, "abcd");
		assert_eq!(cached.note.resources[0].file_name, "readme.txt");
		assert_eq!(
			cached.note.resources[0].text_preview.as_deref(),
			Some("hello world")
		);
		assert_eq!(cached.note.resources[0].size_bytes, Some(11));
		assert_eq!(
			fs::read(&cached.files[0].source_path).unwrap(),
			b"hello world"
		);
	}

	#[test]
	fn api_note_download_cache_reuses_current_note_and_resources() {
		let temp = tempfile::tempdir().unwrap();
		let mut cache = ApiNoteDownloadCache::new(temp.path());
		let mut note = api_test_note(
			"note-guid",
			"API note",
			"<en-note><div>Hello API</div></en-note>",
		);
		note.resources = Some(vec![api_test_text_resource()]);
		cache
			.put_cached_note(
				"notebook-guid",
				&DownloadedNote {
					note,
					tag_names: vec!["api-tag".into()],
				},
			)
			.unwrap();

		let metadata = NoteSummary {
			guid: "note-guid".into(),
			title: Some("API note".into()),
			created: Some(1_700_000_000_000),
			updated: Some(1_700_000_100_000),
			largest_resource_mime: Some("text/plain".into()),
			largest_resource_size: Some(11),
		};
		let cached = cache
			.get_cached_note("notebook-guid", &metadata)
			.unwrap()
			.unwrap();

		assert_eq!(cached.tag_names, vec!["api-tag"]);
		assert_eq!(cached.note.title.as_deref(), Some("API note"));
		assert_eq!(
			cached.note.tag_names.as_deref(),
			Some(["api-tag".to_string()].as_slice())
		);
		let resource = cached.note.resources.as_ref().unwrap().first().unwrap();
		assert_eq!(resource.mime.as_deref(), Some("text/plain"));
		assert_eq!(
			resource
				.attributes
				.as_ref()
				.and_then(|attributes| attributes.file_name.as_deref()),
			Some("readme.txt")
		);
		assert_eq!(
			resource.data.as_ref().and_then(|data| data.body.as_deref()),
			Some(b"hello world".as_slice())
		);

		let stale_metadata = NoteSummary {
			updated: Some(1_700_000_100_001),
			..metadata
		};
		assert!(
			cache
				.get_cached_note("notebook-guid", &stale_metadata)
				.unwrap()
				.is_none()
		);
	}

	#[test]
	fn api_note_download_cache_misses_when_resource_body_is_missing() {
		let temp = tempfile::tempdir().unwrap();
		let mut cache = ApiNoteDownloadCache::new(temp.path());
		let mut note = api_test_note(
			"note-guid",
			"API note",
			"<en-note><div>Hello API</div></en-note>",
		);
		note.resources = Some(vec![api_test_text_resource()]);
		cache
			.put_cached_note(
				"notebook-guid",
				&DownloadedNote {
					note,
					tag_names: Vec::new(),
				},
			)
			.unwrap();
		let resource_dir = temp
			.path()
			.join(safe_file_name("notebook-guid", "notebook"))
			.join(safe_file_name("note-guid", "note"))
			.join("resources");
		for entry in fs::read_dir(resource_dir).unwrap() {
			fs::remove_file(entry.unwrap().path()).unwrap();
		}

		let metadata = NoteSummary {
			guid: "note-guid".into(),
			title: Some("API note".into()),
			created: Some(1_700_000_000_000),
			updated: Some(1_700_000_100_000),
			largest_resource_mime: Some("text/plain".into()),
			largest_resource_size: Some(11),
		};

		assert!(
			cache
				.get_cached_note("notebook-guid", &metadata)
				.unwrap()
				.is_none()
		);
	}

	#[test]
	fn keeps_api_downloads_in_hidden_sites_subdirectory() {
		assert_eq!(
			api_download_root(Path::new("/var/cache/everpublich/sites")),
			Path::new("/var/cache/everpublich/sites/.evernote-api-resources")
		);
	}

	#[test]
	fn api_notebook_reuses_existing_user_by_notebook_guid() {
		let fixture = CacheFixture::new();
		let app_db = fixture.app_db();
		app_db
			.execute(
				"insert into users (
					user_id, site_slug, site_title, registration_date_utc,
					shared_notebook_guid, shared_notebook_name, home_page_mode
				) values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
				params![
					"legacy-user",
					"postgres",
					"postgres",
					"2026-07-06T00:00:00Z",
					"notebook-guid",
					"postgres",
					"full_posts",
				],
			)
			.unwrap();

		let site = api_notebook_to_site(
			&fixture.options(),
			&app_db,
			&fixture.sites_dir.join(".evernote-api-resources"),
			DownloadedLinkedNotebook {
				share_name: Some("postgres".into()),
				owner_username: Some("owner".into()),
				notebook_guid: "notebook-guid".into(),
				notes: vec![DownloadedNote {
					note: api_test_note(
						"note-guid",
						"API note",
						"<en-note><div>Hello API</div></en-note>",
					),
					tag_names: Vec::new(),
				}],
			},
		)
		.unwrap()
		.unwrap();

		assert_eq!(site.user.user_id, "legacy-user");
		assert_eq!(site.user.settings.subdomain, "postgres");
	}

	#[test]
	fn skips_attachment_when_cache_binary_is_missing() {
		let fixture = CacheFixture::new();

		let attachment = cached_attachment(
			&fixture.config_dir,
			"missing-pdf",
			"manual.pdf",
			"application/pdf",
		)
		.unwrap();

		assert!(attachment.is_none());
	}

	#[test]
	fn formats_archive_entries_as_tree_output() {
		let entries = vec![
			"docs/".to_string(),
			"docs/readme.txt".to_string(),
			"readme.md".to_string(),
			"src/main.rs".to_string(),
		];
		let expected = ".\n|-- docs\n|   `-- readme.txt\n|-- readme.md\n`-- src\n    `-- main.rs";

		assert_eq!(archive_entries_to_tree_fallback(&entries), expected);
		assert_eq!(archive_entries_to_tree(&entries), expected);
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
		assert!(html.contains("<!-- Everpublich build:"));
		assert!(html.contains("\n  generated_at: "));
		assert!(html.contains("\n  generation_time: "));
		assert!(html.contains("\n  total_size: "));
		assert!(html.contains("\n  brotli_size: "));
		assert!(html.contains("\n  brotli_savings: "));
		assert!(html.contains("Cached note body"));
		assert!(html.contains("<audio controls"));
		assert!(
			fixture
				.sites_dir
				.join("public-notebook/public/posts/hello-from-cache/episode.mp3")
				.exists()
		);
	}

	#[test]
	fn finds_resource_cache_blobs_by_hash() {
		let temp = tempfile::tempdir().unwrap();
		let config_dir = temp.path().join("Evernote");
		let resource_dir = config_dir
			.join("resource-cache")
			.join("User42")
			.join("note-guid");
		fs::create_dir_all(&resource_dir).unwrap();
		fs::write(resource_dir.join("abc123.meta"), "metadata").unwrap();
		fs::write(resource_dir.join("abc123"), "image").unwrap();

		let found = find_cached_resource_file(&config_dir, "ABC123").unwrap();

		assert_eq!(found, Some(resource_dir.join("abc123")));
	}

	#[test]
	fn classifies_browser_hostile_media_derivatives() {
		let temp = tempfile::tempdir().unwrap();
		let dng = temp.path().join("raw.dng");
		fs::write(
			&dng,
			[b"II*\0".as_slice(), &[0xff, 0xd8, 0xff, 0x10, 0xff, 0xd9]].concat(),
		)
		.unwrap();

		assert_eq!(
			resource_transform("IMG.HEIC", "image/heic", None),
			ResourceTransform::ImageToAvif
		);
		assert_eq!(
			resource_transform("scan.tiff", "image/tiff", None),
			ResourceTransform::ImageToAvif
		);
		assert_eq!(
			resource_transform("raw.nef", "application/octet-stream", None),
			ResourceTransform::ImageToAvif
		);
		assert_eq!(
			resource_transform("design.psd", "image/vnd.adobe.photoshop", None),
			ResourceTransform::ImageToAvif
		);
		assert_eq!(
			resource_transform("poster.ai", "application/illustrator", None),
			ResourceTransform::VectorToAvif
		);
		assert_eq!(
			resource_transform("poster.eps", "application/postscript", None),
			ResourceTransform::VectorToAvif
		);
		assert_eq!(
			resource_transform("raw.dng", "image/x-adobe-dng", Some(&dng)),
			ResourceTransform::DngEmbeddedJpeg
		);
		assert_eq!(
			resource_transform("song.xm", "audio/x-xm", None),
			ResourceTransform::TrackerToOpus
		);
		assert_eq!(
			file_name_with_extension("IMG.HEIC", "hash", "avif"),
			"IMG.avif"
		);
		assert_eq!(
			file_name_with_extension("raw.dng", "hash", "jpg"),
			"raw.jpg"
		);
		assert_eq!(
			file_name_with_extension("song.xm", "hash", "opus"),
			"song.opus"
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
		let text = note_plain_text(
			"Chapter 8. Data Types/nTable of Contents/n8.1. /nNumeric Types",
			"",
			None,
		);
		let enml = plain_text_to_enml(&text, None, &[]);

		assert_eq!(
			enml,
			"<en-note><p>Chapter 8. Data Types<br>Table of Contents<br>8.1. <br>Numeric Types</p></en-note>"
		);
	}

	#[test]
	fn extracts_content_tags_from_last_metadata_line() {
		let (body, metadata) = extract_trailing_content_metadata(
			"Chapter 8. Data Types\n\nTable of Contents\n\n#postgres #page slug:data-types https://example.com/source",
		);

		assert_eq!(body, "Chapter 8. Data Types\n\nTable of Contents");
		assert_eq!(metadata.tags, vec!["postgres", "page", "slug:data-types"]);
		assert_eq!(
			metadata.source_url.as_deref(),
			Some("https://example.com/source")
		);
		assert!(!plain_text_to_enml(&body, None, &[]).contains("slug:data-types"));
	}

	#[test]
	fn keeps_prose_hashtags_in_body() {
		let (body, metadata) = extract_trailing_content_metadata("I like #postgres in prose");

		assert_eq!(body, "I like #postgres in prose");
		assert_eq!(metadata, ContentMetadata::default());
	}

	#[test]
	fn rejects_hash_prefixed_slug_metadata() {
		let (body, metadata) =
			extract_trailing_content_metadata("Body\n\n#postgres #slug:data-types");

		assert_eq!(body, "Body\n\n#postgres #slug:data-types");
		assert_eq!(metadata, ContentMetadata::default());
	}

	#[test]
	fn rejects_unknown_metadata_token() {
		let (body, metadata) = extract_trailing_content_metadata("Body\n\n#postgres language:sql");

		assert_eq!(body, "Body\n\n#postgres language:sql");
		assert_eq!(metadata, ContentMetadata::default());
	}

	#[test]
	fn renders_source_url_first_and_code_as_fenced_markdown() {
		let enml = plain_text_to_enml(
			"UPDATE activities SET tag = replace(tag, 'v', '');",
			Some("https://example.com/source"),
			&[],
		);

		assert!(enml.contains(r#"<p><a href="https://example.com/source">"#));
		assert!(enml.contains("```\nUPDATE activities SET tag = replace(tag, 'v', '');\n```"));
	}

	#[test]
	fn decodes_rte_yjs_doc_to_enml() {
		use yrs::{ReadTxn, StateVector, Transact, XmlElementPrelim, XmlFragment, XmlTextPrelim};

		let doc = Doc::new();
		let content = doc.get_or_insert_xml_fragment("content");
		let mut txn = doc.transact_mut();
		let en_note = content.push_back(&mut txn, XmlElementPrelim::empty("en-note"));
		let div = en_note.push_back(&mut txn, XmlElementPrelim::empty("div"));
		div.insert_attribute(&mut txn, "style", "color: red");
		div.push_back(&mut txn, XmlTextPrelim::new("Hello <unsafe> & "));
		let bold = div.push_back(&mut txn, XmlElementPrelim::empty("b"));
		bold.push_back(&mut txn, XmlTextPrelim::new("bold"));
		let table = en_note.push_back(&mut txn, XmlElementPrelim::empty("table"));
		let row = table.push_back(&mut txn, XmlElementPrelim::empty("tr"));
		let cell = row.push_back(&mut txn, XmlElementPrelim::empty("td"));
		cell.push_back(&mut txn, XmlTextPrelim::new("cell"));
		drop(txn);

		let update = doc
			.transact()
			.encode_state_as_update_v1(&StateVector::default());
		let enml = rte_doc_to_enml(&update).unwrap().unwrap();

		assert!(enml.contains(r#"<div style="color: red">"#));
		assert!(enml.contains("Hello &lt;unsafe&gt; &amp; "));
		assert!(enml.contains("<b>bold</b>"));
		assert!(enml.contains("<table><tr><td>cell</td></tr></table>"));
	}

	#[test]
	fn decodes_rte_yjs_doc_with_evernote_rich_blocks() {
		use std::collections::HashMap;
		use std::sync::Arc;
		use yrs::types::Attrs;
		use yrs::{
			Any, ReadTxn, StateVector, Text, Transact, XmlElementPrelim, XmlFragment, XmlTextPrelim,
		};

		let doc = Doc::new();
		let content = doc.get_or_insert_xml_fragment("content");
		let mut txn = doc.transact_mut();
		let en_note = content.push_back(&mut txn, XmlElementPrelim::empty("en-note"));

		let heading = en_note.push_back(&mut txn, XmlElementPrelim::empty("div"));
		let styled_text = heading.push_back(&mut txn, XmlTextPrelim::new(""));
		let span_attrs = Attrs::from([(
			"span".into(),
			Any::Map(Arc::new(HashMap::from([(
				"style".into(),
				Any::String("font-family: Georgia; font-size: 20px; color: #207a4d".into()),
			)]))),
		)]);
		styled_text.insert_with_attributes(&mut txn, 0, "Styled heading", span_attrs);

		let paragraph = en_note.push_back(&mut txn, XmlElementPrelim::empty("p"));
		let link = paragraph.push_back(&mut txn, XmlElementPrelim::empty("a"));
		link.insert_attribute(&mut txn, "href", "https://example.com/?a=1&b=2");
		link.push_back(&mut txn, XmlTextPrelim::new("Reference"));
		paragraph.push_back(&mut txn, XmlTextPrelim::new(" "));
		let todo = paragraph.push_back(&mut txn, XmlElementPrelim::empty("en-todo"));
		todo.insert_attribute(&mut txn, "checked", "true");

		let table = en_note.push_back(&mut txn, XmlElementPrelim::empty("table"));
		let colgroup = table.push_back(&mut txn, XmlElementPrelim::empty("colgroup"));
		let col = colgroup.push_back(&mut txn, XmlElementPrelim::empty("col"));
		col.insert_attribute(&mut txn, "style", "width: 42px");
		let tbody = table.push_back(&mut txn, XmlElementPrelim::empty("tbody"));
		let row = tbody.push_back(&mut txn, XmlElementPrelim::empty("tr"));
		let cell = row.push_back(&mut txn, XmlElementPrelim::empty("td"));
		cell.insert_attribute(&mut txn, "style", "background-color: #fff");
		cell.push_back(&mut txn, XmlTextPrelim::new("table cell"));

		let media = en_note.push_back(&mut txn, XmlElementPrelim::empty("en-media"));
		media.insert_attribute(&mut txn, "hash", "abc123");
		media.insert_attribute(&mut txn, "type", "image/png");
		drop(txn);

		let update = doc
			.transact()
			.encode_state_as_update_v1(&StateVector::default());
		let enml = rte_doc_to_enml(&update).unwrap().unwrap();

		assert!(enml.contains(
			r#"<span style="font-family: Georgia; font-size: 20px; color: #207a4d">Styled heading</span>"#
		));
		assert!(enml.contains(r#"<a href="https://example.com/?a=1&amp;b=2">Reference</a>"#));
		assert!(enml.contains(r#"<en-todo checked="true"/>"#));
		assert!(enml.contains(r#"<col style="width: 42px"/>"#));
		assert!(enml.contains(r#"<td style="background-color: #fff">table cell</td>"#));
		assert!(enml.contains(r#"<en-media hash="abc123" type="image/png"/>"#));
	}

	#[test]
	fn keeps_spaces_between_adjacent_formatted_words() {
		use yrs::types::Attrs;
		use yrs::{
			Any, ReadTxn, StateVector, Text, Transact, XmlElementPrelim, XmlFragment, XmlTextPrelim,
		};

		let doc = Doc::new();
		let content = doc.get_or_insert_xml_fragment("content");
		let mut txn = doc.transact_mut();
		let en_note = content.push_back(&mut txn, XmlElementPrelim::empty("en-note"));
		let div = en_note.push_back(&mut txn, XmlElementPrelim::empty("div"));
		let text = div.push_back(&mut txn, XmlTextPrelim::new(""));
		text.insert_with_attributes(
			&mut txn,
			0,
			"Bold",
			Attrs::from([("b".into(), Any::Bool(true))]),
		);
		text.insert_with_attributes(
			&mut txn,
			4,
			"italic ",
			Attrs::from([("i".into(), Any::Bool(true))]),
		);
		text.insert_with_attributes(
			&mut txn,
			11,
			"under",
			Attrs::from([("u".into(), Any::Bool(true))]),
		);
		text.insert_with_attributes(
			&mut txn,
			16,
			" strike",
			Attrs::from([("s".into(), Any::Bool(true))]),
		);
		drop(txn);

		let update = doc
			.transact()
			.encode_state_as_update_v1(&StateVector::default());
		let enml = rte_doc_to_enml(&update).unwrap().unwrap();

		assert!(enml.contains("<b>Bold</b>&nbsp;<i>italic </i><u>under</u><s> strike</s>"));
	}

	#[test]
	fn keeps_spaces_between_adjacent_inline_elements() {
		use yrs::{ReadTxn, StateVector, Transact, XmlElementPrelim, XmlFragment, XmlTextPrelim};

		let doc = Doc::new();
		let content = doc.get_or_insert_xml_fragment("content");
		let mut txn = doc.transact_mut();
		let en_note = content.push_back(&mut txn, XmlElementPrelim::empty("en-note"));
		let div = en_note.push_back(&mut txn, XmlElementPrelim::empty("div"));
		let bold = div.push_back(&mut txn, XmlElementPrelim::empty("b"));
		bold.push_back(&mut txn, XmlTextPrelim::new("Bold"));
		let italic = div.push_back(&mut txn, XmlElementPrelim::empty("i"));
		italic.push_back(&mut txn, XmlTextPrelim::new("italic "));
		let underline = div.push_back(&mut txn, XmlElementPrelim::empty("u"));
		underline.push_back(&mut txn, XmlTextPrelim::new("under"));
		let strike = div.push_back(&mut txn, XmlElementPrelim::empty("s"));
		strike.push_back(&mut txn, XmlTextPrelim::new(" strike"));
		drop(txn);

		let update = doc
			.transact()
			.encode_state_as_update_v1(&StateVector::default());
		let enml = rte_doc_to_enml(&update).unwrap().unwrap();

		assert!(enml.contains("<b>Bold</b>&nbsp;<i>italic </i><u>under</u><s> strike</s>"));
	}

	#[test]
	fn keeps_rich_text_newlines_inside_formatted_runs() {
		use yrs::types::Attrs;
		use yrs::{
			Any, ReadTxn, StateVector, Text, Transact, XmlElementPrelim, XmlFragment, XmlTextPrelim,
		};

		let doc = Doc::new();
		let content = doc.get_or_insert_xml_fragment("content");
		let mut txn = doc.transact_mut();
		let en_note = content.push_back(&mut txn, XmlElementPrelim::empty("en-note"));
		let div = en_note.push_back(&mut txn, XmlElementPrelim::empty("div"));
		let text = div.push_back(&mut txn, XmlTextPrelim::new(""));
		text.insert(&mut txn, 0, "8 Days of Christmas");
		text.insert_with_attributes(
			&mut txn,
			19,
			"\n (2001), Destiny's Child announced a hiatus",
			Attrs::from([("sup".into(), Any::Bool(true))]),
		);
		drop(txn);

		let update = doc
			.transact()
			.encode_state_as_update_v1(&StateVector::default());
		let enml = rte_doc_to_enml(&update).unwrap().unwrap();

		assert!(enml.contains(
			"8 Days of Christmas<sup><br> (2001), Destiny's Child announced a hiatus</sup>"
		));
	}

	#[test]
	fn keeps_existing_spaces_between_inline_elements_minifier_safe() {
		let html = restore_adjacent_inline_spacing("<b>Bold</b> <i>italic</i>");

		assert_eq!(html, "<b>Bold</b>&nbsp;<i>italic</i>");
	}

	#[test]
	fn maps_rich_text_inline_code_marks() {
		use yrs::types::Attrs;
		use yrs::{
			Any, ReadTxn, StateVector, Text, Transact, XmlElementPrelim, XmlFragment, XmlTextPrelim,
		};

		let doc = Doc::new();
		let content = doc.get_or_insert_xml_fragment("content");
		let mut txn = doc.transact_mut();
		let en_note = content.push_back(&mut txn, XmlElementPrelim::empty("en-note"));
		let paragraph = en_note.push_back(&mut txn, XmlElementPrelim::empty("p"));
		let text = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
		text.insert_with_attributes(
			&mut txn,
			0,
			"inline_code",
			Attrs::from([("codespan".into(), Any::Bool(true))]),
		);
		drop(txn);

		let update = doc
			.transact()
			.encode_state_as_update_v1(&StateVector::default());
		let enml = rte_doc_to_enml(&update).unwrap().unwrap();

		assert!(enml.contains("<code>inline_code</code>"));
	}

	#[test]
	fn rich_text_enml_removes_metadata_and_keeps_source_and_resources() {
		let resources = vec![Resource {
			hash: "abc123".into(),
			file_name: "voice.mp3".into(),
			original_file_name: None,
			mime: "audio/mpeg".into(),
			s3_key: None,
			text_preview: None,
			archive_tree: None,
			size_bytes: None,
		}];
		let enml = rich_text_to_enml(
			r#"<en-note><div>Body</div><div>#postgres slug:body https://example.com/source</div></en-note>"#,
			Some("#postgres slug:body https://example.com/source"),
			Some("https://example.com/source"),
			&resources,
		);

		assert!(enml.contains(r#"<p><a href="https://example.com/source">"#));
		assert!(enml.contains("<div>Body</div>"));
		assert!(!enml.contains("slug:body"));
		assert!(enml.contains(r#"<en-media type="audio/mpeg" hash="abc123"/>"#));
	}

	fn api_test_note(guid: &str, title: &str, content: &str) -> edam_types::Note {
		edam_types::Note {
			guid: Some(guid.into()),
			title: Some(title.into()),
			content: Some(content.into()),
			content_hash: None,
			content_length: None,
			created: Some(1_700_000_000_000),
			updated: Some(1_700_000_100_000),
			deleted: None,
			active: Some(true),
			update_sequence_num: None,
			notebook_guid: Some("notebook-guid".into()),
			tag_guids: None,
			resources: None,
			attributes: None,
			tag_names: None,
			shared_notes: None,
			restrictions: None,
			limits: None,
		}
	}

	fn api_test_text_resource() -> edam_types::Resource {
		edam_types::Resource {
			guid: Some("resource-guid".into()),
			note_guid: Some("note-guid".into()),
			data: Some(edam_types::Data {
				body_hash: Some(vec![0xab, 0xcd]),
				size: Some(11),
				body: Some(b"hello world".to_vec()),
			}),
			mime: Some("text/plain".into()),
			width: None,
			height: None,
			duration: None,
			active: Some(true),
			recognition: None,
			attributes: Some(edam_types::ResourceAttributes {
				source_u_r_l: None,
				timestamp: None,
				latitude: None,
				longitude: None,
				altitude: None,
				camera_make: None,
				camera_model: None,
				client_will_index: None,
				reco_type: None,
				file_name: Some("readme.txt".into()),
				attachment: Some(true),
				application_data: None,
			}),
			update_sequence_num: None,
			alternate_data: None,
		}
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
				b"fontWeight\0inherit\0y Cached note body\0content\0en-note",
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
				base_domain: "everpublich.my".into(),
				cloudfront_url: Some("https://d111111abcdef8.cloudfront.net/".into()),
				evernote_service_token: None,
				evernote_user_store_url: TEST_USER_STORE_URL.into(),
				evernote_note_store_url: None,
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
				snippet text,
				source_URL text
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
				id, label, created, updated, deleted, parent_Notebook_id, snippet, source_URL
			)
			values (
				'note-1', 'Hello from cache', 1700000000000, 1700000001000, null,
				'notebook-1', '', null
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
