"""Todo código de erro emitido pelo serviço, verifier e offline tem mensagem
nos três idiomas da interface (lyraos-desktop#522)."""

import json
import pathlib
import re
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCES = ("core", "offline", "probe", "protocol", "service", "src-tauri", "verifier")
LOCALES = ("en-US", "pt-BR", "es-ES")
# Strings em maiúsculas que não são códigos de erro: variáveis de ambiente,
# chaves de arquivos de release, nomes de serde.
NOT_CODES = re.compile(
    r"^(CARGO_|LC_|LYRA_|XDG_|ZYPP_)|^(PKEXEC_UID|SCREAMING_SNAKE_CASE|USB_C)$"
)
CATALOG_JS = """
globalThis.window = { LYRA_UPGRADE_CATALOGS: { "en-US": {}, "pt-BR": {}, "es-ES": {} } };
require(process.argv[1]);
console.log(JSON.stringify(Object.fromEntries(Object.entries(window.LYRA_UPGRADE_CATALOGS)
  .map(([locale, catalog]) => [locale, Object.keys(catalog)]))));
"""


def emitted_codes():
    codes = set()
    for source in SOURCES:
        for path in (ROOT / source / "src").rglob("*.rs"):
            text = path.read_text(encoding="utf-8")
            text = text.split("#[cfg(test)]")[0]
            codes.update(re.findall(r'"([A-Z][A-Z0-9]+(?:_[A-Z0-9]+)+)"', text))
    return {code for code in codes if not NOT_CODES.search(code)}


class ErrorCatalogTests(unittest.TestCase):
    def test_every_emitted_code_has_a_message_in_every_locale(self):
        out = subprocess.run(
            ["node", "-e", CATALOG_JS, str(ROOT / "ui" / "errors.js")],
            check=True, capture_output=True, text=True,
        ).stdout
        catalogs = json.loads(out)
        codes = emitted_codes()
        self.assertIn("POST_BOOT_FAILED_UNITS", codes)
        for locale in LOCALES:
            keys = set(catalogs[locale])
            missing = sorted(code for code in codes if f"error_{code}" not in keys)
            self.assertEqual(missing, [], f"{locale} sem mensagem")
            self.assertIn("error_REPORTED", keys)


if __name__ == "__main__":
    unittest.main()
