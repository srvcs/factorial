# srvcs-factorial

The factorial orchestrator of the srvcs.cloud distributed standard library.

Its single concern: **`value!`** (factorial). It does no arithmetic of its own.
It folds a counted loop over `2..=value`, asking
[`srvcs-multiply`](https://github.com/srvcs/multiply) for each partial product:

```
acc = 1
for i in 2..=value: acc = multiply(acc, i)
```

So `0! == 1` and `1! == 1` (the loop body never runs). A negative input has no
factorial and is rejected with `422`.

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/` | Service identity, concern, and dependency list |
| `POST` | `/` | Compute `value!` |
| `GET` | `/healthz` `/readyz` `/metrics` `/openapi.json` | srvcs service standard surface |

```sh
curl -s -X POST localhost:8080/ -H 'content-type: application/json' -d '{"value": 5}'
# {"value":5,"result":120}
```

Responses:

- `200 {"value": v, "result": r}` — evaluated.
- `422 {"error": "factorial of a negative number"}` — negative input (or a
  non-integer value).
- `503` — the multiply dependency is unavailable.

## Dependencies

- [`srvcs-multiply`](https://github.com/srvcs/multiply)

A single request here fans out across the dependency graph: computing `5!`
makes four sequential `srvcs-multiply` calls (`1*2`, `2*3`, `6*4`, `24*5`).

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `SRVCS_BIND_ADDR` | `0.0.0.0:8080` | Bind address |
| `SRVCS_MULTIPLY_URL` | `http://127.0.0.1:8086` | Base URL of `srvcs-multiply` |
| `SRVCS_ENV` | `development` | Environment label for logs |
| `RUST_LOG` | `info,tower_http=info` | Tracing filter |

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Orchestration tests stand up a mock `srvcs-multiply` service in-process that
actually **computes** `a * b`, so the counted-loop logic is genuinely
exercised: `5! == 120`, `0! == 1`, `1! == 1`, plus a negative `422` and a
degraded dependency (`503`). See
[`srvcs/platform`](https://github.com/srvcs/platform) for the shared standard.

> Note: the `cargoHash` in `flake.nix` is inherited from the template and must be
> refreshed with a `nix build` before the Nix gates pass.
