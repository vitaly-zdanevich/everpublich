#!/usr/bin/env python3
"""Build GitHub Pages static assets from web templates."""

import os
import pathlib
import re
import shutil
import subprocess


ROOT = pathlib.Path(__file__).resolve().parents[1]
WEB = ROOT / 'web'
OUTPUT = ROOT / 'dist' / 'pages'

DEFAULT_API_BASE_URL = ''
HTML_TEMPLATES = ('index.html', 'admin.html', 'pricing.html')
MINIFIED_ASSETS = {
	'admin.js': 'admin.min.js',
	'app.css': 'app.min.css',
}


def env(name: str, default: str) -> str:
	"""Read a Pages build setting from the environment."""
	return os.environ.get(name) or default


def replacements() -> dict[str, str]:
	"""Return template replacements for Pages-hosted HTML."""
	return {
		'__API_BASE_URL__': env('EVERPUBLICH_PAGES_API_BASE_URL', DEFAULT_API_BASE_URL),
		'__SUPPORT_EMAIL__': env('SUPPORT_EMAIL', 'zdanevich.vitaly@ya.ru'),
		'__SUPPORT_TELEGRAM__': env('SUPPORT_TELEGRAM', 'https://t.me/vitaly_zdanevich'),
		'__SUPPORT_TICKETS__': env(
			'SUPPORT_TICKETS',
			'https://github.com/vitaly-zdanevich/everpublich/issues',
		),
	}


def render_template(name: str, values: dict[str, str]) -> None:
	"""Render one HTML template into the Pages output directory."""
	html = (WEB / name).read_text(encoding='utf-8')
	for placeholder, value in values.items():
		html = html.replace(placeholder, value)
	(OUTPUT / name).write_text(html, encoding='utf-8')


def copy_asset(name: str) -> None:
	"""Copy a static asset without template replacement."""
	source = MINIFIED_ASSETS.get(name, name)
	shutil.copy2(WEB / source, OUTPUT / name)


def minify_html(names: tuple[str, ...]) -> None:
	"""Minify rendered HTML with the project npm tooling."""
	subprocess.run(
		['node', str(ROOT / 'scripts' / 'minify-html.mjs')]
		+ [str(OUTPUT / name) for name in names],
		check=True,
	)


def validate_output() -> None:
	"""Fail the build if an HTML template placeholder leaked into Pages output."""
	placeholder = re.compile(r'__[A-Z0-9_]+__')
	for path in OUTPUT.glob('*.html'):
		match = placeholder.search(path.read_text(encoding='utf-8'))
		if match:
			raise ValueError(f'{path} still contains template placeholder {match.group(0)}')


def main() -> int:
	"""CLI entry point."""
	if OUTPUT.exists():
		shutil.rmtree(OUTPUT)
	OUTPUT.mkdir(parents=True)

	values = replacements()
	for name in HTML_TEMPLATES:
		render_template(name, values)
	minify_html(HTML_TEMPLATES)
	for name in ('app.css', 'admin.js', 'favicon.svg'):
		copy_asset(name)

	(OUTPUT / '.nojekyll').write_text('', encoding='utf-8')
	validate_output()
	return 0


if __name__ == '__main__':
	raise SystemExit(main())
