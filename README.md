# RGB Multisig Hub

RGB multisig hub to enable collaboration between a group of cosigners.

Cosigners are the members of the multisig setup, they propose new operations
and review, then accept or refuse, the ones proposed by other cosigners.

Optionally, third parties can be given watch-only access.

Each cosigner runs an [rgb-lib] wallet with the same multisig setup, used for
all operations except signing, and a singlesig (software or hardware) wallet
only used for signing.
Watch-only parties only run the multisig wallet and they're only allowed access
to a subset of the exposed APIs.

## Overview

The hub allows a group of cosigners to exchange the information needed to
cooperate a multisig setup.

On initial setup, authentication and the cosigners are configured.
Authentication is handled via biscuit tokens, see [Authentication] for
details. Configuration includes the xPubs for all cosigners, the thresholds for
operation approval and rgb-lib versioning, see [Configuration] for details.

Once the hub is operational, cosigners can use it to propose a new operation,
which is then retrieved by the others, who will review it and respond to either
approve or deny it.

There can only be 1 pending operation at a time. Once enough cosigners have
responded to either reach the threshold (operation approved) or make it
impossible to reach (operation discarded), the operation moves to its final
state and cosigners can process (approved) or skip (discarded) the operation.

Cosigners get the operations from the hub by their (progressive) ID and are
responsible for keeping track of the last operation they have processed. When a
new operation is retrieved from the hub, it can either be pending (to be
reviewed and responded to), approved (to be processed) or discarded (to be
skipped).

Operations must be processed in order. Cosigners are responsible to make sure
they have processed all operations before they propose or process a new one.
The hub keeps track of the last processed operation for each cosigner in order
to help prevent accidental out-of-order processing.

It is advised not to run more than one copy of each cosigner, although the hub
design should allow such mode of operation.

## Requirements

The [biscuit-cli] tool is needed to setup and manage authentication.

Local installation requires [cargo].

Each cosigner must run a compatible version of rgb-lib, specifically:
- the configured rgb-lib version must be in the hub's supported version range
- all cosigners must use the rgb-lib version specified in the configuration file

See [Configuration] for details on how the version boundaries are set.

The service has currently only been tested on Linux but it may run on other
operating systems as well.

## Install

Clone the project:
```sh
git clone https://github.com/RGB-Tools/rgb-multisig-hub
```

### Local

To install the hub locally, from the project root, run:
```sh
cargo install --locked --path .
```

This will produce the `rgb-multisig-hub` binary.

### Docker

To build the docker image, run:
```sh
docker build -t rgb-multisig-hub .
```

## Setup

Before the hub can be run, authentication (root keys and tokens) needs to be
setup and the service needs to be configured.

### Authentication

Authentication is handled via [Biscuit tokens].

To setup the authentication, a root key pair needs to be generated. The private
key is used to generate new signed tokens manually. The public key is
configured in the hub and is used to verify the tokens provided by users in
request headers.

Authentication is mandatory and cannot be disabled.

#### Root key pair

Key and token generation are handled via the biscuit CLI, which can be
installed via cargo:
```sh
cargo install biscuit-cli
```
or a pre-built binary can be downloaded from the [biscuit-cli releases page].

To generate the private key, run:
```sh
biscuit keypair --only-private-key > private-key-file
```

To generate the corresponding public key, run:
```sh
biscuit keypair --from-file private-key-file --only-public-key
```
See [Configuration] for how to configure this in the hub.

Notes:
- the root private key must be kept secret and it is advised to store it safely
  in a password manager
- if the root private key is compromised or lost it needs to be abandoned and
  a new one needs to be generated, along with its public counterpart
- changing root key pair means updating the configured root public key, which
  will make all previous tokens become invalid so new ones will need to be
  generated and distributed

#### Tokens

Tokens must have a role, either `cosigner` or `watch-only`. Cosigner tokens
must embed their xPub, watch-only tokens must not embed an xPub. Cosigner
tokens grant access to all APIs, watch-only tokens only grant access to a
subset of the APIs.

To generate a cosigner token, run:
```sh
echo 'role("cosigner"); xpub("<cosigner_xpub>");' \
  | biscuit generate --private-key-file private-key-file -
```
Repeat this for all cosigners, each identified by its xPub.

To generate a watch-only token, run:
```sh
echo 'role("watch-only");' \
  | biscuit generate --private-key-file private-key-file -
```

Tokens can also carry an **expiry** date. A `check` clause can be added to
enforce it. Here's an example for a watch-only token:
```sh
echo 'role("watch-only"); check if time($t), $t <= 2026-12-25T00:00:00Z;' \
  | biscuit generate --private-key-file private-key-file -
```

Tokens can be revoked, but this features is not implemented at the moment.

### Configuration

The service needs a data directory and a TOML configuration file.

The storage data directory (e.g. `data`) is passed as a CLI parameter when
starting the service. The configuration file is named `config.toml` and is
located inside the data directory (e.g. `data/config.toml`).

