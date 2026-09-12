# Sentia base-files delta

This directory carries the minimal Sentia fork delta for Debian trixie
`base-files`.

Goals:
- preserve Debian ownership of `/usr/lib/os-release` and `/etc/os-release`
  symlink behavior;
- set `ID=sentia` and `ID_LIKE=debian`;
- add `/etc/dpkg/origins/sentia` with `Parent: Debian`;
- set Sentia as dpkg default origin while preserving administrator custom
  defaults and migrating untouched `debian` defaults.

`apply-sentia-delta.py` applies this delta to an unpacked upstream Debian
`base-files` source tree.
