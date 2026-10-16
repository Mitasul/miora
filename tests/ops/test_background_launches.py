"""Check background launch arguments without starting any child process."""
import ast
from pathlib import Path
from types import SimpleNamespace
import unittest


ROOT = Path(__file__).resolve().parents[2]


class BackgroundLaunchTests(unittest.TestCase):
    def test_windows_children_are_hidden_and_posix_flags_are_zero(self):
        paths = sorted((ROOT / 'scripts').rglob('*.py')) + sorted((ROOT / 'tests').rglob('*.py'))
        launches = 0
        for path in paths:
            tree = ast.parse(path.read_text(encoding='utf-8'), filename=str(path))
            for node in ast.walk(tree):
                if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                        and isinstance(node.func.value, ast.Name)
                        and node.func.value.id == 'subprocess'
                        and node.func.attr in ('Popen', 'run', 'check_call', 'check_output')):
                    continue
                launches += 1
                with self.subTest(path=str(path.relative_to(ROOT)), line=node.lineno):
                    flags = next((keyword.value for keyword in node.keywords
                                  if keyword.arg == 'creationflags'), None)
                    self.assertIsNotNone(flags, 'Child launch must explicitly hide the Windows console')
                    expression = compile(ast.Expression(body=flags), str(path), 'eval')
                    for platform, expected in (('nt', 0x08000000), ('posix', 0)):
                        actual = eval(expression, {
                            '__builtins__': {},
                            'os': SimpleNamespace(name=platform),
                            'subprocess': SimpleNamespace(CREATE_NO_WINDOW=0x08000000),
                        })
                        self.assertEqual(actual, expected)
        self.assertGreater(launches, 0)


if __name__ == '__main__':
    unittest.main()