The configuration file requires the following parameters to be set:
- `cosigner_xpubs`: list of the cosigner xPubs
- `threshold_colored`: the threshold for colored operations
- `threshold_vanilla`: the threshold for vanilla operations
- `root_public_key`: the 32-byte hex-encoded authentication root public key
                     (without the `ed25519/` prefix)
- `rgb_lib_version`: the `<major.minor>` rgb-lib version that all cosigners
                     must use

Notes:
- after the service has started, the `cosigner_xpubs` and `threshold_*`
  parameters cannot be changed
- `rgb_lib_version` must fall in the `MIN_RGB_LIB_VERSION`-`MAX_RGB_LIB_VERSION`
  range, defined in `src/startup.rs`

An example configuration file:
```toml
cosigner_xpubs = [
    "tpubD6NzVbkrYhZ4XJ6aDsDYTCUkn1QqC6ie7eappEWB823FLSsRo1VBoEmtQVPJEJYdBt1UArW74BJg54FbW217Xoae6SDgj71JQZTfYCSJUyy",
    "tpubD6NzVbkrYhZ4XoJ4SGokACCMyKUYycuuu4tNDAW9qQrksXPNU9C9jeqQJQsdd18Dgt5v2hcc1w4qjNqYQg4nJ15YQNBHsWUuv2cEmneU7Mn",
    "tpubD6NzVbkrYhZ4WYbMkJwEwwTsQfjND3xNcXF6MoG7Ge8DbP8yWAkeg7DKPcuYfuHZYxCGWg9bFsAKLvJjb66LRM1wAkeszXKNAZdPpwnfHtd",
    "tpubD6NzVbkrYhZ4Yj7WVQNN28FDdpGyyscw1vi73xuxNoKqQ6uStVh3Pp11sh6y1PT7ohULyP6suzZkDBUuLvx7qd3YK4eU36rxAL9wdKRnVJk",
]
threshold_colored = 3
threshold_vanilla = 2
root_public_key = "df200ea3dab3eae6e518e55e6853dc39c50979d77a7d3d36c964c534c66bfad2"
rgb_lib_version = "0.3"
```

## Run

Once the installation and initial setup are complete, the hub daemon can be
started.

### Local

To start the hub daemon locally, run:
```sh
rgb-multisig-hub <data_dir>
```

The data directory needs to exist and contain the configuration file.

### Docker

To start the hub container, run:
```sh
docker run -it \
  -p 3001:3001 \
  -v <host_dir_or_volume>:/srv/data \
  rgb-multisig-hub
```

Notes:
- if using a host directory, it doesn't need to already exist
- it is advised not to use the same host data directory for both local and
  docker running, as the docker container runs as root and may change file
  permissions

## Stop

To stop the daemon press `Ctrl+C` on the console where it is running or, if
running in docker, stop the container.

## Use

Once the daemon is running, it can be operated via HTTP JSON APIs.

The node currently exposes the following APIs:
- `/bumpaddressindices` (POST)
- `/getcurrentaddressindices` (GET)
- `/getfile` (POST)
- `/getlastprocessedopidx` (GET)
- `/getoperationbyidx` (POST)
- `/info` (GET)
- `/markoperationprocessed` (POST)
- `/postoperation` (POST)
- `/respondtooperation` (POST)

See the [OpenAPI specification] for details.

All requests must include the Biscuit token in the `Authorization` header.

### Swagger

A Swagger UI for the `master` branch is generated from the specification and
made available at https://rgb-tools.github.io/rgb-multisig-hub.

A local copy can be exposed. To do so, from the project root, run:
```sh
docker run -it \
  -p 8246:8080 \
  -e SWAGGER_JSON=/var/specs/openapi.yaml \
  -v $PWD/openapi.yaml:/var/specs/openapi.yaml \
  swaggerapi/swagger-ui
```
It can then be accessed by pointing a browser at `http://localhost:8246`.

If a daemon is running on the local host on the default port (3001), the APIs
can be called directly from the Swagger UI.

Authentication is achieved by adding the token via the Authorize button (lock
icon) at the top right, pasting the token and clicking `Authorize`.

### Curl

APIs can be called via curl.

An example `getoperationbyidx` call:
```sh
curl -X POST -H "Content-type: application/json" \
    -H "Authorization: Bearer <token>" \
    -d '{"operation_idx": "1"}' \
    http://localhost:3001/getoperationbyidx
```


[Authentication]: #authentication
[Biscuit tokens]: https://www.biscuitsec.org/
[Configuration]: #configuration
[OpenAPI specification]: /openapi.yaml
[biscuit-cli releases page]: https://github.com/eclipse-biscuit/biscuit-cli/releases
[biscuit-cli]: https://github.com/eclipse-biscuit/biscuit-cli
[cargo]: https://github.com/rust-lang/cargo
[rgb-lib]: https://github.com/RGB-Tools/rgb-lib
