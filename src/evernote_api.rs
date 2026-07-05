//! Live Evernote API probes.
//!
//! This module is intentionally small and read-only. It exists to validate the
//! service-account plan where users share notebooks to one Everpublich Evernote
//! account. The current VM MVP reads the official client cache, while this module
//! remains useful for accounts that still have a developer token.

use anyhow::{Context, Result, anyhow};
use evernote_edam::note_store::{
	NoteFilter, NoteStoreSyncClient, NotesMetadataResultSpec, TNoteStoreSyncClient,
};
use evernote_edam::types::{self, LinkedNotebook, SharedNotebookPrivilegeLevel};
use evernote_edam::user_store::{
	EDAM_VERSION_MAJOR, EDAM_VERSION_MINOR, TUserStoreSyncClient, UserStoreSyncClient,
};
use reqwest::blocking::Client as ReqwestClient;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thrift::protocol::{TBinaryInputProtocol, TBinaryOutputProtocol};
use thrift::transport::{ReadHalf, TIoChannel, WriteHalf};

/// Evernote UserStore endpoint for production accounts.
pub const DEFAULT_USER_STORE_URL: &str = "https://www.evernote.com/edam/user";

const CLIENT_NAME: &str = "everpublich/0.1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

type InputProtocol<C> = TBinaryInputProtocol<ReadHalf<ThriftHttpChannel<C>>>;
type OutputProtocol<C> = TBinaryOutputProtocol<WriteHalf<ThriftHttpChannel<C>>>;

/// Minimal HTTP abstraction for the Evernote Thrift transport.
pub trait ThriftHttpClient: Clone + Send + Sync + 'static {
	/// POST a binary Thrift request and return the binary Thrift response.
	fn post_thrift(&self, url: &str, body: Vec<u8>) -> Result<Vec<u8>, String>;
}

#[derive(Clone)]
/// Blocking reqwest-backed implementation used by CLI probes.
pub struct ReqwestThriftHttpClient {
	client: ReqwestClient,
}

impl ReqwestThriftHttpClient {
	/// Build an HTTP client with a stable user agent and timeout.
	pub fn new() -> Result<Self> {
		let client = ReqwestClient::builder()
			.user_agent(CLIENT_NAME)
			.timeout(REQUEST_TIMEOUT)
			.pool_max_idle_per_host(2)
			.build()
			.context("failed to build Evernote HTTP client")?;
		Ok(Self { client })
	}
}

impl ThriftHttpClient for ReqwestThriftHttpClient {
	fn post_thrift(&self, url: &str, body: Vec<u8>) -> Result<Vec<u8>, String> {
		let response = self
			.client
			.post(url)
			.header(reqwest::header::CONTENT_TYPE, "application/x-thrift")
			.body(body)
			.send()
			.map_err(|error| format!("Evernote request failed: {error}"))?
			.error_for_status()
			.map_err(|error| format!("Evernote returned an HTTP error: {error}"))?;

		response
			.bytes()
			.map(|bytes| bytes.to_vec())
			.map_err(|error| format!("failed to read Evernote response: {error}"))
	}
}

#[derive(Clone)]
/// Read-only Evernote API client for service-account experiments.
pub struct EvernoteApiClient<C = ReqwestThriftHttpClient>
where
	C: ThriftHttpClient,
{
	token: String,
	user_store_url: String,
	note_store_url: Arc<Mutex<Option<String>>>,
	http: C,
}

impl EvernoteApiClient<ReqwestThriftHttpClient> {
	/// Create a production HTTP client. The token is never logged by this type.
	pub fn new(
		token: impl Into<String>,
		user_store_url: Option<String>,
		note_store_url: Option<String>,
	) -> Result<Self> {
		Ok(Self::with_http_client(
			token,
			user_store_url.unwrap_or_else(|| DEFAULT_USER_STORE_URL.to_string()),
			note_store_url,
			ReqwestThriftHttpClient::new()?,
		))
	}
}

