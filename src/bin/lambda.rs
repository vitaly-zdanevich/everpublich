//! AWS Lambda Function URL entry point for Everpublich.

use lambda_http::{Body, Request, Response, service_fn};

use everpublich::lambda_app::{AppConfig, route_or_error};

#[tokio::main]
async fn main() -> Result<(), lambda_http::Error> {
	let cfg = AppConfig::from_env();
	lambda_http::run(service_fn(move |request: Request| {
		let cfg = cfg.clone();
		async move { Ok::<_, lambda_http::Error>(handle(request, &cfg)) }
	}))
	.await
}

fn handle(request: Request, cfg: &AppConfig) -> Response<Body> {
	let method = request.method().as_str().to_string();
	let path = request.uri().path().to_string();
	let body = match request.body() {
		Body::Text(text) => text.as_str(),
		Body::Binary(bytes) => std::str::from_utf8(bytes).unwrap_or(""),
		Body::Empty => "",
		_ => "",
	};
	let response = route_or_error(&method, &path, body, cfg);

	Response::builder()
		.status(response.status)
		.header("content-type", response.content_type)
		.body(Body::Text(response.body))
		.expect("valid Lambda HTTP response")
}
