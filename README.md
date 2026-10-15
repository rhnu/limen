# Limen

English | [简体中文](README.zh-CN.md)

Limen is a Rust Chat Completions gateway for personal and private-network use. Local JavaScript adapts providers and request/response handling, while explicit model aliases give clients stable names.

## Quick start

Get a checkout of this repository and install Docker Compose, then run the following from the repository root in a POSIX shell. The supplied image targets Linux amd64.

```sh
mkdir -p data
cp examples/docker/server.toml examples/docker/config.js data/
```

Edit the copied configuration:

- In `data/server.toml`, replace `client_auth.bearer` with your client token.
- In `data/config.js`, set the complete upstream Chat Completions URL, separate upstream credentials, and the upstream model name.

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