impl<C> EvernoteApiClient<C>
where
	C: ThriftHttpClient,
{
	/// Create a client with an injected HTTP transport for tests.
	pub fn with_http_client(
		token: impl Into<String>,
		user_store_url: impl Into<String>,
		note_store_url: Option<String>,
		http: C,
	) -> Self {
		Self {
			token: token.into(),
			user_store_url: user_store_url.into(),
			note_store_url: Arc::new(Mutex::new(note_store_url)),
			http,
		}
	}

	/// List notebooks shared to the token owner and verify note metadata access.
	pub fn linked_notebook_summaries(
		&self,
		max_sample_notes: i32,
	) -> Result<Vec<LinkedNotebookSummary>> {
		self.linked_notebook_probes(max_sample_notes)?
			.into_iter()
			.map(|probe| match probe {
				LinkedNotebookProbe::Accessible(summary) => Ok(summary),
				LinkedNotebookProbe::Failed(failure) => Err(anyhow!(failure.error)),
			})
			.collect()
	}

	/// List notebooks shared to the token owner and keep per-notebook failures.
	pub fn linked_notebook_probes(
		&self,
		max_sample_notes: i32,
	) -> Result<Vec<LinkedNotebookProbe>> {
		let max_sample_notes = max_sample_notes.clamp(0, 50);
		let mut account_note_store = self.note_store_client()?;
		let linked_notebooks = account_note_store
			.list_linked_notebooks(self.token.clone())
			.map_err(|error| {
				anyhow!("Evernote API error while listing linked notebooks: {error}")
			})?;

		Ok(linked_notebooks
			.into_iter()
			.map(
				|linked| match self.linked_notebook_summary(linked.clone(), max_sample_notes) {
					Ok(summary) => LinkedNotebookProbe::Accessible(summary),
					Err(error) => LinkedNotebookProbe::Failed(LinkedNotebookFailure::from_linked(
						linked,
						error.to_string(),
					)),
				},
			)
			.collect())
	}

	fn linked_notebook_summary(
		&self,
		linked: LinkedNotebook,
		max_sample_notes: i32,
	) -> Result<LinkedNotebookSummary> {
		let note_store_url = required_non_empty(
			linked.note_store_url.clone(),
			"linked notebook did not include a NoteStore URL",
		)?;
		let candidates = shared_notebook_key_candidates(&linked);
		if candidates.is_empty() {
			return Err(anyhow!(
				"linked notebook did not include a shared notebook global ID or URI"
			));
		}

		let mut authenticated =
			self.authenticate_to_linked_notebook(&note_store_url, &candidates)?;
		let notebook_guid = required_non_empty(
			authenticated.shared.notebook_guid.clone(),
			"shared notebook did not include a notebook GUID",
		)?;

		let note_list = authenticated
			.note_store
			.find_notes_metadata(
				authenticated.token,
				NoteFilter {
					notebook_guid: Some(notebook_guid.clone()),
					..NoteFilter::default()
				},
				0,
				max_sample_notes,
				note_metadata_result_spec(),
			)
			.map_err(|error| anyhow!("Evernote API error while reading note metadata: {error}"))?;

		Ok(LinkedNotebookSummary {
			share_name: linked.share_name,
			owner_username: linked.username,
			notebook_guid,
			privilege: authenticated.shared.privilege.map(privilege_name),
			notebook_modifiable: authenticated.shared.notebook_modifiable.unwrap_or(false),
			total_notes: note_list.total_notes,
			sample_notes: note_list.notes.into_iter().map(NoteSummary::from).collect(),
		})
	}

	fn authenticate_to_linked_notebook(
		&self,
		note_store_url: &str,
		candidates: &[String],
	) -> Result<AuthenticatedSharedNotebook<C>> {
		let mut errors = Vec::new();
		for candidate in candidates {
			let mut shared_note_store = self.note_store_client_at(note_store_url.to_string())?;
			let auth = match shared_note_store
				.authenticate_to_shared_notebook(candidate.clone(), self.token.clone())
			{
				Ok(auth) => auth,
				Err(error) => {
					errors.push(format!("{candidate}: {error}"));
					continue;
				}
			};
			let shared_token = auth.authentication_token;
			let shared = match shared_note_store.get_shared_notebook_by_auth(shared_token.clone()) {
				Ok(shared) => shared,
				Err(error) => {
					errors.push(format!("{candidate}: {error}"));
					continue;
				}
			};
			return Ok(AuthenticatedSharedNotebook {
				note_store: shared_note_store,
				token: shared_token,
				shared,
			});
		}

		Err(anyhow!(
			"Evernote API error while authenticating to shared notebook with {} candidate(s): {}",
			candidates.len(),
			errors.join("; ")
		))
	}

	fn user_store_client(
		&self,
	) -> Result<UserStoreSyncClient<InputProtocol<C>, OutputProtocol<C>>> {
		let channel = ThriftHttpChannel::new(self.user_store_url.clone(), self.http.clone());
		let (read, write) = channel.split().map_err(|error| {
			anyhow!("failed to initialize Evernote UserStore transport: {error}")
		})?;
		Ok(UserStoreSyncClient::new(
			TBinaryInputProtocol::new(read, true),
			TBinaryOutputProtocol::new(write, true),
		))
	}

	fn note_store_client(
		&self,
	) -> Result<NoteStoreSyncClient<InputProtocol<C>, OutputProtocol<C>>> {
		let note_store_url = self.note_store_url()?;
		self.note_store_client_at(note_store_url)
	}

	fn note_store_client_at(
		&self,
		note_store_url: String,
	) -> Result<NoteStoreSyncClient<InputProtocol<C>, OutputProtocol<C>>> {
		let channel = ThriftHttpChannel::new(note_store_url, self.http.clone());
		let (read, write) = channel.split().map_err(|error| {
			anyhow!("failed to initialize Evernote NoteStore transport: {error}")
		})?;
		Ok(NoteStoreSyncClient::new(
			TBinaryInputProtocol::new(read, true),
			TBinaryOutputProtocol::new(write, true),
		))
	}

	fn note_store_url(&self) -> Result<String> {
		if let Some(url) = self
			.note_store_url
			.lock()
			.map_err(|_| anyhow!("Evernote NoteStore URL cache is poisoned"))?
			.clone()
		{
			return Ok(url);
		}

		let mut client = self.user_store_client()?;
		let version_ok = client
			.check_version(
				CLIENT_NAME.to_string(),
				EDAM_VERSION_MAJOR,
				EDAM_VERSION_MINOR,
			)
			.map_err(|error| anyhow!("Evernote UserStore API error: {error}"))?;
		if !version_ok {
			return Err(anyhow!("Evernote EDAM protocol version is not supported"));
		}

		let urls = client
			.get_user_urls(self.token.clone())
			.map_err(|error| anyhow!("Evernote UserStore API error: {error}"))?;
		let note_store_url = required_non_empty(
			urls.note_store_url,
			"Evernote did not return a NoteStore URL",
		)?;

		*self
			.note_store_url
			.lock()
			.map_err(|_| anyhow!("Evernote NoteStore URL cache is poisoned"))? =
			Some(note_store_url.clone());
		Ok(note_store_url)
	}
}

