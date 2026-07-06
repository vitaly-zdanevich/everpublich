//! SQLite row contract.
//!
//! The single-VM MVP stores users and preferences in SQLite. This module keeps
//! the SQL-facing shape small and testable without coupling the core crate to a
//! concrete SQLite client yet.

use crate::models::{GithubVisibility, IndexMode, UserItem};
use chrono::{DateTime, Utc};

/// Name of the SQLite table that stores SaaS users.
pub const USERS_TABLE: &str = "users";

/// SQL-friendly representation of a row in the `users` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteUserRow {
	/// Stable service user ID.
	pub user_id: String,
	/// Per-user subdomain and generated-site directory name.
	pub site_slug: String,
	/// Public site title rendered by Zola.
	pub site_title: String,
	/// Registration timestamp stored in UTC.
	pub registration_date_utc: String,
	/// Shared Evernote notebook GUID, when known.
	pub shared_notebook_guid: Option<String>,
	/// Shared Evernote notebook name, used before GUID resolution.
	pub shared_notebook_name: Option<String>,
	/// Home-page rendering preference stored by SQLite.
	pub home_page_mode: SqliteHomePageMode,
	/// Public URL of the generated website.
	pub public_base_url: String,
	/// GitHub backup repository visibility, when configured.
	pub github_repository_visibility: Option<SqliteGithubVisibility>,
}

impl SqliteUserRow {
	/// Convert the domain model into the row shape used by SQLite inserts.
	pub fn from_user(user: &UserItem) -> Self {
		Self {
			user_id: user.user_id.clone(),
			site_slug: user.settings.subdomain.clone(),
			site_title: user.settings.title.clone(),
			registration_date_utc: to_sqlite_utc(user.registration_date),
			shared_notebook_guid: user.settings.notebook_guid.clone(),
			shared_notebook_name: user.settings.notebook_name.clone(),
			home_page_mode: SqliteHomePageMode::from(user.settings.index_mode),
			public_base_url: user.settings.base_url.clone(),
			github_repository_visibility: Some(SqliteGithubVisibility::from(
				user.settings.github_visibility,
			)),
		}
	}
}

/// Home-page mode values accepted by `infra/sqlite-schema.sql`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqliteHomePageMode {
	/// Store full post bodies on the home page.
	FullPosts,
	/// Store titles only on the home page.
	TitlesOnly,
}

impl SqliteHomePageMode {
	/// Return the string value stored in SQLite.
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::FullPosts => "full_posts",
			Self::TitlesOnly => "titles_only",
		}
	}
}

impl From<IndexMode> for SqliteHomePageMode {
	fn from(value: IndexMode) -> Self {
		match value {
			IndexMode::FullPosts => Self::FullPosts,
			IndexMode::TitlesOnly => Self::TitlesOnly,
		}
	}
}

/// GitHub visibility values accepted by `infra/sqlite-schema.sql`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqliteGithubVisibility {
	/// Public GitHub repository.
	Public,
	/// Private GitHub repository.
	Private,
}

impl SqliteGithubVisibility {
	/// Return the string value stored in SQLite.
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Public => "public",
			Self::Private => "private",
		}
	}
}

impl From<GithubVisibility> for SqliteGithubVisibility {
	fn from(value: GithubVisibility) -> Self {
		match value {
			GithubVisibility::Public => Self::Public,
			GithubVisibility::Private => Self::Private,
		}
	}
}

fn to_sqlite_utc(value: DateTime<Utc>) -> String {
	value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::SiteSettings;

	#[test]
	fn user_row_matches_sqlite_schema_values() {
		let mut settings = SiteSettings::new("Site", "everpublich.my");
		settings.notebook_guid = Some("notebook-guid".into());
		settings.notebook_name = Some("Public Notebook".into());

		let user = UserItem::new("42", settings);
		let row = SqliteUserRow::from_user(&user);

		assert_eq!(USERS_TABLE, "users");
		assert_eq!(row.user_id, "42");
		assert_eq!(row.site_slug, "site");
		assert_eq!(row.site_title, "Site");
		assert_eq!(row.shared_notebook_guid.as_deref(), Some("notebook-guid"));
		assert_eq!(row.shared_notebook_name.as_deref(), Some("Public Notebook"));
		assert_eq!(row.home_page_mode.as_str(), "full_posts");
		assert_eq!(
			row.github_repository_visibility.unwrap().as_str(),
			"private"
		);
		assert!(row.registration_date_utc.ends_with('Z'));
	}
}
