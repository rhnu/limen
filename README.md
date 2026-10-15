# Limen

[English](README.md) | [简体中文](README.zh-CN.md)

Limen is a configurable Chat Completions gateway written in Rust for personal and private-network use.
Expose a small set of model aliases, connect them to your upstream providers, and adapt requests and
responses with local JavaScript modules. Linux Docker is the primary deployment target.

## Features

- OpenAI-style `POST /v1/chat/completions` and `GET /v1/models` endpoints.
- Explicit model aliases with reusable provider settings and independent client and upstream authentication.
- Ordered provider → model pipelines for request bodies, URLs, headers, responses, and streaming events.
- Synchronous and asynchronous JavaScript hooks, local ESM imports, and per-request state.
- JSON and SSE in all four upstream/downstream combinations, including text, refusal, function tool calls,
  multiple choices, finish reasons, and usage when converting between formats.
- Bounded requests, queues, JavaScript execution, event expansion, and stream aggregation, with explicit
  timeout and truncated-stream errors.
- Configuration validation, structured metadata logs, and a non-root Docker image.

See the [feature and usage guide](docs/features.md) (Chinese) for configuration and hook examples.
The [roadmap](docs/roadmap.md) tracks remaining deployment validation. Limen currently focuses on Chat
Completions; automatic retries, failover, hot reload, model discovery, UI, and other API families are
outside its scope.

## Build from source

Requirements:

- Rustup and the toolchain pinned in [rust-toolchain.toml](rust-toolchain.toml).
- A C/C++ build toolchain, CMake, and platform development tools for QuickJS and AWS-LC.
  On Debian/Ubuntu, install `build-essential`, `cmake`, and `pkg-config`. On Windows, use the MSVC
  build tools and CMake.

```sh
git clone https://github.com/rhnu/limen.git
cd limen
cargo build --release --locked --bin limen
```

The binary is `target/release/limen` (`target/release/limen.exe` on Windows).

## Quick start

The commands below use a POSIX shell. In PowerShell, copy the configuration tree with
`Copy-Item -LiteralPath examples -Destination local -Recurse`. The curl examples use POSIX line
continuations and quoting; adapt them for PowerShell and invoke `curl.exe`.

1. Copy the entire example tree, preserving the shared module imports:

   ```sh
   cp -R examples local
   ```

2. Edit `local/basic/server.yaml` and set your client bearer token. Edit `local/basic/config.js` with
   your provider's **full Chat Completions URL**, upstream credentials, and upstream model name.
   The bundled `.invalid` URL and tokens are placeholders. Keep real configuration in `local/`.

3. Validate the configuration, then start the gateway:

   ```sh
   ./target/release/limen check --config local/basic/server.yaml
   ./target/release/limen serve --config local/basic/server.yaml
   ```

The default listener is `127.0.0.1:8080`. `config.js` is loaded from the directory containing the
selected YAML file. `check` executes top-level JavaScript, including imported modules and top-level
await; it does not call a provider or run request hooks.

### Call the API

Replace the token below with the one in your YAML configuration. The `chat` model is the public alias
defined in the example configuration.

```sh
curl http://127.0.0.1:8080/v1/models \
  -H 'Authorization: Bearer example-limen-client-token'

curl http://127.0.0.1:8080/v1/chat/completions \
  -H 'Authorization: Bearer example-limen-client-token' \
  -H 'Content-Type: application/json' \
  -d '{"model":"chat","messages":[{"role":"user","content":"Hello"}]}'
```

For streaming, use `curl -N` and add `"stream":true` to the request body:

```sh
curl -N http://127.0.0.1:8080/v1/chat/completions \
  -H 'Authorization: Bearer example-limen-client-token' \
  -H 'Content-Type: application/json' \
  -d '{"model":"chat","messages":[{"role":"user","content":"Hello"}],"stream":true}'
```

Clients with a configurable Chat Completions base URL can use `http://127.0.0.1:8080/v1`, the client
bearer token, and a configured model alias. Select their Chat Completions mode.

## Docker

Docker Engine or Docker Desktop with Linux containers and Docker Compose is required. The supplied
build targets Linux `amd64`.

After copying `examples/` to `local/`, fill in the provider settings in `local/basic/config.js` and
the client bearer token in `local/docker/server.yaml`. Then run:

```sh
docker compose build
docker compose run --rm limen check --config /config/docker/server.yaml
docker compose up -d
docker compose logs -f
```

Stop the service with `docker compose down`. Compose mounts the whole `local/` tree read-only at
`/config` and publishes `127.0.0.1:8080` on the host. The container configuration listens on
`0.0.0.0:8080`; mounted files must be readable by UID/GID `10001`. Local credentials are excluded
from the image build context. See the [deployment guide](docs/features.md#docker-部署) for details.

## Configuration and operation

- `server.yaml` controls listening, client authentication, resource limits, and logging.
- The adjacent `config.js` exports providers, model aliases, pipelines, and optional global hooks.
- Client credentials are separate from provider credentials. Client Authorization and Cookie headers
  are not forwarded by default.
- Model routing happens once, after `onRequest`. Each pipeline stage runs provider steps first, then
  model steps, in array order.
- The original client `stream` setting selects the downstream format. Hooks can independently choose
  the upstream format. JSON → SSE waits for the complete upstream JSON response.
- A successful SSE response ends with `[DONE]`. A failure after streaming starts sends `event: error`
  and closes the stream without a success marker.
- Configuration changes require a restart. Ctrl-C stops the listener and cancels in-flight requests;
  Unix also supports SIGTERM.

Run `limen --help`, `limen check --help`, or `limen serve --help` for command options, and
`limen --version` for the version. Without `--config`, both commands use `server.yaml` in the current
directory. Detailed errors, exit codes, limits, and hook contracts are in the
[usage guide](docs/features.md).

## Documentation and development

The detailed documentation is currently in Chinese:

- [Features and usage](docs/features.md): configuration, API calls, hooks, streaming, and troubleshooting.
- [Design](docs/design.md): architecture and contracts, including confirmed decisions and validation gaps.
- [Development](docs/development.md): build requirements, Rusteward, unit tests, and isolated runtime experiments.
- [Roadmap](docs/roadmap.md): remaining work and deployment acceptance.

For contributions, read the [workspace conventions](AGENTS.md) and development guide. Keep real
credentials out of changes and distinguish unit-test results from runtime and deployment validation.

## License

Limen is licensed under the GNU Affero General Public License, version 3 only
(`AGPL-3.0-only`). See [LICENSE](LICENSE) for the full text.
