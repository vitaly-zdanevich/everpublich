//! Scheduled per-user builder Lambda entry point.

use lambda_runtime::{Error, LambdaEvent, service_fn};
use serde_json::{Value, json};

#[tokio::main]
async fn main() -> Result<(), Error> {
	lambda_runtime::run(service_fn(handle)).await
}

async fn handle(event: LambdaEvent<Value>) -> Result<Value, Error> {
	let user_id = std::env::var("EVERPUBLICH_USER_ID").unwrap_or_else(|_| "unknown".to_string());
	let request_id = event.context.request_id;

	Ok(json!({
		"status": "accepted",
		"mode": "full_regeneration",
		"user_id": user_id,
		"request_id": request_id
	}))
}
