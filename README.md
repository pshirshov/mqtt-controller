# mqtt-controller

Unified zigbee2mqtt provisioner and runtime controller with a web dashboard.

The flake exposes:

- `packages.<system>.default` and `packages.<system>.mqtt-controller`
- `packages.<system>.mqtt-controller-frontend`
- `overlays.default`
- `nixosModules.default`

Import `nixosModules.default`, then configure
`smind.services.mqtt-controller`. The module installs the controller package
through the flake overlay.

Build and test the complete package with:

```sh
nix build
cargo test -p mqtt-controller -p mqtt-controller-wire --locked
```

Frontend and motion-rule details are documented in [docs/frontend.md](docs/frontend.md)
and [docs/motion.md](docs/motion.md).
