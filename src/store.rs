//! DynamoDB item contract.
//!
//! The production Lambda should map this JSON contract to DynamoDB attribute
//! values. Keeping the contract here lets tests validate keys and serialized
//! shape without pulling the AWS SDK into the core crate.

use crate::models::UserItem;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Partition-key prefix for user items.
pub const USER_PK_PREFIX: &str = "USER#";
/// Sort-key value for the single user profile item.
pub const USER_SK: &str = "PROFILE";

/// JSON-friendly representation of the DynamoDB user item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamoUserRecord {
	/// DynamoDB partition key.
	pub pk: String,
	/// DynamoDB sort key.
	pub sk: String,
	/// Serialized user payload.
	pub user: UserItem,
}

impl DynamoUserRecord {
	/// Wrap a user item with DynamoDB keys.
	pub fn from_user(user: UserItem) -> Self {
		Self {
			pk: user_pk(&user.user_id),
			sk: USER_SK.to_string(),
			user,
		}
	}

	/// Serialize this record as JSON.
	pub fn to_json(&self) -> Result<String> {
		serde_json::to_string(self).context("failed to serialize DynamoDB user record")
	}
}

/// Build the partition key for a user ID.
pub fn user_pk(user_id: &str) -> String {
	format!("{USER_PK_PREFIX}{user_id}")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::{SiteSettings, UserItem};

	#[test]
	fn user_record_has_single_item_keys() {
		let user = UserItem::new("42", SiteSettings::new("Site", "everpublich.example"));
		let record = DynamoUserRecord::from_user(user);

		assert_eq!(record.pk, "USER#42");
		assert_eq!(record.sk, "PROFILE");
		assert!(record.to_json().unwrap().contains("\"registration_date\""));
	}
}
