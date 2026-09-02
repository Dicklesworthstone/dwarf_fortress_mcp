#!/usr/bin/env python3

from __future__ import annotations

import copy
import hashlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from typing import Any, Callable

CREATOR_PATH = Path(__file__).with_name("create_source_bundle.py")
CREATOR_SPEC = importlib.util.spec_from_file_location("source_bundle_creator_test", CREATOR_PATH)
if CREATOR_SPEC is None or CREATOR_SPEC.loader is None:
    raise RuntimeError("cannot load source bundle creator")
creator = importlib.util.module_from_spec(CREATOR_SPEC)
sys.modules[CREATOR_SPEC.name] = creator
CREATOR_SPEC.loader.exec_module(creator)
verifier = creator.verifier


def run_git(root: Path, *arguments: str, env: dict[str, str] | None = None) -> str:
    environment = os.environ.copy()
    if env is not None:
        environment.update(env)
    completed = subprocess.run(
        ["git", "-C", os.fspath(root), *arguments],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=environment,
    )
    return completed.stdout.strip()


class Fixture:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.repository = root / "repository"
        self.outputs = root / "outputs"
        self.repository.mkdir()
        self.outputs.mkdir()
        run_git(self.repository, "init", "-q")
        run_git(self.repository, "config", "user.email", "source-bundle@example.invalid")
        run_git(self.repository, "config", "user.name", "Source Bundle Tests")
        (self.repository / "nested").mkdir()
        (self.repository / "README.md").write_text("deterministic source\n", encoding="utf-8")
        tool = self.repository / "nested" / "tool.sh"
        tool.write_text("#!/usr/bin/env bash\nprintf 'ok\\n'\n", encoding="utf-8")
        tool.chmod(0o755)
        long_name = "x" * 96 + ".txt"
        (self.repository / "nested" / long_name).write_text("long path\n", encoding="utf-8")
        run_git(self.repository, "add", ".")
        fixed = {
            "GIT_AUTHOR_DATE": "2001-02-03T04:05:06Z",
            "GIT_COMMITTER_DATE": "2001-02-03T04:05:06Z",
        }
        run_git(self.repository, "commit", "-q", "-m", "fixture", env=fixed)

    def create(self, name: str) -> tuple[dict[str, Any], Path, Path, Path]:
        destination = self.outputs / name
        result = creator.create_bundle(
            self.repository.resolve(),
            destination.resolve(),
            creator.DEFAULT_CONTRACT,
        )
        archive = Path(result["archive"])
        manifest = Path(result["manifest"])
        verification = Path(result["verification"])
        return result, archive, manifest, verification

    def rewrite_archive(
        self,
        archive: Path,
        manifest_path: Path,
        transform: Callable[[list[tuple[tarfile.TarInfo, bytes | None]]], None],
        *,
        trailing: bytes = b"",
    ) -> None:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        records: list[tuple[tarfile.TarInfo, bytes | None]] = []
        with tarfile.open(archive, "r:") as source:
            for member in source:
                extracted = source.extractfile(member) if member.isfile() else None
                data = extracted.read() if extracted is not None else None
                if extracted is not None:
                    extracted.close()
                clone = copy.copy(member)
                clone.pax_headers = {
                    key: value for key, value in member.pax_headers.items() if key == "path"
                }
                records.append((clone, data))
        transform(records)
        temporary = archive.with_name(f".{archive.name}.rewrite")
        with tarfile.open(
            temporary,
            "w",
            format=tarfile.PAX_FORMAT,
            pax_headers={"comment": manifest["commit"]},
        ) as destination:
            for member, data in records:
                destination.addfile(member, io.BytesIO(data) if data is not None else None)
        if trailing:
            with temporary.open("ab") as handle:
                handle.write(trailing)
        temporary.replace(archive)
        raw = archive.read_bytes()
        manifest["archive"]["bytes"] = len(raw)
        manifest["archive"]["sha256"] = hashlib.sha256(raw).hexdigest()
        unsigned = dict(manifest)
        unsigned.pop("manifest_digest", None)
        manifest["manifest_digest"] = verifier.sha256_bytes(
            verifier.canonical_json(unsigned)
        )
        manifest_path.write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )


class SourceBundleTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Fixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, Fixture(Path(temporary.name))

    def test_clean_bundle_round_trip_is_deterministic(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            first, first_archive, first_manifest, first_verification = fixture.create("first")
            _, second_archive, second_manifest, second_verification = fixture.create("second")
            self.assertEqual(first["status"], "created_and_verified")
            self.assertEqual(first["authority"]["mutation_capabilities"], [])
            self.assertEqual(first_archive.read_bytes(), second_archive.read_bytes())
            self.assertEqual(first_manifest.read_bytes(), second_manifest.read_bytes())
            self.assertEqual(first_verification.read_bytes(), second_verification.read_bytes())
            receipt = verifier.verify(
                first_manifest,
                first_archive,
                creator.DEFAULT_CONTRACT,
                fixture.repository.resolve(),
                True,
            )
            self.assertTrue(receipt["checkout"]["clean"])
            self.assertTrue(receipt["checkout"]["clean_required"])
            with tarfile.open(first_archive, "r:") as archive:
                members = archive.getmembers()
            regular_modes = {
                member.name: member.mode for member in members if member.isfile()
            }
            self.assertIn(0o644, regular_modes.values())
            self.assertIn(0o755, regular_modes.values())
            self.assertTrue(all(member.uid == 0 and member.gid == 0 for member in members))

    def test_dirty_source_and_existing_destination_fail_without_replacement(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            dirty = fixture.repository / "untracked.txt"
            dirty.write_text("dirty\n", encoding="utf-8")
            destination = (fixture.outputs / "dirty").resolve()
            with self.assertRaises(creator.SourceBundleCreationError):
                creator.create_bundle(
                    fixture.repository.resolve(), destination, creator.DEFAULT_CONTRACT
                )
            self.assertFalse(destination.exists())
            dirty.unlink()
            _, archive, manifest, verification = fixture.create("published")
            before = {
                archive: archive.read_bytes(),
                manifest: manifest.read_bytes(),
                verification: verification.read_bytes(),
            }
            with self.assertRaises(creator.SourceBundleCreationError):
                creator.create_bundle(
                    fixture.repository.resolve(),
                    (fixture.outputs / "published").resolve(),
                    creator.DEFAULT_CONTRACT,
                )
            for path, expected in before.items():
                self.assertEqual(path.read_bytes(), expected)

    def test_tracked_symbolic_link_is_rejected_without_publication(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            link = fixture.repository / "tracked-link"
            try:
                link.symlink_to("README.md")
            except OSError:
                self.skipTest("symbolic links are unavailable")
            run_git(fixture.repository, "add", "tracked-link")
            run_git(fixture.repository, "commit", "-q", "-m", "add symbolic link")
            destination = (fixture.outputs / "symlink").resolve()
            with self.assertRaises(
                (creator.SourceBundleCreationError, verifier.SourceBundleError)
            ):
                creator.create_bundle(
                    fixture.repository.resolve(), destination, creator.DEFAULT_CONTRACT
                )
            self.assertFalse(destination.exists())

    def test_tracked_gitlink_is_rejected_without_publication(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            nested = fixture.repository / "vendor" / "submodule"
            nested.mkdir(parents=True)
            run_git(nested, "init", "-q")
            run_git(nested, "config", "user.email", "source-bundle@example.invalid")
            run_git(nested, "config", "user.name", "Source Bundle Tests")
            (nested / "README.md").write_text("nested repository\n", encoding="utf-8")
            run_git(nested, "add", ".")
            run_git(nested, "commit", "-q", "-m", "nested fixture")
            nested_commit = run_git(nested, "rev-parse", "HEAD")
            run_git(
                fixture.repository,
                "update-index",
                "--add",
                "--cacheinfo",
                f"160000,{nested_commit},vendor/submodule",
            )
            run_git(fixture.repository, "commit", "-q", "-m", "add gitlink")
            self.assertEqual(
                run_git(
                    fixture.repository,
                    "status",
                    "--porcelain=v1",
                    "--untracked-files=all",
                ),
                "",
            )
            destination = (fixture.outputs / "gitlink").resolve()
            with self.assertRaises(
                (creator.SourceBundleCreationError, verifier.SourceBundleError)
            ):
                creator.create_bundle(
                    fixture.repository.resolve(), destination, creator.DEFAULT_CONTRACT
                )
            self.assertFalse(destination.exists())

    def test_reordered_and_semantically_duplicate_members_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            _, archive, manifest, _ = fixture.create("reordered")

            def reorder(records: list[tuple[tarfile.TarInfo, bytes | None]]) -> None:
                records[-1], records[-2] = records[-2], records[-1]

            fixture.rewrite_archive(archive, manifest, reorder)
            with self.assertRaises(verifier.SourceBundleError):
                verifier.verify(manifest, archive, creator.DEFAULT_CONTRACT)

            _, archive, manifest, _ = fixture.create("duplicate")

            def duplicate(records: list[tuple[tarfile.TarInfo, bytes | None]]) -> None:
                records.insert(1, (copy.copy(records[0][0]), records[0][1]))

            fixture.rewrite_archive(archive, manifest, duplicate)
            with self.assertRaises(verifier.SourceBundleError):
                verifier.verify(manifest, archive, creator.DEFAULT_CONTRACT)

    def test_noncanonical_member_metadata_is_rejected(self) -> None:
        mutations: list[
            tuple[str, Callable[[list[tuple[tarfile.TarInfo, bytes | None]]], None]]
        ] = []

        def wrong_mode(records: list[tuple[tarfile.TarInfo, bytes | None]]) -> None:
            next(member for member, _ in records if member.isfile()).mode = 0o664

        def wrong_owner(records: list[tuple[tarfile.TarInfo, bytes | None]]) -> None:
            records[0][0].uid = 1

        def mixed_time(records: list[tuple[tarfile.TarInfo, bytes | None]]) -> None:
            records[-1][0].mtime += 1

        def unsupported_pax(records: list[tuple[tarfile.TarInfo, bytes | None]]) -> None:
            records[-1][0].pax_headers["SCHILY.xattr.user.test"] = "1"

        mutations.extend(
            [
                ("mode", wrong_mode),
                ("owner", wrong_owner),
                ("mtime", mixed_time),
                ("pax", unsupported_pax),
            ]
        )
        for name, mutation in mutations:
            with self.subTest(name=name):
                temporary, fixture = self.fixture()
                with temporary:
                    _, archive, manifest, _ = fixture.create(name)
                    fixture.rewrite_archive(archive, manifest, mutation)
                    with self.assertRaises(verifier.SourceBundleError):
                        verifier.verify(manifest, archive, creator.DEFAULT_CONTRACT)

    def test_links_unmanifested_content_and_trailing_payload_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            _, archive, manifest, _ = fixture.create("link")

            def make_link(records: list[tuple[tarfile.TarInfo, bytes | None]]) -> None:
                index = next(
                    index for index, item in enumerate(records) if item[0].isfile()
                )
                member, _ = records[index]
                member.type = tarfile.SYMTYPE
                member.linkname = "target"
                member.size = 0
                records[index] = (member, None)

            fixture.rewrite_archive(archive, manifest, make_link)
            with self.assertRaises(verifier.SourceBundleError):
                verifier.verify(manifest, archive, creator.DEFAULT_CONTRACT)

            _, archive, manifest, _ = fixture.create("extra")

            def add_extra(records: list[tuple[tarfile.TarInfo, bytes | None]]) -> None:
                template = copy.copy(next(member for member, _ in records if member.isfile()))
                template.name = template.name.rsplit("/", 1)[0] + "/unmanifested.txt"
                template.size = 5
                template.mode = 0o644
                template.pax_headers = {}
                records.append((template, b"extra"))

            fixture.rewrite_archive(archive, manifest, add_extra)
            with self.assertRaises(verifier.SourceBundleError):
                verifier.verify(manifest, archive, creator.DEFAULT_CONTRACT)

            _, archive, manifest, _ = fixture.create("trailing")
            fixture.rewrite_archive(archive, manifest, lambda _records: None, trailing=b"evil")
            with self.assertRaises(verifier.SourceBundleError):
                verifier.verify(manifest, archive, creator.DEFAULT_CONTRACT)

    def test_manifest_and_checkout_tampering_are_rejected(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            _, archive, manifest, _ = fixture.create("tamper")
            value = json.loads(manifest.read_text(encoding="utf-8"))
            value["entries"][0]["sha256"] = hashlib.sha256(b"wrong").hexdigest()
            manifest.write_text(json.dumps(value) + "\n", encoding="utf-8")
            with self.assertRaises(verifier.SourceBundleError):
                verifier.verify(manifest, archive, creator.DEFAULT_CONTRACT)

            _, archive, manifest, _ = fixture.create("dirty-checkout")
            (fixture.repository / "untracked.txt").write_text("dirty\n", encoding="utf-8")
            with self.assertRaises(verifier.SourceBundleError):
                verifier.verify(
                    manifest,
                    archive,
                    creator.DEFAULT_CONTRACT,
                    fixture.repository.resolve(),
                    True,
                )
            observed = verifier.verify(
                manifest,
                archive,
                creator.DEFAULT_CONTRACT,
                fixture.repository.resolve(),
                False,
            )
            self.assertFalse(observed["checkout"]["clean"])
            self.assertFalse(observed["checkout"]["clean_required"])

    def test_verification_output_is_create_only(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "receipt.json"
            verifier.write_atomic(path, {"status": "first"})
            before = path.read_bytes()
            with self.assertRaises(verifier.SourceBundleError):
                verifier.write_atomic(path, {"status": "second"})
            self.assertEqual(path.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
