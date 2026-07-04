#!/usr/bin/env python3
"""Render a small README.md HTML excerpt for the landing page."""

import html
import pathlib
import sys


def render_markdown_subset(markdown: str) -> str:
	"""Render the README subset used by the landing page."""
	lines: list[str] = []
	in_code = False

	for raw in markdown.splitlines():
		line = raw.rstrip()
		if line.startswith('```'):
			if in_code:
				lines.append('</code></pre>')
				in_code = False
			else:
				lines.append('<pre><code>')
				in_code = True
			continue
		if in_code:
			lines.append(html.escape(line))
			continue
		if line.startswith('# '):
			lines.append(f'<h1>{html.escape(line[2:].strip())}</h1>')
		elif line.startswith('## '):
			lines.append(f'<h2>{html.escape(line[3:].strip())}</h2>')
		elif line.startswith('### '):
			lines.append(f'<h3>{html.escape(line[4:].strip())}</h3>')
		elif line.startswith('- '):
			lines.append(f'<p>• {html.escape(line[2:].strip())}</p>')
		elif line:
			lines.append(f'<p>{html.escape(line)}</p>')

	return '\n'.join(lines)


def main() -> int:
	"""CLI entry point."""
	readme = pathlib.Path(sys.argv[1])
	output = pathlib.Path(sys.argv[2])
	if not readme.exists():
		output.write_text('<p>README.md is not available yet.</p>', encoding='utf-8')
		return 0
	output.write_text(render_markdown_subset(readme.read_text(encoding='utf-8')), encoding='utf-8')
	return 0


if __name__ == '__main__':
	raise SystemExit(main())
