//! HTTP application used by the AWS Lambda Function URL.
//!
//! The app intentionally returns static HTML for the landing/admin pages and a
//! small JSON API. Production persistence is provided by DynamoDB in Terraform;
//! the core route behavior stays testable here without AWS credentials.

use crate::admin::{ConnectRequest, ConnectResponse, initial_settings};
use crate::auth;
use crate::models::UserItem;
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Runtime configuration loaded from Lambda environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
	/// Public root domain used for per-user subdomains.
	pub base_domain: String,
	/// Secret for admin-token HMAC.
	pub admin_secret: String,
	/// Evernote OAuth consumer key.
	pub evernote_consumer_key: Option<String>,
	/// Support email shown in the admin panel.
	pub support_email: String,
	/// Support Telegram URL.
	pub support_telegram: String,
	/// GitHub issue tracker URL.
	pub support_tickets: String,
}

impl AppConfig {
	/// Load Lambda app config from environment variables.
	pub fn from_env() -> Self {
		Self {
			base_domain: std::env::var("EVERPUBLICH_BASE_DOMAIN")
				.unwrap_or_else(|_| "everpublich.example".to_string()),
			admin_secret: std::env::var("EVERPUBLICH_ADMIN_SECRET")
				.unwrap_or_else(|_| "development-admin-secret-change-me".to_string()),
			evernote_consumer_key: std::env::var("EVERNOTE_CONSUMER_KEY").ok(),
			support_email: std::env::var("SUPPORT_EMAIL")
				.unwrap_or_else(|_| "zdanevich.vitaly@ya.ru".to_string()),
			support_telegram: std::env::var("SUPPORT_TELEGRAM")
				.unwrap_or_else(|_| "https://t.me/vitaly_zdanevich".to_string()),
			support_tickets: std::env::var("SUPPORT_TICKETS").unwrap_or_else(|_| {
				"https://github.com/vitaly-zdanevich/everpublich/issues".to_string()
			}),
		}
	}
}

/// Minimal HTTP response for the Lambda adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppResponse {
	/// HTTP status code.
	pub status: u16,
	/// HTTP content type.
	pub content_type: &'static str,
	/// Response body.
	pub body: String,
}

impl AppResponse {
	fn html(body: impl Into<String>) -> Self {
		Self {
			status: 200,
			content_type: "text/html; charset=utf-8",
			body: body.into(),
		}
	}

	fn json<T: Serialize>(value: &T) -> Result<Self> {
		Ok(Self {
			status: 200,
			content_type: "application/json; charset=utf-8",
			body: serde_json::to_string(value).context("failed to serialize JSON response")?,
		})
	}

	fn error(status: u16, message: impl Into<String>) -> Self {
		Self {
			status,
			content_type: "application/json; charset=utf-8",
			body: serde_json::json!({ "error": message.into() }).to_string(),
		}
	}
}

/// Route one HTTP request.
pub fn route(method: &str, path: &str, body: &str, cfg: &AppConfig) -> Result<AppResponse> {
	match (method, path) {
		("GET", "/") | ("GET", "/index.html") => Ok(AppResponse::html(landing_html(cfg))),
		("GET", "/admin") | ("GET", "/admin.html") => Ok(AppResponse::html(admin_html(cfg))),
		("GET", "/pricing") | ("GET", "/pricing.html") => Ok(AppResponse::html(pricing_html())),
		("GET", "/app.css") => Ok(AppResponse {
			status: 200,
			content_type: "text/css; charset=utf-8",
			body: include_str!("../web/app.css").to_string(),
		}),
		("GET", "/landing.js") => Ok(AppResponse {
			status: 200,
			content_type: "text/javascript; charset=utf-8",
			body: include_str!("../web/landing.js").to_string(),
		}),
		("GET", "/admin.js") => Ok(AppResponse {
			status: 200,
			content_type: "text/javascript; charset=utf-8",
			body: include_str!("../web/admin.js").to_string(),
		}),
		("POST", "/api/connect") => connect(body, cfg),
		("POST", "/api/remove-account") => Ok(AppResponse::json(&serde_json::json!({
			"status": "queued_for_removal"
		}))?),
		("POST", "/api/build-all") => Ok(AppResponse::json(&serde_json::json!({
			"status": "queued",
			"mode": "full_regeneration"
		}))?),
		_ => Ok(AppResponse::error(404, "not found")),
	}
}

