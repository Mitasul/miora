"""Dependency-free static contract checks for embedded admin assets.
Run: python tests/ui/check_admin_assets.py [--node D:/node.exe]
This test inspects shipped assets; no API fixture data is installed in the UI.
"""
from __future__ import annotations
import argparse
import base64
from html.parser import HTMLParser
from pathlib import Path
import re
import shutil
import os
import subprocess
import sys
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
ASSETS = ROOT / 'assets' / 'admin'
EXPECTED = {
    'index.html',
    'login.html',
    'styles.css',
    'app.js',
    'theme.js',
    'login.js',
    'api.js',
    'icons.js',
    'plugin-manager.js',
    'usage-dashboard.js',
    'ui.js',
    'favicon.svg',
    'channel-editor.js',
    'models-editor.js',
    'parameters-editor.js',
    'request-logs.js',
    'workspace.js',
    'backend-logs.js',
    'model-testing.js',
    'channel-accounts.js',
    'channel-balances.js',
    'playground.js',
    'virtual-models.js',
    'key-manager.js',
    'log-settings.js',
}
NODE = shutil.which('node')


class Document(HTMLParser):
    def __init__(self, text):
        super().__init__(convert_charrefs=True)
        self.tags = []
        self.feed(text)
    def handle_starttag(self, tag, attrs):
        self.tags.append((tag, dict(attrs)))


