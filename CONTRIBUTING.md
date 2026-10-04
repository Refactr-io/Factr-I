# Contributing to Factr-I

Thanks for helping. Full build instructions and prerequisites are in
[docs/BUILDING.md](docs/BUILDING.md).

## Build and test, per component

Run each block from the repository root (the subshell keeps your directory).

Engine (Rust 1.88+):

```sh
(cd engine && cargo build --locked -p factr-cli && cargo test --workspace --locked)
```

Backend (Python 3.11 to 3.13, uv). The full suite is large; run the files you
touched, or the CI subset:

```sh
(cd backend && uv sync --locked --extra dev && uv run pytest -q tests/test_base_url_hostname.py)
```

Desktop (Node 22.22+ or 24.11+; always install from `desktop/`):

```sh
(cd desktop && npm ci && npm --workspace app run typecheck && npm --workspace app test)
(cd desktop && npm --workspace tests-js test)   # repository-level checks
```

## Name gate

Every change must pass the name gate (needs `rg`):

```sh
bash scripts/gate-names.sh
```

It must print zero hits. CI runs it.

## Commits and pull requests

- Short imperative subject (for example `Fix session restore on Windows`), a body
  that says why when it is not obvious, one logical change per commit.
- Open the pull request against `main`, fill in the template, and make sure CI
  is green.
- Do not commit secrets, `.env` files or credentials.

## License

Factr-I is MIT licensed (see `LICENSE`). By submitting a contribution you agree
that it is licensed under the MIT license and that you have the right to submit
it (Developer Certificate of Origin, https://developercertificate.org/).
Sign-off with `git commit -s` is welcome but not required.

## Conduct and security

Please follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report vulnerabilities
privately as described in [SECURITY.md](SECURITY.md), not in public issues.
