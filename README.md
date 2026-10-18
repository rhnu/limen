# Limen

English | [简体中文](README.zh-CN.md)

Limen is a Rust Chat Completions gateway for personal and private-network use. Local JavaScript adapts providers and request/response handling, while explicit model aliases give clients stable names. Built-in OpenAI, DeepSeek and OpenRouter factories provide typed provider options; see the [provider guide](docs/features/configuration.md#内置-provider) and [offline examples](examples/providers/).

## Quick start

Get a checkout of this repository and install Docker Compose, then run the following from the repository root in a POSIX shell. The supplied image targets Linux amd64.

```sh
mkdir -p config data logs
cp examples/docker/server.toml examples/docker/gateway.js config/
```

On Linux, grant UID/GID 10001 read access to `config/` and write access to `data/` and `logs/` before starting; the container runs without root or permission repair.

Edit the copied configuration:

- In `config/server.toml`, replace `auth.api_key` with your client token.
- In `config/gateway.js`, register the provider and its model with `defineGateway()`, setting the complete upstream Chat Completions URL, separate upstream credentials, and the upstream model name.

Validate and start the gateway:

```sh
docker compose run --rm limen check
docker compose up -d
```

The gateway listens at `http://127.0.0.1:8080`. See the deployment guide for PowerShell preparation, environment variables, and operations.

## Documentation

Detailed guides are in Simplified Chinese:

- [Documentation index](docs/README.md)
- [Build from source and development](docs/development.md)
- [Deployment guide](docs/features/deployment.md)

Licensed under [AGPL-3.0-only](LICENSE).
