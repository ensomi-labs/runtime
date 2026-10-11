# Agent guide

Read `README.md` first; its boundary rules are decisions, not suggestions.

- Core crates take no platform dependency and do no IO. Platform glue lives in binding
  crates or in the apps.
- Write the plainest code that meets the contract, in the style of the surrounding code.
  Validate only where outside input enters (fixtures, bindings, model files).
- Behaviour is pinned by `conformance/` fixtures: plain JSON, or a small binary format
  described beside it, that another implementation can read without this code.
- Builds and tests run on bings-mac (`~/ensomi/ensomi-runtime`, synced from the control
  plane). Cargo output stays in `target/`, which is not synced.