struct AuthenticatedSharedNotebook<C>
where
	C: ThriftHttpClient,
{
	note_store: NoteStoreSyncClient<InputProtocol<C>, OutputProtocol<C>>,
	token: String,
	shared: types::SharedNotebook,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Readable summary proving that a linked notebook is accessible.
pub struct LinkedNotebookSummary {
	/// Display name of the shared notebook in the recipient account.
	pub share_name: Option<String>,
	/// Evernote username of the notebook owner, when returned.
	pub owner_username: Option<String>,
	/// Owner-side notebook GUID returned after shared-notebook auth.
	pub notebook_guid: String,
	/// Human-readable shared-notebook privilege.
	pub privilege: Option<String>,
	/// Whether Evernote says the notebook is modifiable for this token.
	pub notebook_modifiable: bool,
	/// Number of notes Evernote reports in the shared notebook.
	pub total_notes: i32,
	/// First notes returned by metadata search.
	pub sample_notes: Vec<NoteSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Per-linked-notebook probe result.
pub enum LinkedNotebookProbe {
	/// Evernote accepted shared-notebook auth and returned note metadata.
	Accessible(LinkedNotebookSummary),
	/// The linked notebook was listed, but validation failed.
	Failed(LinkedNotebookFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Diagnostic data for a linked notebook that could not be authenticated.
pub struct LinkedNotebookFailure {
	/// Display name of the shared notebook in the recipient account.
	pub share_name: Option<String>,
	/// Evernote username of the notebook owner, when returned.
	pub owner_username: Option<String>,
	/// Whether Evernote returned the private shared-notebook global ID.
	pub has_shared_notebook_global_id: bool,
	/// Whether Evernote returned a public notebook URI.
	pub has_uri: bool,
	/// Whether Evernote returned a NoteStore URL for the owner shard.
	pub has_note_store_url: bool,
	/// Human-readable failure from the attempted shared-notebook calls.
	pub error: String,
}

impl LinkedNotebookFailure {
	fn from_linked(linked: LinkedNotebook, error: String) -> Self {
		Self {
			share_name: linked.share_name,
			owner_username: linked.username,
			has_shared_notebook_global_id: linked
				.shared_notebook_global_id
				.as_deref()
				.is_some_and(|value| !value.trim().is_empty()),
			has_uri: linked
				.uri
				.as_deref()
				.is_some_and(|value| !value.trim().is_empty()),
			has_note_store_url: linked
				.note_store_url
				.as_deref()
				.is_some_and(|value| !value.trim().is_empty()),
			error,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Minimal note metadata used by the shared-notebook probe.
pub struct NoteSummary {
	/// Evernote note GUID.
	pub guid: String,
	/// Note title, when requested and returned.
	pub title: Option<String>,
	/// Creation timestamp in Evernote milliseconds.
	pub created: Option<i64>,
	/// Update timestamp in Evernote milliseconds.
	pub updated: Option<i64>,
	/// Largest resource MIME type, if the note has resources.
	pub largest_resource_mime: Option<String>,
	/// Largest resource size in bytes, if the note has resources.
	pub largest_resource_size: Option<i32>,
}

impl From<evernote_edam::note_store::NoteMetadata> for NoteSummary {
	fn from(note: evernote_edam::note_store::NoteMetadata) -> Self {
		Self {
			guid: note.guid,
			title: note.title,
			created: note.created,
			updated: note.updated,
			largest_resource_mime: note.largest_resource_mime,
			largest_resource_size: note.largest_resource_size,
		}
	}
}

fn note_metadata_result_spec() -> NotesMetadataResultSpec {
	NotesMetadataResultSpec {
		include_title: Some(true),
		include_created: Some(true),
		include_updated: Some(true),
		include_notebook_guid: Some(true),
		include_tag_guids: Some(true),
		include_largest_resource_mime: Some(true),
		include_largest_resource_size: Some(true),
		..NotesMetadataResultSpec::default()
	}
}

fn required_non_empty(value: Option<String>, context: &str) -> Result<String> {
	value
		.filter(|value| !value.trim().is_empty())
		.ok_or_else(|| anyhow!(context.to_string()))
}

fn shared_notebook_key_candidates(linked: &LinkedNotebook) -> Vec<String> {
	let mut candidates = Vec::new();
	for value in [
		linked.shared_notebook_global_id.as_deref(),
		linked.uri.as_deref(),
	]
	.into_iter()
	.flatten()
	{
		let value = value.trim();
		if !value.is_empty() && !candidates.iter().any(|candidate| candidate == value) {
			candidates.push(value.to_string());
		}
	}
	candidates
}

fn privilege_name(privilege: SharedNotebookPrivilegeLevel) -> String {
	match privilege {
		SharedNotebookPrivilegeLevel::READ_NOTEBOOK => "READ_NOTEBOOK".to_string(),
		SharedNotebookPrivilegeLevel::READ_NOTEBOOK_PLUS_ACTIVITY => {
			"READ_NOTEBOOK_PLUS_ACTIVITY".to_string()
		}
		SharedNotebookPrivilegeLevel::MODIFY_NOTEBOOK_PLUS_ACTIVITY => {
			"MODIFY_NOTEBOOK_PLUS_ACTIVITY".to_string()
		}
		SharedNotebookPrivilegeLevel::GROUP => "GROUP".to_string(),
		SharedNotebookPrivilegeLevel::FULL_ACCESS => "FULL_ACCESS".to_string(),
		SharedNotebookPrivilegeLevel::BUSINESS_FULL_ACCESS => "BUSINESS_FULL_ACCESS".to_string(),
		other => format!("unknown({})", i32::from(other)),
	}
}

#[derive(Clone)]
struct ThriftHttpChannel<C>
where
	C: ThriftHttpClient,
{
	endpoint: String,
	http: C,
	state: Arc<Mutex<ThriftHttpState>>,
}

#[derive(Default)]
struct ThriftHttpState {
	read_bytes: Vec<u8>,
	read_pos: usize,
	write_bytes: Vec<u8>,
}

impl<C> ThriftHttpChannel<C>
where
	C: ThriftHttpClient,
{
	fn new(endpoint: String, http: C) -> Self {
		Self {
			endpoint,
			http,
			state: Arc::new(Mutex::new(ThriftHttpState::default())),
		}
	}
}

impl<C> TIoChannel for ThriftHttpChannel<C>
where
	C: ThriftHttpClient,
{
	fn split(self) -> thrift::Result<(ReadHalf<Self>, WriteHalf<Self>)>
	where
		Self: Sized,
	{
		Ok((ReadHalf::new(self.clone()), WriteHalf::new(self)))
	}
}

impl<C> Read for ThriftHttpChannel<C>
where
	C: ThriftHttpClient,
{
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		let mut state = self
			.state
			.lock()
			.map_err(|_| io::Error::other("Evernote transport state is poisoned"))?;
		let remaining = state.read_bytes.len().saturating_sub(state.read_pos);
		let read_len = remaining.min(buf.len());
		if read_len == 0 {
			return Ok(0);
		}

		let start = state.read_pos;
		let end = start + read_len;
		buf[..read_len].copy_from_slice(&state.read_bytes[start..end]);
		state.read_pos = end;
		Ok(read_len)
	}
}

impl<C> Write for ThriftHttpChannel<C>
where
	C: ThriftHttpClient,
{
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		let mut state = self
			.state
			.lock()
			.map_err(|_| io::Error::other("Evernote transport state is poisoned"))?;
		state.write_bytes.extend_from_slice(buf);
		Ok(buf.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		let request_body = {
			let mut state = self
				.state
				.lock()
				.map_err(|_| io::Error::other("Evernote transport state is poisoned"))?;
			std::mem::take(&mut state.write_bytes)
		};

		let response_body = self
			.http
			.post_thrift(&self.endpoint, request_body)
			.map_err(io::Error::other)?;
		let mut state = self
			.state
			.lock()
			.map_err(|_| io::Error::other("Evernote transport state is poisoned"))?;
		state.read_bytes = response_body;
		state.read_pos = 0;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn shared_notebook_key_prefers_global_id() {
		let linked = LinkedNotebook {
			shared_notebook_global_id: Some("global".into()),
			uri: Some("public-uri".into()),
			..LinkedNotebook::default()
		};

		assert_eq!(
			shared_notebook_key_candidates(&linked),
			vec!["global".to_string(), "public-uri".to_string()]
		);
	}

	#[test]
	fn shared_notebook_key_falls_back_to_uri() {
		let linked = LinkedNotebook {
			shared_notebook_global_id: Some(" ".into()),
			uri: Some("public-uri".into()),
			..LinkedNotebook::default()
		};

		assert_eq!(
			shared_notebook_key_candidates(&linked),
			vec!["public-uri".to_string()]
		);
	}

	#[test]
	fn privilege_names_are_readable() {
		assert_eq!(
			privilege_name(SharedNotebookPrivilegeLevel::READ_NOTEBOOK),
			"READ_NOTEBOOK"
		);
		assert_eq!(
			privilege_name(SharedNotebookPrivilegeLevel(999)),
			"unknown(999)"
		);
	}
}
