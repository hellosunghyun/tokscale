"""Pure synthetic validator tests; never starts Aside or reads an account."""
import json
from pathlib import Path
import unittest

scope = {'__name__': 'helper_validator_test'}
source = Path(__file__).with_name('aside.py')
exec(compile(source.read_text(), str(source), 'exec'), scope)
validate = scope['safe_diagnostic']
parse = scope['parse_diagnostic_output']

class SafeDiagnostics(unittest.TestCase):
    def value(self):
        return {'http_status': 503, 'phase': 'walk', 'successful_pages': 2, 'elapsed_ms': 1250}

    def test_exact_allowed_fields(self):
        for status in (None, 200, 401, 403, 429, 500, 503, 599):
            value = self.value(); value['http_status'] = status
            self.assertEqual(validate(value), value)

    def test_private_unknown_missing_and_invalid_fields_are_discarded(self):
        for key in ('url', 'query', 'cursor', 'account', 'body', 'token'):
            value = self.value(); value[key] = 'private-canary'
            self.assertIsNone(validate(value))
        for key in self.value():
            value = self.value(); del value[key]
            self.assertIsNone(validate(value))
        for key, bad in [('http_status', True), ('http_status', '503 private-canary'), ('http_status', 99), ('http_status', 600), ('phase', 'private-canary'), ('successful_pages', -1), ('successful_pages', 513), ('successful_pages', True), ('elapsed_ms', -1), ('elapsed_ms', 3600001), ('elapsed_ms', 1.5)]:
            value = self.value(); value[key] = bad
            self.assertIsNone(validate(value))

    def test_only_one_exact_console_record_is_accepted(self):
        line = b'GROK_DIAGNOSTIC:' + json.dumps(self.value()).encode() + b'\r\n'
        self.assertEqual(parse(line), self.value())
        self.assertIsNone(parse(b'echo ' + line))
        self.assertIsNone(parse(line + line))
        lf=line.replace(b'\r\n',b'\n');self.assertIsNone(parse(lf+lf))
        self.assertIsNone(parse(b'GROK_DIAGNOSTIC:{private-canary}\n'))

if __name__ == '__main__':
    unittest.main()
