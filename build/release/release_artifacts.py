#!/usr/bin/env python3
"""Split, verify, and join release artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
import sys
import uuid
from dataclasses import asdict, dataclass
from pathlib import Path

MANIFEST_VERSION = 1
MAX_PART_SIZE = 1 << 30
# Reserve one of GitHub's 1000 release-asset slots for the manifest alongside parts.
MAX_PART_COUNT = 999
# Keep manifest parsing bounded well below GitHub's asset ceiling.
MAX_MANIFEST_BYTES = 1 << 20
BUFFER_SIZE = 1 << 20
PART_INDEX_WIDTH = 12


class ReleaseArtifactError(Exception):
    """Raised when an artifact manifest or part set is invalid."""


@dataclass(frozen=True)
class OriginalRecord:
    basename: str
    size: int
    sha256: str


@dataclass(frozen=True)
class PartRecord:
    name: str
    size: int
    sha256: str


@dataclass(frozen=True)
class ManifestRecord:
    manifest_version: int
    chunk_size: int
    original: OriginalRecord
    parts: list[PartRecord]


@dataclass(frozen=True)
class SplitOutcome:
    manifest_path: Path
    manifest: ManifestRecord


def _path_type(value: str) -> Path:
    return Path(value).expanduser()


def _parse_chunk_size(value: str) -> int:
    try:
        parsed = int(value, 10)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("chunk size must be a decimal byte count") from exc
    if type(parsed) is not int or isinstance(parsed, bool):
        raise argparse.ArgumentTypeError("chunk size must be an integer byte count")
    if parsed < 1:
        raise argparse.ArgumentTypeError("chunk size must be at least 1 byte")
    if parsed > MAX_PART_SIZE:
        raise argparse.ArgumentTypeError(
            f"chunk size must not exceed {MAX_PART_SIZE} bytes ({MAX_PART_SIZE // (1 << 20)} MiB)"
        )
    return parsed


def _duplicate_rejecting_object_pairs_hook(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ReleaseArtifactError(f"duplicate JSON key {key!r} in manifest")
        result[key] = value
    return result


def _type_name(value: object) -> str:
    return type(value).__name__


def _require_exact_keys(mapping: dict[str, object], expected: set[str], context: str) -> None:
    actual = set(mapping)
    missing = sorted(expected - actual)
    extra = sorted(actual - expected)
    if missing or extra:
        problems = []
        if missing:
            problems.append(f"missing keys {missing}")
        if extra:
            problems.append(f"unexpected keys {extra}")
        raise ReleaseArtifactError(f"{context} has {' and '.join(problems)}")


def _require_int(value: object, field: str, *, minimum: int | None = None, maximum: int | None = None) -> int:
    if type(value) is not int or isinstance(value, bool):
        raise ReleaseArtifactError(f"{field} must be an integer, not {_type_name(value)}")
    if minimum is not None and value < minimum:
        raise ReleaseArtifactError(f"{field} must be at least {minimum}")
    if maximum is not None and value > maximum:
        raise ReleaseArtifactError(f"{field} must not exceed {maximum}")
    return value


def _require_str(value: object, field: str) -> str:
    if type(value) is not str:
        raise ReleaseArtifactError(f"{field} must be a string, not {_type_name(value)}")
    return value


def _require_digest(value: object, field: str) -> str:
    digest = _require_str(value, field)
    if len(digest) != 64 or any(ch not in "0123456789abcdef" for ch in digest):
        raise ReleaseArtifactError(f"{field} must be a lowercase SHA-256 hex digest")
    return digest


def _validate_relative_name(value: object, field: str) -> str:
    name = _require_str(value, field)
    if not name:
        raise ReleaseArtifactError(f"{field} must not be empty")
    if "\x00" in name:
        raise ReleaseArtifactError(f"{field} must not contain NUL bytes")
    if os.path.isabs(name):
        raise ReleaseArtifactError(f"{field} must be a relative name, not an absolute path")
    if name in {".", ".."}:
        raise ReleaseArtifactError(f"{field} must not be a traversal component")
    if "/" in name or (os.altsep is not None and os.altsep in name):
        raise ReleaseArtifactError(f"{field} must not contain path separators")
    return name


def _validate_regular_source(path: Path) -> None:
    try:
        stat_result = path.stat()
    except FileNotFoundError as exc:
        raise ReleaseArtifactError(f"source file {path} does not exist") from exc
    except OSError as exc:
        raise ReleaseArtifactError(f"cannot stat source file {path}: {exc.strerror or exc}") from exc
    if not stat.S_ISREG(stat_result.st_mode):
        raise ReleaseArtifactError(f"source file {path} must be a regular file")


def _validate_existing_directory(path: Path, field: str) -> None:
    try:
        stat_result = path.stat()
    except FileNotFoundError as exc:
        raise ReleaseArtifactError(f"{field} {path} does not exist") from exc
    except OSError as exc:
        raise ReleaseArtifactError(f"cannot stat {field} {path}: {exc.strerror or exc}") from exc
    if not stat.S_ISDIR(stat_result.st_mode):
        raise ReleaseArtifactError(f"{field} {path} must be a directory")


def _cleanup_owned_paths(paths: list[Path]) -> list[str]:
    errors: list[str] = []
    for path in reversed(paths):
        try:
            path.unlink()
        except FileNotFoundError:
            continue
        except OSError as exc:
            errors.append(f"{path}: {exc.strerror or exc}")
    return errors


def _raise_with_cleanup(primary: ReleaseArtifactError, cleanup_errors: list[str]) -> None:
    if cleanup_errors:
        raise ReleaseArtifactError(f"{primary}; cleanup errors: {', '.join(cleanup_errors)}") from primary
    raise primary


def part_name_for(basename: str, index: int) -> str:
    return f"{basename}.part{index:0{PART_INDEX_WIDTH}d}"


def manifest_path_for(output_dir: Path, basename: str) -> Path:
    return output_dir / f"{basename}.manifest.json"


def _manifest_json_text(manifest: ManifestRecord) -> str:
    return json.dumps(asdict(manifest), ensure_ascii=False, indent=2) + "\n"


def _manifest_record_from_data(data: object) -> ManifestRecord:
    if type(data) is not dict:
        raise ReleaseArtifactError(f"manifest root must be a JSON object, not {_type_name(data)}")
    _require_exact_keys(data, {"manifest_version", "chunk_size", "original", "parts"}, "manifest root")

    manifest_version = _require_int(data["manifest_version"], "manifest_version", minimum=1)
    if manifest_version != MANIFEST_VERSION:
        raise ReleaseArtifactError(
            f"unsupported manifest version {manifest_version}; expected {MANIFEST_VERSION}"
        )

    chunk_size = _require_int(data["chunk_size"], "chunk_size", minimum=1, maximum=MAX_PART_SIZE)
    original = data["original"]
    if type(original) is not dict:
        raise ReleaseArtifactError(f"original must be a JSON object, not {_type_name(original)}")
    _require_exact_keys(original, {"basename", "size", "sha256"}, "original")

    basename = _validate_relative_name(original["basename"], "original.basename")
    size = _require_int(original["size"], "original.size", minimum=0)
    digest = _require_digest(original["sha256"], "original.sha256")

    parts_raw = data["parts"]
    if type(parts_raw) is not list:
        raise ReleaseArtifactError(f"parts must be a JSON array, not {_type_name(parts_raw)}")
    if len(parts_raw) > MAX_PART_COUNT:
        raise ReleaseArtifactError(
            f"manifest contains {len(parts_raw)} parts, exceeds {MAX_PART_COUNT}-part limit"
        )

    parts: list[PartRecord] = []
    seen_names: set[str] = set()
    total_size = 0
    for index, part_data in enumerate(parts_raw, start=1):
        field = f"parts[{index - 1}]"
        if type(part_data) is not dict:
            raise ReleaseArtifactError(f"{field} must be a JSON object, not {_type_name(part_data)}")
        _require_exact_keys(part_data, {"name", "size", "sha256"}, field)
        name = _validate_relative_name(part_data["name"], f"{field}.name")
        expected_name = part_name_for(basename, index)
        if name in seen_names:
            raise ReleaseArtifactError(f"duplicate part name {name!r} in manifest")
        if name != expected_name:
            raise ReleaseArtifactError(
                f"part list gap or out-of-order entry at index {index}: expected {expected_name!r}, found {name!r}"
            )
        part_size = _require_int(part_data["size"], f"{field}.size", minimum=1)
        if part_size > chunk_size:
            raise ReleaseArtifactError(f"{field}.size must not exceed chunk size {chunk_size}")
        part_digest = _require_digest(part_data["sha256"], f"{field}.sha256")
        if index < len(parts_raw) and part_size != chunk_size:
            raise ReleaseArtifactError(
                f"{field}.size must equal chunk size {chunk_size} for every non-final part"
            )
        parts.append(PartRecord(name=name, size=part_size, sha256=part_digest))
        seen_names.add(name)
        total_size += part_size

    if total_size != size:
        raise ReleaseArtifactError(
            f"part sizes sum to {total_size} bytes, but original size is {size} bytes"
        )

    return ManifestRecord(
        manifest_version=manifest_version,
        chunk_size=chunk_size,
        original=OriginalRecord(basename=basename, size=size, sha256=digest),
        parts=parts,
    )


def load_manifest(manifest_path: Path | str) -> ManifestRecord:
    path = Path(manifest_path)
    try:
        flags = os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_NOFOLLOW", 0)
        fd = os.open(path, flags)
        with os.fdopen(fd, "rb") as handle:
            stat_result = os.fstat(handle.fileno())
            if not stat.S_ISREG(stat_result.st_mode):
                raise ReleaseArtifactError(f"manifest file {path} must be a regular file")
            if stat_result.st_size > MAX_MANIFEST_BYTES:
                raise ReleaseArtifactError(
                    f"manifest file {path} exceeds {MAX_MANIFEST_BYTES}-byte limit"
                )
            raw_bytes = handle.read(MAX_MANIFEST_BYTES + 1)
        if len(raw_bytes) > MAX_MANIFEST_BYTES:
            raise ReleaseArtifactError(
                f"manifest file {path} exceeds {MAX_MANIFEST_BYTES}-byte limit"
            )
        raw_text = raw_bytes.decode("utf-8")
    except FileNotFoundError as exc:
        raise ReleaseArtifactError(f"manifest file {path} does not exist") from exc
    except UnicodeDecodeError as exc:
        raise ReleaseArtifactError(f"manifest file {path} is not valid UTF-8") from exc
    except OSError as exc:
        raise ReleaseArtifactError(f"cannot read manifest file {path}: {exc.strerror or exc}") from exc
    try:
        data = json.loads(raw_text, object_pairs_hook=_duplicate_rejecting_object_pairs_hook)
    except ReleaseArtifactError:
        raise
    except json.JSONDecodeError as exc:
        raise ReleaseArtifactError(
            f"manifest file {path} is not valid JSON at line {exc.lineno}, column {exc.colno}: {exc.msg}"
        ) from exc
    return _manifest_record_from_data(data)


def _verify_part_stream(
    part_path: Path, record: PartRecord, full_hasher: hashlib._Hash, output_file: object | None = None
) -> int:
    try:
        stat_result = part_path.stat(follow_symlinks=False)
    except FileNotFoundError as exc:
        raise ReleaseArtifactError(f"missing part file {part_path}") from exc
    except OSError as exc:
        raise ReleaseArtifactError(f"cannot stat part file {part_path}: {exc.strerror or exc}") from exc
    if stat.S_ISLNK(stat_result.st_mode):
        raise ReleaseArtifactError(f"part file {part_path} must not be a symlink")
    if not stat.S_ISREG(stat_result.st_mode):
        raise ReleaseArtifactError(f"part file {part_path} must be a regular file")
    if stat_result.st_size != record.size:
        raise ReleaseArtifactError(
            f"part file {part_path} has size {stat_result.st_size} bytes, expected {record.size}"
        )

    open_flags = os.O_RDONLY
    if hasattr(os, "O_NOFOLLOW"):
        open_flags |= os.O_NOFOLLOW

    try:
        fd = os.open(part_path, open_flags)
    except OSError as exc:
        raise ReleaseArtifactError(f"cannot open part file {part_path}: {exc.strerror or exc}") from exc

    part_hasher = hashlib.sha256()
    remaining = record.size
    consumed = 0
    try:
        with os.fdopen(fd, "rb") as handle:
            while remaining:
                chunk = handle.read(min(BUFFER_SIZE, remaining))
                if not chunk:
                    raise ReleaseArtifactError(
                        f"part file {part_path} was truncated: expected {record.size} bytes, read {consumed}"
                    )
                part_hasher.update(chunk)
                full_hasher.update(chunk)
                if output_file is not None:
                    try:
                        output_file.write(chunk)
                    except OSError as exc:
                        raise ReleaseArtifactError(
                            f"cannot write reconstructed output for part {part_path}: {exc.strerror or exc}"
                        ) from exc
                consumed += len(chunk)
                remaining -= len(chunk)
            post_stat = os.fstat(handle.fileno())
            if post_stat.st_size != record.size:
                raise ReleaseArtifactError(
                    f"part file {part_path} changed size during verification: expected {record.size}, found {post_stat.st_size}"
                )
    except ReleaseArtifactError:
        raise
    except OSError as exc:
        raise ReleaseArtifactError(f"cannot read part file {part_path}: {exc.strerror or exc}") from exc

    if part_hasher.hexdigest() != record.sha256:
        raise ReleaseArtifactError(f"sha256 mismatch for part file {part_path}")
    return consumed


def verify_manifest(manifest_path: Path | str, parts_dir: Path | str | None = None) -> ManifestRecord:
    manifest = load_manifest(manifest_path)
    manifest_path_obj = Path(manifest_path)
    parts_base = manifest_path_obj.parent if parts_dir is None else Path(parts_dir)
    _validate_existing_directory(parts_base, "parts directory")

    full_hasher = hashlib.sha256()
    total = 0
    for part in manifest.parts:
        total += _verify_part_stream(parts_base / part.name, part, full_hasher)

    if total != manifest.original.size:
        raise ReleaseArtifactError(
            f"verified {total} bytes from parts, but original size is {manifest.original.size}"
        )
    if full_hasher.hexdigest() != manifest.original.sha256:
        raise ReleaseArtifactError("full artifact sha256 mismatch")
    return manifest


def split_artifact(
    source_path: Path | str,
    output_dir: Path | str | None = None,
    chunk_size: int = MAX_PART_SIZE,
) -> SplitOutcome:
    source = Path(source_path)
    if type(chunk_size) is not int or isinstance(chunk_size, bool):
        raise ReleaseArtifactError("chunk size must be an integer")
    if chunk_size < 1 or chunk_size > MAX_PART_SIZE:
        raise ReleaseArtifactError(
            f"chunk size must be between 1 and {MAX_PART_SIZE} bytes inclusive"
        )

    _validate_regular_source(source)
    output_base = source.parent if output_dir is None else Path(output_dir)
    _validate_existing_directory(output_base, "output directory")

    basename = source.name
    created_paths: list[Path] = []
    try:
        source_size = source.stat().st_size
    except OSError as exc:
        raise ReleaseArtifactError(f"cannot stat source file {source}: {exc.strerror or exc}") from exc
    required_part_count = 0 if source_size == 0 else (source_size + chunk_size - 1) // chunk_size
    if required_part_count > MAX_PART_COUNT:
        raise ReleaseArtifactError(
            f"split would create {required_part_count} parts, exceeds {MAX_PART_COUNT}-part limit"
        )

    source_hasher = hashlib.sha256()
    parts: list[PartRecord] = []
    total = 0
    try:
        with source.open("rb") as source_handle:
            part_index = 1
            while True:
                first_chunk = source_handle.read(min(BUFFER_SIZE, chunk_size))
                if not first_chunk:
                    break

                part_name = part_name_for(basename, part_index)
                part_path = output_base / part_name
                part_hasher = hashlib.sha256()
                part_size = 0
                try:
                    with part_path.open("xb") as part_handle:
                        created_paths.append(part_path)
                        chunk = first_chunk
                        while True:
                            part_handle.write(chunk)
                            source_hasher.update(chunk)
                            part_hasher.update(chunk)
                            part_size += len(chunk)
                            total += len(chunk)
                            if part_size >= chunk_size:
                                break
                            chunk = source_handle.read(min(BUFFER_SIZE, chunk_size - part_size))
                            if not chunk:
                                break
                        part_handle.flush()
                        os.fsync(part_handle.fileno())
                except FileExistsError as exc:
                    raise ReleaseArtifactError(f"refusing to overwrite existing output part {part_path}") from exc
                except OSError as exc:
                    raise ReleaseArtifactError(f"cannot write part file {part_path}: {exc.strerror or exc}") from exc

                parts.append(
                    PartRecord(name=part_name, size=part_size, sha256=part_hasher.hexdigest())
                )
                part_index += 1
                if part_size < chunk_size:
                    break
    except OSError as exc:
        primary_error = ReleaseArtifactError(f"cannot read source file {source}: {exc.strerror or exc}")
        cleanup_errors = _cleanup_owned_paths(created_paths)
        _raise_with_cleanup(primary_error, cleanup_errors)
    except ReleaseArtifactError as exc:
        cleanup_errors = _cleanup_owned_paths(created_paths)
        _raise_with_cleanup(exc, cleanup_errors)

    if total != source_size:
        primary_error = ReleaseArtifactError(
            f"source file changed during split: expected {source_size} bytes, read {total}"
        )
        cleanup_errors = _cleanup_owned_paths(created_paths)
        _raise_with_cleanup(primary_error, cleanup_errors)

    manifest = ManifestRecord(
        manifest_version=MANIFEST_VERSION,
        chunk_size=chunk_size,
        original=OriginalRecord(
            basename=basename,
            size=source_size,
            sha256=source_hasher.hexdigest(),
        ),
        parts=parts,
    )

    manifest_path = manifest_path_for(output_base, basename)
    temp_manifest_path = output_base / f".{basename}.manifest.{uuid.uuid4().hex}.json.tmp"

    try:
        manifest_text = _manifest_json_text(manifest)
        try:
            with temp_manifest_path.open("x", encoding="utf-8", newline="\n") as handle:
                created_paths.append(temp_manifest_path)
                handle.write(manifest_text)
                handle.flush()
                os.fsync(handle.fileno())
        except FileExistsError as exc:
            raise ReleaseArtifactError(
                f"refusing to overwrite existing temporary manifest {temp_manifest_path}"
            ) from exc
        except OSError as exc:
            raise ReleaseArtifactError(
                f"cannot write temporary manifest {temp_manifest_path}: {exc.strerror or exc}"
            ) from exc
        try:
            os.link(temp_manifest_path, manifest_path)
        except FileExistsError as exc:
            raise ReleaseArtifactError(f"refusing to overwrite existing manifest {manifest_path}") from exc
        except OSError as exc:
            raise ReleaseArtifactError(f"cannot publish manifest {manifest_path}: {exc.strerror or exc}") from exc
    except ReleaseArtifactError as exc:
        cleanup_errors = _cleanup_owned_paths(created_paths)
        _raise_with_cleanup(exc, cleanup_errors)

    try:
        temp_manifest_path.unlink()
    except FileNotFoundError:
        pass
    except OSError as exc:
        raise ReleaseArtifactError(
            f"artifact parts and manifest were published, but cleanup failed for "
            f"{temp_manifest_path}: {exc.strerror or exc}"
        ) from exc

    return SplitOutcome(manifest_path=manifest_path, manifest=manifest)


def join_artifact(
    manifest_path: Path | str,
    output_path: Path | str | None = None,
    parts_dir: Path | str | None = None,
) -> Path:
    manifest = load_manifest(manifest_path)
    manifest_path_obj = Path(manifest_path)
    parts_base = manifest_path_obj.parent if parts_dir is None else Path(parts_dir)
    _validate_existing_directory(parts_base, "parts directory")

    output_base = manifest_path_obj.parent
    final_output = output_base / manifest.original.basename if output_path is None else Path(output_path)
    final_parent = final_output.parent
    _validate_existing_directory(final_parent, "output directory")

    temp_output = final_parent / f".{final_output.name}.join.{uuid.uuid4().hex}.tmp"
    created_paths: list[Path] = []
    full_hasher = hashlib.sha256()

    try:
        try:
            output_handle_cm = temp_output.open("xb")
        except FileExistsError as exc:
            raise ReleaseArtifactError(f"refusing to overwrite existing temporary output {temp_output}") from exc
        except OSError as exc:
            raise ReleaseArtifactError(f"cannot create temporary output {temp_output}: {exc.strerror or exc}") from exc

        created_paths.append(temp_output)
        with output_handle_cm as output_handle:
            for part in manifest.parts:
                _verify_part_stream(parts_base / part.name, part, full_hasher, output_handle)
            try:
                output_handle.flush()
                os.fsync(output_handle.fileno())
            except OSError as exc:
                raise ReleaseArtifactError(
                    f"cannot flush reconstructed output {temp_output}: {exc.strerror or exc}"
                ) from exc
        if full_hasher.hexdigest() != manifest.original.sha256:
            raise ReleaseArtifactError("full artifact sha256 mismatch")
        try:
            os.link(temp_output, final_output)
        except FileExistsError as exc:
            raise ReleaseArtifactError(f"refusing to overwrite existing output file {final_output}") from exc
        except OSError as exc:
            raise ReleaseArtifactError(f"cannot publish output file {final_output}: {exc.strerror or exc}") from exc
    except ReleaseArtifactError as exc:
        cleanup_errors = _cleanup_owned_paths(created_paths)
        if cleanup_errors:
            raise ReleaseArtifactError(
                f"{exc}; cleanup errors: {', '.join(cleanup_errors)}"
            ) from exc
        raise

    try:
        temp_output.unlink()
    except FileNotFoundError:
        pass
    except OSError as exc:
        raise ReleaseArtifactError(
            f"cleanup failed removing temporary output {temp_output}: {exc.strerror or exc}"
        ) from exc

    return final_output


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="release-artifacts",
        description="Split, verify, and reconstruct GitHub release artifacts with streaming SHA-256 checks.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    split_parser = subparsers.add_parser(
        "split",
        help="split a large file into deterministic release parts",
    )
    split_parser.add_argument("source", type=_path_type, help="source artifact file")
    split_parser.add_argument(
        "--output-dir",
        type=_path_type,
        default=None,
        help="directory that will receive the parts and manifest (default: source directory)",
    )
    split_parser.add_argument(
        "--chunk-size",
        type=_parse_chunk_size,
        default=MAX_PART_SIZE,
        help=f"maximum bytes per part (default: {MAX_PART_SIZE})",
    )
    split_parser.set_defaults(func=_command_split)

    verify_parser = subparsers.add_parser(
        "verify",
        help="verify a manifest and all referenced parts",
    )
    verify_parser.add_argument("manifest", type=_path_type, help="manifest JSON file")
    verify_parser.add_argument(
        "--parts-dir",
        type=_path_type,
        default=None,
        help="directory that contains the parts (default: manifest directory)",
    )
    verify_parser.set_defaults(func=_command_verify)

    join_parser = subparsers.add_parser(
        "join",
        help="reconstruct the original file from a manifest and parts",
    )
    join_parser.add_argument("manifest", type=_path_type, help="manifest JSON file")
    join_parser.add_argument(
        "--parts-dir",
        type=_path_type,
        default=None,
        help="directory that contains the parts (default: manifest directory)",
    )
    join_parser.add_argument(
        "--output",
        type=_path_type,
        default=None,
        help="destination path for the reconstructed file (default: manifest basename in manifest directory)",
    )
    join_parser.set_defaults(func=_command_join)

    return parser


def _command_split(args: argparse.Namespace) -> int:
    outcome = split_artifact(args.source, args.output_dir, args.chunk_size)
    print(
        f"split {len(outcome.manifest.parts)} part(s) from {outcome.manifest.original.size} bytes "
        f"into {outcome.manifest_path}"
    )
    return 0


def _command_verify(args: argparse.Namespace) -> int:
    manifest = verify_manifest(args.manifest, args.parts_dir)
    print(
        f"verified {len(manifest.parts)} part(s) for {manifest.original.basename} "
        f"({manifest.original.size} bytes)"
    )
    return 0


def _command_join(args: argparse.Namespace) -> int:
    output_path = join_artifact(args.manifest, args.output, args.parts_dir)
    manifest = load_manifest(args.manifest)
    print(f"joined {manifest.original.size} bytes into {output_path}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = _build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except ReleaseArtifactError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        print("error: interrupted", file=sys.stderr)
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
