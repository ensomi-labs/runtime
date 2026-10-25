# Agent guide

Read `README.md` first; its boundary rules are decisions, not suggestions.

- Core crates take no platform dependency and do no IO. Platform glue lives in binding
  crates or in the apps.
- Write the plainest code that meets the contract, in the style of the surrounding code.
  Validate only where outside input enters (fixtures, bindings, model files).
- Behaviour is pinned by `conformance/` fixtures: plain JSON, or a small binary format
  described beside it, that another implementation can read without this code.
- Apple builds (`scripts/build-apple.sh`, `swift test`) run on macOS. Cargo output stays in
  `target/`.

## Facts shared with other Ensomi repositories

Some facts this repository states are defined in another Ensomi repository, and
some defined here are stated elsewhere: the released model's files, revision and
licence, versions and pins, protocol messages, generation rules. ensomi-model's
[`docs/ecosystem.md`](https://github.com/ensomi-labs/ensomi-model/blob/main/docs/ecosystem.md)
says where each kind is defined and what a change affects in the other
repositories; with ensomi-model checked out beside this repository it is `../ensomi-model/docs/ecosystem.md`.
Before such a change, follow ensomi-model's
[`ensomi-cross-repo-changes`](https://github.com/ensomi-labs/ensomi-model/blob/main/.agents/skills/ensomi-cross-repo-changes/SKILL.md)
skill.
