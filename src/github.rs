//! GitHub backup configuration.

use crate::models::GithubVisibility;

/// OAuth scopes requested only when the user enables GitHub backup.
pub fn oauth_scopes(visibility: GithubVisibility) -> &'static [&'static str] {
	match visibility {
		GithubVisibility::Public => &["public_repo"],
		GithubVisibility::Private => &["repo"],
	}
}

/// User-facing copy shown before a repository visibility change.
pub fn backup_warning(visibility: GithubVisibility) -> &'static str {
	match visibility {
		GithubVisibility::Public => {
			"Public GitHub backup is useful because git stores every version, but public history can preserve private mistakes. If you accidentally publish something private, you must also repair git history."
		}
		GithubVisibility::Private => {
			"Private GitHub backup is safer for drafts and personal notes. Git still stores every version, so accidental private content may require history cleanup."
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn public_repo_uses_narrower_scope() {
		assert_eq!(oauth_scopes(GithubVisibility::Public), ["public_repo"]);
		assert_eq!(oauth_scopes(GithubVisibility::Private), ["repo"]);
	}
}
