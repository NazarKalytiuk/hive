# Changelog

All notable changes to the Zed Tarn extension will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - 2026-04-16

Initial public release.

### Added
- Language registration for `.tarn.yaml` / `.tarn.yml` files.
- Syntax highlighting via a pinned `tree-sitter-yaml` grammar.
- `tarn-lsp` language server adapter with auto-download from GitHub releases, `$PATH` lookup, and `lsp.tarn-lsp.binary.path` override.
- Forwarding of `lsp.tarn-lsp.settings` via `workspace/configuration`.
- Snippets: `tarn-test`, `tarn-step`, `tarn-capture`, `tarn-poll`, `tarn-form`, `tarn-graphql`, `tarn-multipart`, `tarn-lifecycle`, `tarn-include`.
- Runnable tasks: `tarn: run file`, `tarn: dry-run file`, `tarn: validate file`, `tarn: run all`, `tarn: list all`, `tarn: validate all`.
- Gutter runnable for whole-file execution (`tarn-file` tag).
- Outline and bracket-matching queries.
