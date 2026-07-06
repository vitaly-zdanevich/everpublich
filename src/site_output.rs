//! Post-build helpers for generated public website output.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const BUILD_COMMENT_PREFIX: &str = "<!-- Everpublich build:";
const BUILD_COMMENT_SUFFIX: &str = "-->";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BuiltSiteAnnotation {
	/// UTC time written to the generated HTML comment.
	pub(crate) generated_at: DateTime<Utc>,
	/// Full generation time, including note conversion, link checks, Zola, and annotation.
	pub(crate) generation_duration_milliseconds: u64,
	/// Raw byte size of the generated public directory.
	pub(crate) total_size_bytes: u64,
	/// Byte size after applying the same Brotli policy used for S3 uploads.
	pub(crate) brotli_size_bytes: u64,
	/// Raw-to-Brotli percentage savings for the generated public directory.
	pub(crate) brotli_savings_percent: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SiteSize {
	total_bytes: u64,
	brotli_bytes: u64,
}

/// Convert a duration to saturated milliseconds for metrics/log formatting.
pub(crate) fn duration_milliseconds(duration: Duration) -> u64 {
	duration.as_millis().min(u128::from(u64::MAX)) as u64
}

/// Add a build metadata HTML comment to every generated HTML file and return
/// the final size metrics measured after the comment is present.
pub(crate) fn annotate_built_site(
	public_dir: &Path,
	generated_at: DateTime<Utc>,
	generation_duration: Duration,
) -> Result<BuiltSiteAnnotation> {
	let html_files = html_files(public_dir)?;
	let duration_ms = duration_milliseconds(generation_duration);
	let mut size = measure_site_size(public_dir)?;
	let mut comment = build_comment(generated_at, duration_ms, size);

	for _ in 0..6 {
		write_comment_to_html_files(&html_files, &comment)?;
		size = measure_site_size(public_dir)?;
		let next_comment = build_comment(generated_at, duration_ms, size);
		if next_comment == comment {
			return Ok(annotation(generated_at, duration_ms, size));
		}
		comment = next_comment;
	}

	write_comment_to_html_files(&html_files, &comment)?;
	size = measure_site_size(public_dir)?;
	Ok(annotation(generated_at, duration_ms, size))
}

fn annotation(
	generated_at: DateTime<Utc>,
	generation_duration_milliseconds: u64,
	size: SiteSize,
) -> BuiltSiteAnnotation {
	BuiltSiteAnnotation {
		generated_at,
		generation_duration_milliseconds,
		total_size_bytes: size.total_bytes,
		brotli_size_bytes: size.brotli_bytes,
		brotli_savings_percent: brotli_savings_percent(size),
	}
}

fn build_comment(generated_at: DateTime<Utc>, duration_ms: u64, size: SiteSize) -> String {
	format!(
		"\n{BUILD_COMMENT_PREFIX}\n  generated_at: {}\n  generation_time: {}\n  total_size: {}\n  brotli_size: {}\n  brotli_savings: {:.2}%\n{BUILD_COMMENT_SUFFIX}\n",
		generated_at.to_rfc3339_opts(SecondsFormat::Millis, true),
		format_duration_minutes_seconds(duration_ms),
		format_bytes(size.total_bytes),
		format_bytes(size.brotli_bytes),
		brotli_savings_percent(size)
	)
}

fn format_duration_minutes_seconds(duration_ms: u64) -> String {
	let total_seconds = duration_ms.div_ceil(1000);
	let minutes = total_seconds / 60;
	let seconds = total_seconds % 60;
	format!("{minutes}m{seconds:02}s")
}

fn format_bytes(bytes: u64) -> String {
	const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
	let mut value = bytes as f64;
	let mut unit = 0;
	while value >= 1024.0 && unit < UNITS.len() - 1 {
		value /= 1024.0;
		unit += 1;
	}
	if unit == 0 {
		return format!("{bytes} B");
	}
	if value >= 100.0 {
		format!("{value:.0} {}", UNITS[unit])
	} else if value >= 10.0 {
		format!("{value:.1} {}", UNITS[unit])
	} else {
		format!("{value:.2} {}", UNITS[unit])
	}
}

fn brotli_savings_percent(size: SiteSize) -> f64 {
	if size.total_bytes == 0 {
		return 0.0;
	}
	((size.total_bytes as f64 - size.brotli_bytes as f64) / size.total_bytes as f64) * 100.0
}

fn write_comment_to_html_files(html_files: &[PathBuf], comment: &str) -> Result<()> {
	for file in html_files {
		let html = fs::read_to_string(file)
			.with_context(|| format!("failed to read {}", file.display()))?;
		let annotated = insert_build_comment(&html, comment);
		fs::write(file, annotated)
			.with_context(|| format!("failed to write {}", file.display()))?;
	}
	Ok(())
}

fn insert_build_comment(html: &str, comment: &str) -> String {
	let mut html = remove_build_comments(html);
	if let Some(index) = html.rfind("</html>") {
		html.insert_str(index, comment);
	} else {
		if !html.ends_with('\n') {
			html.push('\n');
		}
		html.push_str(comment);
		html.push('\n');
	}
	html
}

fn remove_build_comments(html: &str) -> String {
	let mut clean = html.to_string();
	while let Some(start) = clean.find(BUILD_COMMENT_PREFIX) {
		let Some(end) = clean[start..].find(BUILD_COMMENT_SUFFIX) else {
			break;
		};
		clean.replace_range(start..start + end + BUILD_COMMENT_SUFFIX.len(), "");
	}
	clean
}

fn measure_site_size(public_dir: &Path) -> Result<SiteSize> {
	let mut size = SiteSize {
		total_bytes: 0,
		brotli_bytes: 0,
	};
	for file in public_files(public_dir)? {
		let metadata =
			fs::metadata(&file).with_context(|| format!("failed to stat {}", file.display()))?;
		let file_size = metadata.len();
		size.total_bytes = size.total_bytes.saturating_add(file_size);
		if is_brotli_uploaded_path(&file) {
			size.brotli_bytes = size.brotli_bytes.saturating_add(brotli_size(&file)?);
		} else {
			size.brotli_bytes = size.brotli_bytes.saturating_add(file_size);
		}
	}
	Ok(size)
}

fn brotli_size(file: &Path) -> Result<u64> {
	let output = Command::new("brotli")
		.arg("--stdout")
		.arg("--quality=11")
		.arg(file)
		.output()
		.with_context(|| {
			format!(
				"failed to execute brotli for {}; install the brotli package",
				file.display()
			)
		})?;
	if !output.status.success() {
		bail!(
			"brotli failed for {}\nstdout:\n{}\nstderr:\n{}",
			file.display(),
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
	}
	Ok(output.stdout.len() as u64)
}

fn html_files(public_dir: &Path) -> Result<Vec<PathBuf>> {
	let files = public_files(public_dir)?;
	Ok(files
		.into_iter()
		.filter(|path| {
			path.extension()
				.and_then(|extension| extension.to_str())
				.map(|extension| matches!(extension.to_ascii_lowercase().as_str(), "html" | "htm"))
				.unwrap_or(false)
		})
		.collect())
}

fn public_files(public_dir: &Path) -> Result<Vec<PathBuf>> {
	let mut files = Vec::new();
	collect_public_files(public_dir, &mut files)?;
	files.sort();
	Ok(files)
}

fn collect_public_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
	if !dir.exists() {
		return Ok(());
	}
	let mut entries = fs::read_dir(dir)
		.with_context(|| format!("failed to read {}", dir.display()))?
		.collect::<Result<Vec<_>, _>>()?;
	entries.sort_by_key(|entry| entry.path());
	for entry in entries {
		let path = entry.path();
		let file_type = entry
			.file_type()
			.with_context(|| format!("failed to read file type for {}", path.display()))?;
		if file_type.is_dir() {
			collect_public_files(&path, files)?;
		} else if file_type.is_file() {
			files.push(path);
		}
	}
	Ok(())
}

fn is_brotli_uploaded_path(path: &Path) -> bool {
	let Some(extension) = path
		.extension()
		.and_then(|extension| extension.to_str())
		.map(str::to_ascii_lowercase)
	else {
		return false;
	};
	matches!(
		extension.as_str(),
		"html"
			| "htm" | "css"
			| "js" | "mjs"
			| "json" | "webmanifest"
			| "xml" | "rss"
			| "atom" | "svg"
			| "txt" | "text"
			| "log" | "csv"
			| "tsv" | "wasm"
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::TimeZone;

	#[test]
	fn annotates_html_with_generation_and_size_metadata() {
		let temp = tempfile::tempdir().unwrap();
		let public = temp.path().join("public");
		fs::create_dir_all(public.join("posts/hello")).unwrap();
		fs::write(public.join("index.html"), "<!doctype html><html></html>").unwrap();
		fs::write(
			public.join("posts/hello/index.html"),
			"<!doctype html><html><body>Hello</body></html>",
		)
		.unwrap();
		fs::write(public.join("style.css"), "body { color: green; }\n").unwrap();
		fs::write(public.join("photo.jpg"), [1_u8, 2, 3, 4]).unwrap();

		let generated_at = Utc.with_ymd_and_hms(2026, 7, 6, 10, 30, 0).unwrap();
		let annotation =
			annotate_built_site(&public, generated_at, Duration::from_millis(1234)).unwrap();

		let index = fs::read_to_string(public.join("index.html")).unwrap();
		assert!(index.contains("<!-- Everpublich build:"));
		assert!(index.contains("\n  generated_at: 2026-07-06T10:30:00.000Z"));
		assert!(index.contains("\n  generation_time: 0m02s"));
		assert!(index.contains("\n  total_size: "));
		assert!(index.contains("\n  brotli_size: "));
		assert!(index.contains("\n  brotli_savings: "));
		assert_eq!(index.matches("<!-- Everpublich build:").count(), 1);
		assert_eq!(annotation.generation_duration_milliseconds, 1234);
		assert!(annotation.total_size_bytes >= annotation.brotli_size_bytes);

		annotate_built_site(&public, generated_at, Duration::from_millis(1234)).unwrap();
		let index = fs::read_to_string(public.join("index.html")).unwrap();
		assert_eq!(index.matches("<!-- Everpublich build:").count(), 1);
	}
}
