# Changelog - [mqtt-viewer](https://github.com/bircni/mqtt-viewer)

All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

## [0.1.0] - 2026-11-26

### Continuous Integration

- run on Linux only - ([529de75](https://github.com/bircni/mqtt-viewer/commit/529de75ff4e59d868c178a44d41b7b503abaf4f6)) - bircni
- run three independent jobs instead of a dependency chain - ([81466e4](https://github.com/bircni/mqtt-viewer/commit/81466e4fbfb22009ea6a62ec05251d0f2f54387c)) - bircni
- build release files on request, draft the release before publishing it and check before tagging - ([9ed14e4](https://github.com/bircni/mqtt-viewer/commit/9ed14e474a8f16f3dca3de3402348f4ff364035c)) - bircni
- build the Windows archive and the macOS disk image on their own runners - ([0a84a22](https://github.com/bircni/mqtt-viewer/commit/0a84a228209422b6170222381162d8c685564319)) - bircni

### Documentation

- restructure the README with screenshots, add CONTRIBUTING and release tooling - ([6695370](https://github.com/bircni/mqtt-viewer/commit/6695370711b8ac16f015e080ca19ded5a812985c)) - bircni
- describe the packages and document icon and macOS releases - ([3f14076](https://github.com/bircni/mqtt-viewer/commit/3f14076b43bd076a7417b1192f21cb21b536adbb)) - bircni

### Features

- **(app)** add the egui application with tree, value, publish, chart and settings views - ([a9fb647](https://github.com/bircni/mqtt-viewer/commit/a9fb647caa5e79a4dbc56caa3f04e5bfedd42957)) - bircni
- **(app)** add window icon, publish topic prefill, load tools and documentation - ([0bc0cb8](https://github.com/bircni/mqtt-viewer/commit/0bc0cb80fb120b0ddbda8966e832bd5296bcb4d8)) - bircni
- **(core)** add connection worker, topic tree, decoders, recording and history store - ([416bbcd](https://github.com/bircni/mqtt-viewer/commit/416bbcd03bbbf2650027986ae476a3e55e13a991)) - bircni
- **(core)** cover TLS, WebSocket and MQTT 5, back off flapping connections and throttle tree refresh - ([745bf56](https://github.com/bircni/mqtt-viewer/commit/745bf563f536666706409b6eb132b01d276122af)) - bircni
- give up system subscriptions on brokers that reject them and make file dialogs testable - ([1aa87d2](https://github.com/bircni/mqtt-viewer/commit/1aa87d2aaf4099cdbe42b133afefdfbecc3edcd2)) - bircni
- expand new topics automatically, add keyboard shortcuts and show the version - ([5115d46](https://github.com/bircni/mqtt-viewer/commit/5115d4604c8e2c6ce5dc4e139150f6205215c6c8)) - bircni
- redraw the icon and package the app for Windows, macOS and Linux - ([5b5d249](https://github.com/bircni/mqtt-viewer/commit/5b5d24992739c9d6041521cdf5f4dd40fd3c74ca)) - bircni

### Tests

- prove replay, retained deletion, charts and stored history against a broker - ([f682aa4](https://github.com/bircni/mqtt-viewer/commit/f682aa4e981dde2073f0f3fbd2d816e788ea704e)) - bircni
- wait until the broker delivers to a client before another one publishes for it - ([211ceca](https://github.com/bircni/mqtt-viewer/commit/211ceca59286205202741b63c2d7188042043149)) - bircni
