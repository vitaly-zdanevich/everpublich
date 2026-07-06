//! Data structures stored in SQLite and passed through the build pipeline.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Stored secret encrypted by [`crate::crypto::TokenCipher`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedSecret {
	/// Encryption algorithm and version marker.
	pub algorithm: String,
	/// Base64-encoded encrypted token payload.
	pub ciphertext: String,
	/// Timestamp when this encrypted value was created.
	pub created_at: DateTime<Utc>,
}

impl EncryptedSecret {
	/// Create an encrypted-secret record for storage.
	pub fn new(algorithm: impl Into<String>, ciphertext: impl Into<String>) -> Self {
		Self {
			algorithm: algorithm.into(),
			ciphertext: ciphertext.into(),
			created_at: Utc::now(),
		}
	}
}

/// How a user connected Evernote content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvernoteAccessMode {
	/// Legacy path where a user authorized an Evernote App Notebook or Full Access OAuth token.
	UserOauth,
	/// The user shared a notebook read-only to the service Evernote account.
	SharedToServiceAccount,
}

/// Whether the generated GitHub backup repository should be public or private.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubVisibility {
	/// Public GitHub repository.
	Public,
	/// Private GitHub repository.
	#[default]
	Private,
}

/// How the blog index renders posts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexMode {
	/// Render full post bodies on the home page.
	#[default]
	FullPosts,
	/// Render only titles on the home page.
	TitlesOnly,
}

/// Search mode exposed in generated Zola config.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
	/// Use Zola's generated static search index.
	#[default]
	ZolaStatic,
	/// Render a Google site-search form.
	Google,
	/// Disable search UI.
	None,
}

/// User-controlled website settings stored for a generated site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteSettings {
	/// Website and optional GitHub repository name chosen by the user.
	pub site_name: String,
	/// Public site title rendered by Zola.
	pub title: String,
	/// Per-user subdomain under the configured service domain.
	pub subdomain: String,
	/// Fully qualified website URL.
	pub base_url: String,
	/// Selected Evernote notebook GUID, when known.
	pub notebook_guid: Option<String>,
	/// Selected Evernote notebook name, used before GUID resolution.
	pub notebook_name: Option<String>,
	/// Home page rendering preference.
	pub index_mode: IndexMode,
	/// Search implementation preference.
	pub search_mode: SearchMode,
	/// Optional Zola theme name.
	pub zola_theme: Option<String>,
	/// Optional CSS appended to the generated theme.
	pub custom_css: Option<String>,
	/// Optional Google Analytics measurement ID.
	pub google_analytics_id: Option<String>,
	/// Optional Yandex Metrica counter ID.
	pub yandex_metrica_id: Option<String>,
	/// GitHub backup repository visibility.
	pub github_visibility: GithubVisibility,
	/// Whether supported bare links should become widgets.
	pub expand_widgets: bool,
}

impl SiteSettings {
	/// Create default settings for a newly registered site.
	pub fn new(site_name: impl Into<String>, base_domain: &str) -> Self {
		let site_name = site_name.into();
		let subdomain = crate::slug::slugify(&site_name);
		let base = base_domain.trim_end_matches('.');
		Self {
			title: site_name.clone(),
			site_name,
			subdomain: subdomain.clone(),
			base_url: format!("https://{subdomain}.{base}/"),
			notebook_guid: None,
			notebook_name: None,
			index_mode: IndexMode::default(),
			search_mode: SearchMode::default(),
			zola_theme: None,
			custom_css: None,
			google_analytics_id: None,
			yandex_metrica_id: None,
			github_visibility: GithubVisibility::default(),
			expand_widgets: true,
		}
	}

	/// Renaming changes both the visible title and the subdomain.
	pub fn rename(&mut self, new_site_name: &str, base_domain: &str) {
		self.site_name = new_site_name.to_string();
		self.title = new_site_name.to_string();
		self.subdomain = crate::slug::slugify(new_site_name);
		self.base_url = format!(
			"https://{}.{} /",
			self.subdomain,
			base_domain.trim_end_matches('.')
		)
		.replace(" /", "/");
	}
}

/// One SaaS user stored in SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserItem {
	/// Stable service user ID.
	pub user_id: String,
	/// Registration timestamp stored for product and billing decisions.
	pub registration_date: DateTime<Utc>,
	/// Evernote account ID returned by OAuth or inferred from cache data, when available.
	pub evernote_user_id: Option<String>,
	/// Evernote access path used for this account.
	pub evernote_access_mode: EvernoteAccessMode,
	/// Legacy encrypted Evernote OAuth token.
	pub evernote_token: Option<EncryptedSecret>,
	/// Encrypted GitHub OAuth token.
	pub github_token: Option<EncryptedSecret>,
	/// User-controlled website settings.
	pub settings: SiteSettings,
	/// Latest build state shown in the admin panel.
	pub build: BuildState,
	/// Soft-delete timestamp.
	pub deleted_at: Option<DateTime<Utc>>,
}

