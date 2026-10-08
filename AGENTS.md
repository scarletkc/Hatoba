# Agent instructions

- [docs/hatoba-spec.md](docs/hatoba-spec.md) defines behavior, data formats,
  the sync protocol, and the security model. When a change alters shipped
  behavior, update the matching section there and the progress in
  [docs/status.md](docs/status.md).
- [docs/development.md](docs/development.md) lists the build, run, and test
  commands.
- [docs/design/README.md](docs/design/README.md) lists the design files and
  the conventions for porting them.

## Documentation convention

Markdown in this repository follows the Seiso Convention 0.2.0:
https://seiso.fog.moe/0.2.0/convention

- Every document has one `kind`, declared in frontmatter or assigned by the
  configured path mapping: readme, howto, reference, runbook, agents, adr,
  plan, or changelog; `generated` is assigned only by mapping. Hold only
  what that kind is for; a how-to gives steps, a reference gives
  definitions, an ADR gives the reasons.
- Each fact has one home. Link to it instead of restating it.
- Long-lived pages state requirements and point to sources. They do not
  record the current version, deployment state, commit id, or count.
- A pointer names a file, symbol, section, or document, never "the source"
  or a repository root.
- Do not address the person who asked for the document or describe how it
  was written.
- Where the checker in use reports a finding that does not apply, write
  `<!-- seiso: allow CODE -- reason -->` with that finding's complete code
  and a reason a reviewer can evaluate. Without a checker, or without a
  finding to name, satisfy the requirement instead.

The path mappings live in [seiso.toml](seiso.toml). Run `seiso check` after
changing Markdown.
