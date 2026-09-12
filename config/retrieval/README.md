# Sentia Retrieval Sources

This directory defines the explicit retrieval-source allowlist.

- `sources.toml` limits indexing to packaged documentation, dpkg metadata,
  selected Debian man pages, and selected service documentation.
- `query-policy.json` pins protocol/router IDs and the trust-boundary text used
  in retrieval responses.

The indexer never crawls home directories or credentials paths and does not
execute `man` macros.
