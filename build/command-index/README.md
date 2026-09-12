# command-index generator

Builds a bounded offline command-to-package index from official Debian
`Contents-*` metadata.

## Example

```bash
python3 build/command-index/generate_command_index.py \
  --contents /var/lib/apt/lists/deb.debian.org_debian_dists_trixie_main_Contents-amd64.gz \
  --suite trixie \
  --snapshot 2026-09-12T00:00:00Z \
  --source-uri https://snapshot.debian.org/archive/debian/20260912T000000Z/ \
  --output artifacts/command-index/command-index.json
```

The output includes provenance (input files, hashes, suite, snapshot, URI) and
is bounded by configurable limits to keep runtime memory predictable.