fn connect(body: &str, cfg: &AppConfig) -> Result<AppResponse> {
	let req = serde_json::from_str::<ConnectRequest>(body).context("invalid connect request")?;
	let settings = initial_settings(&req.site_name, &cfg.base_domain)?;
	let user_id = deterministic_user_id(&settings.site_name);
	let fake_evernote_token = format!("pending-oauth-{user_id}");
	let admin_token = auth::session_token(&user_id, &fake_evernote_token, &cfg.admin_secret)?;
	let user = UserItem::new(user_id.clone(), settings);
	let response = ConnectResponse {
		admin_token,
		user_id,
		website_url: user.settings.base_url,
		message: "Check after a few minutes while notes download and the website builds.".into(),
	};
	AppResponse::json(&response)
}

fn deterministic_user_id(site_name: &str) -> String {
	let digest = Sha256::digest(site_name.as_bytes());
	hex_bytes(&digest[..8])
}

fn hex_bytes(bytes: &[u8]) -> String {
	bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn landing_html(cfg: &AppConfig) -> String {
	include_str!("../web/index.html")
		.replace("__API_BASE_URL__", "")
		.replace("__BASE_DOMAIN__", &cfg.base_domain)
		.replace("__README_HTML__", include_str!("../web/readme-embed.html"))
}

fn admin_html(cfg: &AppConfig) -> String {
	include_str!("../web/admin.html")
		.replace("__API_BASE_URL__", "")
		.replace("__SUPPORT_EMAIL__", &cfg.support_email)
		.replace("__SUPPORT_TELEGRAM__", &cfg.support_telegram)
		.replace("__SUPPORT_TICKETS__", &cfg.support_tickets)
}

fn pricing_html() -> &'static str {
	include_str!("../web/pricing.html")
}

/// Convert internal errors to API responses at the edge.
pub fn route_or_error(method: &str, path: &str, body: &str, cfg: &AppConfig) -> AppResponse {
	match route(method, path, body, cfg) {
		Ok(response) => response,
		Err(error) => AppResponse::error(400, error.to_string()),
	}
}

/// Parse a Lambda request body that should be UTF-8 JSON.
pub fn body_to_str(bytes: &[u8]) -> Result<&str> {
	std::str::from_utf8(bytes).map_err(|error| anyhow!("request body is not UTF-8: {error}"))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn cfg() -> AppConfig {
		AppConfig {
			base_domain: "everpublich.example".into(),
			admin_secret: "secret".into(),
			evernote_consumer_key: None,
			support_email: "support@example.com".into(),
			support_telegram: "https://t.me/support".into(),
			support_tickets: "https://github.com/example/issues".into(),
		}
	}

	#[test]
	fn landing_contains_connect_button() {
		let response = route("GET", "/", "", &cfg()).unwrap();

		assert_eq!(response.status, 200);
		assert!(!response.body.contains("__API_BASE_URL__"));
		assert!(response.body.contains("data-api-base-url=''"));
		assert!(
			response
				.body
				.contains("Connect Evernote notebook read-only to make a website from it")
		);
	}

	#[test]
	fn admin_html_uses_same_origin_api_and_support_links() {
		let response = route("GET", "/admin.html", "", &cfg()).unwrap();

		assert_eq!(response.status, 200);
		assert!(!response.body.contains("__API_BASE_URL__"));
		assert!(response.body.contains("data-api-base-url=''"));
		assert!(response.body.contains("mailto:support@example.com"));
		assert!(response.body.contains("https://t.me/support"));
	}

	#[test]
	fn connect_returns_admin_token_and_url() {
		let response = route(
			"POST",
			"/api/connect",
			r#"{"site_name":"My Notebook"}"#,
			&cfg(),
		)
		.unwrap();
		let json: serde_json::Value = serde_json::from_str(&response.body).unwrap();

		assert_eq!(
			json["website_url"],
			"https://my-notebook.everpublich.example/"
		);
		assert!(json["admin_token"].as_str().unwrap().starts_with("adm1."));
	}
}
