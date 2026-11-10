"""python scripts/publish_artifacts_test.py — the Publish workflow's script, without a server."""
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import MagicMock, patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('publish', Path(__file__).parent / 'publish-artifacts.py')
publish = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publish)

ENV = {'FTP_HOST': 'example.test', 'FTP_USERNAME': 'test', 'FTP_PASSWORD': 'test-only', 'FTP_DIRECTORY': '/www/downloads/UB/'}


class PublishTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)

    def file(self, rel):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b'artifact')

    def packages(self):
        for p in publish.PLATFORMS:
            self.file(f'ultimate-basic-{p}/ultimate-basic-{p}.zip')

    def test_collects_the_platform_zips_not_the_linux_packages(self):
        self.packages()
        self.file('ultimate-basic-linux-deb/ultimate-basic_1.6.6_amd64.deb')
        self.file('ultimate-basic-linux-rpm/ultimate-basic-1.6.6-1.x86_64.rpm')
        self.assertEqual(set(publish.collect_files(self.root)), publish.PACKAGES)

    def test_rejects_a_missing_platform_before_connecting(self):
        self.packages()
        (self.root / 'ultimate-basic-macos-x64/ultimate-basic-macos-x64.zip').unlink()
        with patch.dict(os.environ, ENV), patch.object(publish.ftplib, 'FTP_TLS') as ftp:
            with self.assertRaisesRegex(ValueError, 'macos-x64'):
                publish.upload(self.root)
            ftp.assert_not_called()

    def test_rejects_duplicate_names(self):
        self.packages()
        self.file('again/ultimate-basic-linux-x64.zip')
        with self.assertRaisesRegex(ValueError, 'Duplicate'):
            publish.collect_files(self.root)

    def test_uploads_with_tls_and_checks_sizes(self):
        self.packages()
        ftp = MagicMock()
        ftp.size.return_value = 8
        with patch.dict(os.environ, ENV), patch.object(publish.ftplib, 'FTP_TLS') as cls:
            cls.return_value.__enter__.return_value = ftp
            publish.upload(self.root)
            self.assertTrue(cls.call_args.kwargs['context'].check_hostname)
        ftp.prot_p.assert_called_once()
        self.assertEqual(ftp.storbinary.call_count, 5)
        ftp.delete.assert_not_called()


if __name__ == '__main__':
    unittest.main()