class AdminAssetsTest(unittest.TestCase):
    def test_embed_inventory(self):
        self.assertEqual({p.name for p in ASSETS.iterdir() if p.is_file()}, EXPECTED)

    def test_html_local_resources_and_accessibility(self):
        for name in ['index.html', 'login.html']:
            text = (ASSETS / name).read_text(encoding='utf-8')
            parsed = Document(text)
            ids = [a['id'] for _, a in parsed.tags if 'id' in a]
            self.assertEqual(len(ids), len(set(ids)), f'{name}: duplicate IDs')
            self.assertIn(('html', {'lang': 'zh-CN'}), parsed.tags)
            self.assertIn('/admin/assets/favicon.svg', text)
            for tag, attrs in parsed.tags:
                self.assertNotIn('style', attrs, f'{name}: inline CSS violates CSP')
                self.assertFalse(any(key.startswith('on') for key in attrs), f'{name}: inline event handler')
                if tag == 'script':
                    if attrs.get('src') == '/admin/assets/theme.js':
                        self.assertNotIn('type', attrs)
                        self.assertNotIn('defer', attrs)
                        self.assertLess(text.index('src="/admin/assets/theme.js"'), text.index('href="/admin/assets/styles.css"'))
                    else:
                        self.assertEqual(attrs.get('type'), 'module')
                        self.assertIn(attrs.get('src'), ['/admin/assets/app.js', '/admin/assets/login.js'])
                if tag in {'script', 'link'}:
                    url = attrs.get('src', attrs.get('href', ''))
                    self.assertTrue(url.startswith('/admin/assets/'), f'{name}: non-local resource {url}')
                    self.assertTrue((ASSETS / url.rsplit('/', 1)[-1]).is_file())
                if tag == 'label' and 'for' in attrs:
                    self.assertIn(attrs['for'], ids)
                if tag == 'img':
                    self.assertIn(attrs.get('class'), {'brand-mark', 'gateway-logo'})
                    self.assertEqual(attrs.get('src'), '/admin/assets/favicon.svg')
                    self.assertEqual(attrs.get('alt'), '')
                    self.assertIn('width', attrs)
                    self.assertIn('height', attrs)

    def test_no_remote_runtime_or_emoji(self):
        for path in (entry for entry in ASSETS.iterdir() if entry.is_file()):
            text = path.read_text(encoding='utf-8')
            self.assertNotRegex(text, r'(?i)(?:src|href)=[\"\'](?:https?:)?//')
            self.assertNotRegex(text, r'(?i)@import|@font-face|\b(?:unpkg|jsdelivr|fonts\.googleapis)\b')
            self.assertFalse(any(0x1F000 <= ord(c) <= 0x1FAFF or 0x2600 <= ord(c) <= 0x27BF for c in text), f'{path.name}: emoji/pictograph')
            self.assertNotRegex(text, r'\b(?:eval|new Function)\s*\(')
            if path.name == 'theme.js':
                self.assertIn("const storageKey = 'miora.theme'", text)
                self.assertNotIn('sessionStorage', text)
            else:
                self.assertNotRegex(text, r'\b(?:localStorage|sessionStorage)\s*\.')

    def test_module_imports_and_syntax(self):
        for path in ASSETS.glob('*.js'):
            text = path.read_text(encoding='utf-8')
            for module in re.findall(r'\bfrom\s+[\"\']([^\"\']+)', text):
                self.assertTrue(module.startswith('./'), f'{path.name}: external import')
                self.assertTrue((ASSETS / module).is_file())
            if NODE:
                result = subprocess.run([NODE, '--input-type=module', '--check'], input=text.encode('utf-8'), capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
                self.assertEqual(result.returncode, 0, result.stderr.decode('utf-8', errors='replace'))

    def test_auth_contract(self):
        text = (ASSETS / 'api.js').read_text(encoding='utf-8')
        self.assertIn("const BASE = '/admin/api'", text)
        self.assertIn("credentials: 'same-origin'", text)
        self.assertIn("headers['X-CSRF-Token'] = csrfToken", text)
        self.assertIn("cache: 'no-store'", text)
        self.assertIn("window.location.replace('/login')", text)
        self.assertIn('AbortController', text)
        login = (ASSETS / 'login.js').read_text(encoding='utf-8')
        self.assertIn("window.location.replace('/admin#overview')", login)
        self.assertNotRegex(text + login, r'document\.cookie|Authorization:')

    def test_routes_and_no_placeholder_links(self):
        text = (ASSETS / 'index.html').read_text(encoding='utf-8')
        for route in ['overview', 'channels', 'plugins', 'interceptors', 'keys', 'logs', 'settings']:
            self.assertIn(f'href="#{route}"', text)
        for path in (entry for entry in ASSETS.iterdir() if entry.is_file()):
            self.assertNotRegex(path.read_text(encoding='utf-8'), r'href=[\"\'](?:#|javascript:[^\"\']*)[\"\']')

    def test_plugin_installation_has_no_configuration_step(self):
        text = (ASSETS / 'plugin-manager.js').read_text(encoding='utf-8')
        self.assertIn('type="file"', text)
        self.assertIn("api.upload('/plugins/upload', selected, { expected_revision:", text)
        self.assertNotRegex(text, r'schemaFields|readSchema|openPluginConfig|edit-plugin|service_token')
        self.assertNotRegex(text, r'<(?:textarea|select)\b|type="password"')
        app = (ASSETS / 'app.js').read_text(encoding='utf-8')
        self.assertIn('openPluginUpload', app)
        self.assertNotIn('function editPlugin(', app)

    def test_redundant_introductions_are_removed(self):
        for name in ['app.js', 'request-logs.js', 'log-settings.js']:
            text = (ASSETS / name).read_text(encoding='utf-8')
            self.assertNotIn('class="section-tag"', text)
        for name in ['index.html', 'login.html']:
            text = (ASSETS / name).read_text(encoding='utf-8')
            self.assertNotRegex(text, r'GATEWAY CONSOLE|WORKSPACE ACCESS|ONE GATEWAY|默认工作空间|自由掌控')

    def test_svg_icon_module(self):
        root = ET.fromstring((ASSETS / 'favicon.svg').read_text(encoding='utf-8'))
        self.assertTrue(root.tag.endswith('svg'))
        text = (ASSETS / 'icons.js').read_text(encoding='utf-8')
        self.assertIn('aria-hidden="true"', text)
        self.assertIn('focusable="false"', text)
        self.assertNotIn('<image', text)
        self.assertNotIn('<script', text)

    def test_brand_uses_repository_artwork(self):
        root = ET.fromstring((ASSETS / 'favicon.svg').read_text(encoding='utf-8'))
        self.assertEqual(root.get('data-source'), 'assets/branding/miora-logo.jpg')
        self.assertIsNone(root.get('data-source-sha256'))
        image = root.find('{http://www.w3.org/2000/svg}image')
        self.assertIsNotNone(image)
        prefix, encoded = image.get('href', '').split(',', 1)
        self.assertEqual(prefix, 'data:image/jpeg;base64')
        self.assertTrue(base64.b64decode(encoded, validate=True).startswith(b'\xff\xd8\xff'))
        for name in ['index.html', 'login.html']:
            parsed = Document((ASSETS / name).read_text(encoding='utf-8'))
            self.assertEqual(sum(tag == 'img' and attrs.get('class') == 'brand-mark' for tag, attrs in parsed.tags), 1)
            self.assertNotIn('data-icon="brand"', (ASSETS / name).read_text(encoding='utf-8'))
        for name in ['README.md', 'README_CN.md']:
            self.assertIn('src="assets/branding/miora-logo.jpg"', (ROOT / name).read_text(encoding='utf-8'))

    def test_ai_icon_inventory_and_embedding(self):
        directory = ASSETS / 'ai-icons'
        files = list(directory.glob('*.svg'))
        module = (ASSETS / 'icons.js').read_text(encoding='utf-8')
        routes = (ROOT / 'src' / 'api' / 'admin_ui.rs').read_text(encoding='utf-8')
        self.assertEqual(len(files), 114)
        self.assertEqual(set(re.findall(r"\['([^']+\.svg)'", module)), {path.name for path in files})
        for path in [*files, directory / 'LICENSE']:
            self.assertIn(f'"/admin/assets/ai-icons/{path.name}"', routes)
            self.assertIn(f'"/assets/admin/ai-icons/{path.name}"', routes)
        self.assertIn('MIT License', (directory / 'LICENSE').read_text(encoding='utf-8'))
        self.assertIn('@lobehub/icons-static-svg@1.95.1', (directory / 'SOURCE.md').read_text(encoding='utf-8'))

    def test_ai_icons_are_passive_local_svg(self):
        for path in (ASSETS / 'ai-icons').glob('*.svg'):
            root = ET.fromstring(path.read_text(encoding='utf-8'))
            self.assertEqual(root.tag, '{http://www.w3.org/2000/svg}svg')
            self.assertTrue(root.get('viewBox'), path.name)
            for element in root.iter():
                self.assertNotIn(element.tag.rsplit('}', 1)[-1].lower(), {'script', 'foreignobject', 'image'})
                for key, value in element.attrib.items():
                    attribute = key.rsplit('}', 1)[-1].lower()
                    self.assertFalse(attribute.startswith('on'), path.name)
                    if attribute == 'href':
                        self.assertTrue(value.startswith('#'), path.name)
            self.assertNotRegex(path.read_text(encoding='utf-8'), r'@import|https?://(?!www\.w3\.org/)')

    def test_ai_icon_theme_scope(self):
        css = (ASSETS / 'styles.css').read_text(encoding='utf-8')
        self.assertRegex(css, r':root\[data-theme="dark"\] \.ai-icon-monochrome\s*\{\s*filter: invert\(1\);')
        self.assertRegex(css, r'\.row-icon\.ai-brand-mark\s*\{\s*background: transparent;')
        self.assertNotRegex(css, r'\.ai-icon\s*\{[^}]*filter:')

    def test_restrained_geometry(self):
        text = (ASSETS / 'styles.css').read_text(encoding='utf-8')
        self.assertIn('--radius: 4px;', text)
        self.assertNotRegex(text, r'(?:linear|radial)-gradient\(')
        for radius in re.findall(r'border-radius:\s*(\d+)px;', text):
            self.assertLessEqual(int(radius), 4, 'Use restrained corners; circles may use 50%')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--node')
    args, rest = parser.parse_known_args()
    if args.node:
        NODE = args.node
    print(f'Assets: {ASSETS}\nJavaScript runtime: {NODE or "not found; syntax check skipped"}')
    unittest.main(argv=[sys.argv[0], *rest], verbosity=2)
