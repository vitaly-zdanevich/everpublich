import pathlib
import subprocess
import sys
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
OUTPUT = ROOT / 'dist' / 'pages'


class BuildPagesTest(unittest.TestCase):
	def test_build_pages_minifies_html_and_uses_minified_css(self) -> None:
		subprocess.run(
			[sys.executable, str(ROOT / 'scripts' / 'build_pages.py')],
			check=True,
			cwd=ROOT,
		)

		index = (OUTPUT / 'index.html').read_text(encoding='utf-8')
		css = (OUTPUT / 'app.css').read_text(encoding='utf-8')

		self.assertIn(
			'<title>Everpublich: Evernote notebook to a static website</title>',
			index,
		)
		self.assertIn(
			'Your address will be https://d2ieo3xczytvos.cloudfront.net/&lt;your-notebook-name>',
			index,
		)
		self.assertIn('If you stop sharing the notebook', index)
		self.assertIn('Plans, if we have users', index)
		self.assertIn('Nice subdomain address and custom domains', index)
		self.assertIn('Sync to WordPress', index)
		self.assertIn('https://t.me/vitaly_zdanevich', index)
		self.assertIn('Support, ideas, suggestions', index)
		self.assertNotIn('\n\t', index)
		self.assertIn('<link href="app.css" rel="stylesheet">', index)
		self.assertIn('prefers-color-scheme', css)
		self.assertIn('::selection', css)
		self.assertIn('background-color:#292', css)
		self.assertNotIn('border-bottom:1px solid var(--line)', css)
		self.assertNotIn('min-height:calc(100vh', css)
		self.assertNotIn('\n', css)


if __name__ == '__main__':
	unittest.main()
