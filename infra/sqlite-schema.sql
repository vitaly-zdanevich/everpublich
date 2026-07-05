-- SQLite schema for the single-VM MVP.
--
-- The official Evernote client cache is treated as an external read-only source.
-- This database stores Everpublich users, site preferences, and build metadata.

pragma foreign_keys = on;

create table if not exists users (
	user_id text primary key,
	site_slug text not null unique,
	site_title text not null,
	registration_date_utc text not null,
	evernote_account_label text,
	shared_notebook_guid text,
	shared_notebook_name text,
	home_page_mode text not null default 'full_posts' check (home_page_mode in ('full_posts', 'titles_only')),
	public_base_url text,
	admin_token_hash text,
	github_repository_full_name text,
	github_repository_visibility text check (github_repository_visibility in ('private', 'public')),
	github_token_ciphertext text,
	removed_at_utc text,
	created_at_utc text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
	updated_at_utc text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

create table if not exists site_settings (
	user_id text primary key references users(user_id) on delete cascade,
	google_analytics_id text,
	yandex_metrica_id text,
	custom_css text,
	expand_widgets integer not null default 1 check (expand_widgets in (0, 1)),
	static_search_enabled integer not null default 1 check (static_search_enabled in (0, 1)),
	google_search_enabled integer not null default 0 check (google_search_enabled in (0, 1)),
	offline_support_enabled integer not null default 1 check (offline_support_enabled in (0, 1)),
	updated_at_utc text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

create table if not exists note_index (
	user_id text not null references users(user_id) on delete cascade,
	note_guid text not null,
	title text not null,
	slug text not null,
	updated_at_utc text,
	is_page integer not null default 0 check (is_page in (0, 1)),
	is_podcast integer not null default 0 check (is_podcast in (0, 1)),
	primary key (user_id, note_guid)
);

create table if not exists build_runs (
	build_id integer primary key autoincrement,
	user_id text references users(user_id) on delete set null,
	started_at_utc text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
	finished_at_utc text,
	status text not null check (status in ('running', 'succeeded', 'failed')),
	notes_seen integer not null default 0,
	error_message text
);

create index if not exists idx_users_site_slug on users(site_slug);
create index if not exists idx_build_runs_user_started on build_runs(user_id, started_at_utc desc);
