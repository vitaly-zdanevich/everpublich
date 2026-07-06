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
			'Your address will be https://&lt;your-notebook-name>.everpublich.my',
			index,
		)
		self.assertIn('If you stop sharing the notebook', index)
		self.assertIn('Plans, if we have users', index)
		self.assertIn('Internal Evernote links inside the shared notebook', index)
		self.assertIn('SEO-friendly output with minimal JavaScript', index)
		self.assertIn('share@everpublich.my', index)
		self.assertIn('Custom domains', index)
		self.assertNotIn('or on your GitHub', index)
		self.assertIn('Sync to WordPress', index)
		self.assertIn('Sync to a Telegram channel', index)
		self.assertIn('Different languages', index)
		self.assertIn('Custom styles and CSS', index)
		self.assertIn('Send your ideas', index)
		self.assertIn('https://t.me/vitaly_zdanevich', index)
		self.assertIn('Support, ideas, suggestions', index)
		self.assertIn('I have used Evernote since 2009 and love it', index)
		self.assertNotIn('\n\t', index)
		self.assertIn('<link href="app.css" rel="stylesheet">', index)
		self.assertIn('prefers-color-scheme', css)
		self.assertIn('::selection', css)
		self.assertIn('background-color:#292', css)
		self.assertIn('.feature-grid+h2{margin-top:48px}', css)
		self.assertIn('min-height:100vh', css)
		self.assertIn('main{', css)
		self.assertIn('width:min(1120px,100% - 28px)', css)
		self.assertIn('flex-direction:column', css)
		self.assertIn('flex:1 0 auto', css)
		self.assertIn('.hero-copy{width:100%}', css)
		self.assertIn('.lead{width:100%', css)
		self.assertIn('border-radius:6px;width:100%;margin-top:28px', css)
		self.assertIn('.landing-support{color:var(--muted);margin-top:auto;padding:8px 0 36px}', css)
		self.assertNotIn('.hero-copy{max-width:780px}', css)
		self.assertNotIn('.lead{max-width:700px', css)
		self.assertNotIn('border-bottom:1px solid var(--line)', css)
		self.assertNotIn('min-height:calc(100vh', css)
		self.assertNotIn('\n', css)


if __name__ == '__main__':
	unittest.main()
