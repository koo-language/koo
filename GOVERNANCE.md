# Koo Governance

This document describes how decisions are made in the Koo project and who makes them. It applies to every repository in the `koo-language` GitHub organization.

## Roles

### Project lead

The project lead has final authority over the language's design, the roadmap, release timing, and what merges into the main repositories. The project lead appoints and removes core team members and resolves anything the core team cannot.

The project lead is **Doron Grinstein** ([@dorongrinstein](https://github.com/dorongrinstein)).

### Core team

Core team members maintain specific areas of the project and have merge rights for those areas. They review pull requests, triage issues, discuss proposals, and mentor new contributors. Core team members are appointed by the project lead, normally after a sustained record of high-quality contributions.

| Area | Scope | Members |
| --- | --- | --- |
| Compiler | Parser, type checker, ownership inference, KIR, backends | Doron Grinstein |
| Runtime | Scheduler, actors, allocator, I/O | Doron Grinstein |
| Standard library | Everything under `std/` | Doron Grinstein |
| Tooling | `koo` CLI, formatter, language server, package manager, registry | Doron Grinstein |
| Documentation | Specification, website, guides | Doron Grinstein |

A core team member may step down at any time by opening a pull request against this file. A member who has been inactive for six months may be moved to emeritus status by the project lead.

### Contributors

Anyone who opens an issue, submits a pull request, writes documentation, reports a bug, or takes part in a proposal discussion is a contributor. No membership or approval is needed. Contributors are listed in the repository history and in release notes.

## How decisions are made

### Ordinary changes

Bug fixes, performance work, documentation, new platform support, additions to the standard library that do not change existing behavior, and tooling improvements are ordinary changes. They are made by pull request and merged by a core team member for the relevant area after review. A core team member does not merge their own pull request without a second review except for trivial fixes.

### Language changes

Any change to Koo's syntax or semantics, to the public API of the standard library, or to the behavior of the `koo` tool in a way users would notice is a language change. Language changes require a proposal.

1. **Draft.** Open a pull request against the [`koo-language/proposals`](https://github.com/koo-language/proposals) repository using the template there. A proposal states the problem, the proposed change, alternatives considered, how the change interacts with the rest of the language, and a migration path if anything existing would break.
2. **Discussion.** The community and the core team comment on the pull request. The author revises. There is no fixed length; most proposals need a few weeks.
3. **Decision.** When discussion has settled, the project lead accepts, rejects, or defers the proposal, with written reasons. A deferred proposal may be reopened when the circumstances that caused the deferral change.
4. **Implementation.** An accepted proposal is implemented behind a feature flag (`koo build --feature <name>`). It is tracked in an issue that lists remaining work.
5. **Stabilization.** When the implementation is complete, tested in the conformance suite, and documented in the specification, the project lead stabilizes it. The flag is removed in the next release.

Proposals are numbered in the order accepted and are permanent records of why the language is the way it is.

### Disagreements

Most disagreements are resolved in the discussion of the pull request or proposal where they arise. If a core team member and a contributor cannot agree, any other core team member may be asked to weigh in. If the core team cannot agree, the project lead decides. Decisions are explained in writing in the thread where the question was raised.

## Releases

Koo follows semantic versioning.

- **Patch releases** (1.2.3 → 1.2.4) fix bugs and ship as needed.
- **Minor releases** (1.2 → 1.3) add features and ship on a regular cadence, published in the roadmap.
- **Major releases** (1.x → 2.0) may break compatibility and require a new major-version proposal and an automated migration tool. There is no schedule for major releases and no expectation of one.

From version 1.0, code that compiles under any 1.x release compiles under every later 1.x release. Before 1.0, each minor release may break compatibility; breaking changes are listed in the release notes with the proposal that caused them.

The project lead decides when a release ships. A release is cut by a core team member following the documented release checklist.

## Stability tiers

Platform support is graded so users know what to expect.

| Tier | Meaning |
| --- | --- |
| 1 | Built and fully tested in CI on every merge; official binaries published; a failure blocks the release |
| 2 | Built in CI on every merge; tests may not all run; official binaries published; a failure is fixed before the next release but does not block |
| 3 | Code exists and is expected to work; not built in CI; no official binaries |

The current tier of every target is listed in the specification.

## Code of conduct

Participation in the Koo project is governed by the [Contributor Covenant, version 2.1](https://www.contributor-covenant.org/version/2/1/code_of_conduct/). Reports go to conduct@koo.io and are handled by the project lead, or by a core team member the project lead designates when the report concerns the project lead.

## Licensing and contributions

Koo is dual-licensed under Apache 2.0 and MIT; see `LICENSE`. Contributions are accepted under the same terms. Each commit is signed off under the Developer Certificate of Origin (`git commit -s`), which certifies that you have the right to submit the change. No contributor license agreement is required, and no copyright is assigned.

The name Koo and the Koo logo are trademarks; see `TRADEMARK.md`.

## Changing this document

This document is changed by pull request. Changes to roles or to the decision process require the project lead's approval. Changes that are purely editorial may be merged by any core team member.
