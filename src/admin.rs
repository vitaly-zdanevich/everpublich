//! Admin-panel request/response models and settings updates.

use crate::models::{GithubVisibility, IndexMode, SearchMode, SiteSettings, UserItem};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// Registration request sent by the landing page before Evernote OAuth starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectRequest {
	/// Desired website and GitHub repository name.
	pub site_name: String,
}

/// Response returned after OAuth callback or MVP mock registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectResponse {
	/// Browser-stored admin token. It is not the raw Evernote OAuth token.
	pub admin_token: String,
	/// Generated user ID.
	pub user_id: String,
	/// Public site URL. The first build can take a few minutes.
	pub website_url: String,
	/// Human-readable status for the spinner screen.
	pub message: String,
}

/// Settings editable from the per-user admin panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UpdateSettingsRequest {
	/// Rename the website; this changes title and subdomain.
	pub site_name: Option<String>,
	/// Home page rendering preference.
	pub index_mode: Option<IndexMode>,
	/// Site search backend.
	pub search_mode: Option<SearchMode>,
	/// GitHub backup repository visibility.
	pub github_visibility: Option<GithubVisibility>,
	/// Optional Zola theme name.
	pub zola_theme: Option<String>,
	/// Optional custom CSS appended to the built-in theme.
	pub custom_css: Option<String>,
	/// Optional Google Analytics measurement ID.
	pub google_analytics_id: Option<String>,
	/// Optional Yandex Metrica counter ID.
	pub yandex_metrica_id: Option<String>,
	/// Enable or disable widget expansion.
	pub expand_widgets: Option<bool>,
}

/// Apply an admin update to a stored user item.
pub fn apply_settings_update(
	user: &mut UserItem,
	update: UpdateSettingsRequest,
	base_domain: &str,
) -> Result<()> {
	if let Some(site_name) = update.site_name {
		let trimmed = site_name.trim();
		if trimmed.len() < 3 {
			bail!("site name must be at least 3 characters");
		}
		user.settings.rename(trimmed, base_domain);
	}
	if let Some(index_mode) = update.index_mode {
		user.settings.index_mode = index_mode;
	}
	if let Some(search_mode) = update.search_mode {
		user.settings.search_mode = search_mode;
	}
	if let Some(github_visibility) = update.github_visibility {
		user.settings.github_visibility = github_visibility;
	}
	if let Some(zola_theme) = clean_optional(update.zola_theme) {
		user.settings.zola_theme = Some(zola_theme);
	}
	if let Some(custom_css) = update.custom_css {
		user.settings.custom_css = Some(custom_css);
	}
	if let Some(google_analytics_id) = clean_optional(update.google_analytics_id) {
		user.settings.google_analytics_id = Some(google_analytics_id);
	}
	if let Some(yandex_metrica_id) = clean_optional(update.yandex_metrica_id) {
		user.settings.yandex_metrica_id = Some(yandex_metrica_id);
	}
	if let Some(expand_widgets) = update.expand_widgets {
		user.settings.expand_widgets = expand_widgets;
	}
	Ok(())
}

fn clean_optional(value: Option<String>) -> Option<String> {
	value
		.map(|s| s.trim().to_string())
		.filter(|s| !s.is_empty())
}

/// Create initial settings from the landing form.
pub fn initial_settings(site_name: &str, base_domain: &str) -> Result<SiteSettings> {
	let trimmed = site_name.trim();
	if trimmed.len() < 3 {
		bail!("website name must be at least 3 characters");
	}
	Ok(SiteSettings::new(trimmed, base_domain))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::UserItem;

	#[test]
	fn applies_rename_and_preferences() {
		let settings = initial_settings("Old Site", "everpublich.example").unwrap();
		let mut user = UserItem::new("u1", settings);
		apply_settings_update(
			&mut user,
			UpdateSettingsRequest {
				site_name: Some("New Site".into()),
				index_mode: Some(IndexMode::TitlesOnly),
				search_mode: Some(SearchMode::Google),
				github_visibility: Some(GithubVisibility::Public),
				..UpdateSettingsRequest::default()
			},
			"everpublich.example",
		)
		.unwrap();

		assert_eq!(user.settings.subdomain, "new-site");
		assert_eq!(user.settings.index_mode, IndexMode::TitlesOnly);
		assert_eq!(user.settings.search_mode, SearchMode::Google);
		assert_eq!(user.settings.github_visibility, GithubVisibility::Public);
	}
}