impl UserItem {
	/// Create a new SQLite-backed user item.
	pub fn new(user_id: impl Into<String>, settings: SiteSettings) -> Self {
		Self {
			user_id: user_id.into(),
			registration_date: Utc::now(),
			evernote_user_id: None,
			evernote_access_mode: EvernoteAccessMode::SharedToServiceAccount,
			evernote_token: None,
			github_token: None,
			settings,
			build: BuildState::default(),
			deleted_at: None,
		}
	}
}

/// Build status visible in the admin panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildState {
	/// Timestamp when the latest build started.
	pub last_started_at: Option<DateTime<Utc>>,
	/// Timestamp when the latest build finished.
	pub last_finished_at: Option<DateTime<Utc>>,
	/// Latest build status.
	pub last_status: BuildStatus,
	/// Optional human-readable build message.
	pub last_message: Option<String>,
}

impl Default for BuildState {
	fn default() -> Self {
		Self {
			last_started_at: None,
			last_finished_at: None,
			last_status: BuildStatus::NeverBuilt,
			last_message: None,
		}
	}
}

/// Status of the latest site build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStatus {
	/// The site has not been built yet.
	NeverBuilt,
	/// A build has been queued.
	Queued,
	/// A build is currently running.
	Running,
	/// The latest build succeeded.
	Succeeded,
	/// The latest build failed.
	Failed,
}

/// Simplified Evernote note representation used by the renderer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
	/// Evernote note GUID.
	pub guid: String,
	/// Evernote note title.
	pub title: String,
	/// Note creation timestamp.
	pub created: DateTime<Utc>,
	/// Note update timestamp.
	pub updated: DateTime<Utc>,
	/// Evernote tag names.
	pub tags: Vec<String>,
	/// ENML note body.
	pub enml: String,
	/// Evernote resources referenced by the note body.
	pub resources: Vec<Resource>,
}

/// Evernote resource metadata. The binary itself should be copied into the
/// generated site or a temporary build directory; Markdown references use `file_name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
	/// Evernote body hash used by `<en-media>`.
	pub hash: String,
	/// File name written into the Zola page bundle.
	pub file_name: String,
	/// Optional original file name when `file_name` points to a generated web preview.
	#[serde(default)]
	pub original_file_name: Option<String>,
	/// Resource MIME type.
	pub mime: String,
	/// Optional remote object key after upload or mirroring.
	pub s3_key: Option<String>,
	/// Optional human-readable preview for text-like attachments.
	#[serde(default)]
	pub text_preview: Option<String>,
	/// Optional archive listing rendered as a closed tree.
	#[serde(default)]
	pub archive_tree: Option<String>,
}

/// A post/page ready to write into Zola content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
	/// Source Evernote note GUID.
	pub guid: String,
	/// Zola content slug.
	pub slug: String,
	/// Rendered post/page title.
	pub title: String,
	/// Publication date.
	pub date: DateTime<Utc>,
	/// Public tags.
	pub tags: Vec<String>,
	/// Zola body HTML/Markdown.
	pub body: String,
	/// Media and attachment resources referenced by this post.
	pub resources: Vec<Resource>,
	/// Whether this content is a blog post or page.
	pub kind: PostKind,
}

/// Zola content type derived from Evernote tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostKind {
	/// Regular blog post.
	BlogPost,
	/// Dedicated static page.
	Page,
	/// About page.
	About,
	/// Metadata note that adds a tag archive link to the top navigation.
	NavTag,
	/// Notebook-level configuration note, not rendered as public content.
	Config,
}

impl PostKind {
	/// Whether this note should get a public URL and internal Evernote links.
	pub fn is_linkable(self) -> bool {
		matches!(self, Self::BlogPost | Self::Page | Self::About)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_to_full_posts_and_static_search() {
		let s = SiteSettings::new("My Evernote Site", "everpublich.example");

		assert_eq!(s.index_mode, IndexMode::FullPosts);
		assert_eq!(s.search_mode, SearchMode::ZolaStatic);
		assert_eq!(s.github_visibility, GithubVisibility::Private);
		assert_eq!(s.subdomain, "my-evernote-site");
	}

	#[test]
	fn rename_updates_subdomain_and_title() {
		let mut s = SiteSettings::new("Old Name", "everpublich.example");
		s.rename("New Name", "everpublich.example");

		assert_eq!(s.title, "New Name");
		assert_eq!(s.subdomain, "new-name");
		assert_eq!(s.base_url, "https://new-name.everpublich.example/");
	}
}
