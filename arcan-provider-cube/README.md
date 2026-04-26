# arcan-provider-cube

[`HypervisorBackend`] implementation backed by [TencentCloud/CubeSandbox][cube]'s CubeAPI v1.

[cube]: https://github.com/TencentCloud/CubeSandbox

## Identity

| | |
|---|---|
| Backend name | `cube` |
| Capabilities | `FILESYSTEM_READ \| FILESYSTEM_WRITE \| FILESYSTEM_EXT \| NETWORK_EGRESS \| PERSISTENCE \| FORK \| TAGS` |
| Hibernate / resume | not supported in v1 (returns `BackendError::NotSupported`) |

## Environment

| Variable | Required | Notes |
|---|---|---|
| `CUBE_API_URL` | yes | Base URL of the CubeSandbox HTTP API (e.g. `http://localhost:8080`) |
| `CUBE_API_TOKEN` | yes | Bearer token issued by the Cube control plane |

## Dev setup

See [`deploy/test/cube/README.md`](../../../deploy/test/cube/README.md) for WSL2 and Hetzner AX41 install paths.

## Tests

* Unit tests (mockito): `cargo test -p arcan-provider-cube`
* Conformance against real Cube: `cargo test -p life-kernel-conformance --test conformance_local_cube -- --ignored`
* Cold-start bench: `cargo test -p arcan-provider-cube --test cold_start_latency -- --ignored`

[`HypervisorBackend`]: aios_protocol::hypervisor::HypervisorBackend
